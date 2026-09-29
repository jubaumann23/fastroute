"""GUI-independent fastroute <-> KiCad bridge.

Routing uses the Specctra file exchange that KiCad supports natively:
export the board as DSN, run `fastroute -de board.dsn -do board.ses`,
import the SES session back. Only the `pcbnew` Python module is needed, so
this works both inside the PCB editor and headless (see route_cli.py).
"""

import fnmatch
import os
import platform
import re
import shutil
import subprocess
import tempfile
import threading
from pathlib import Path

import pcbnew

PLUGIN_DIR = Path(__file__).resolve().parent

# Lines like "   12.345 INFO  message" from fastroute's stderr.
_LOG_LINE = re.compile(r"^\s*\d+\.\d+\s+(\w+)\s+(.*)$")
# Router score reports; the last one seen is the final state of the board.
_SCORES = (
    re.compile(r"final score: ([\d.]+) \((\d+) unrouted and (\d+) violations?\)"),
    re.compile(
        r"router score: ([\d.]+), incomplete connections: (\d+), clearance violations: (\d+)"
    ),
)


def _exe_name():
    return "fastroute.exe" if os.name == "nt" else "fastroute"


def platform_tag():
    """Platform directory name used for bundled binaries, e.g. macos-arm64."""
    system = {"Darwin": "macos", "Windows": "windows", "Linux": "linux"}.get(
        platform.system(), platform.system().lower()
    )
    machine = platform.machine().lower()
    arch = {"amd64": "x64", "x86_64": "x64", "arm64": "arm64", "aarch64": "arm64"}.get(
        machine, machine
    )
    return f"{system}-{arch}"


def _make_runnable(path):
    """Bundled binaries may lose the executable bit when the package is
    unzipped, and macOS quarantines downloaded unsigned executables."""
    if os.name == "nt":
        return
    try:
        mode = path.stat().st_mode
        if not mode & 0o100:
            path.chmod(mode | 0o755)
    except OSError:
        pass
    if platform.system() == "Darwin":
        subprocess.run(
            ["xattr", "-d", "com.apple.quarantine", str(path)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )


def find_binary():
    """Locates the fastroute executable.

    Order: $FASTROUTE_BIN, the binary bundled with the plugin
    (bin/<platform>/fastroute), then PATH.
    """
    env = os.environ.get("FASTROUTE_BIN")
    if env and Path(env).is_file():
        return Path(env)
    for candidate in (
        PLUGIN_DIR / "bin" / platform_tag() / _exe_name(),
        PLUGIN_DIR / "bin" / _exe_name(),
    ):
        if candidate.is_file():
            _make_runnable(candidate)
            return candidate
    found = shutil.which("fastroute")
    return Path(found) if found else None


def has_board_outline(board):
    """True if the board has an outline on Edge.Cuts (KiCad's DSN export crashes without)."""
    try:
        for d in board.GetDrawings():
            if d.GetLayer() == pcbnew.Edge_Cuts:
                return True
        for fp in board.GetFootprints():
            for g in fp.GraphicalItems():
                if g.GetLayer() == pcbnew.Edge_Cuts:
                    return True
    except Exception:
        return True  # cannot tell: let KiCad try
    return False


def export_dsn(board, path):
    """Writes the board as a Specctra DSN file. Returns True on success."""
    if not has_board_outline(board):
        return False
    try:
        ok = pcbnew.ExportSpecctraDSN(board, str(path))
    except TypeError:  # KiCad 6: operates on the board open in the editor
        ok = pcbnew.ExportSpecctraDSN(str(path))
    return bool(ok) and Path(path).is_file()


def import_ses(board, path, in_editor=False):
    """Loads a Specctra session into the board. Returns True on success.

    The session replaces all tracks and vias. Inside the PCB editor the
    frame-level import is used, which also updates the editor's state
    (selection, connectivity, ratsnest).
    """
    if in_editor:
        try:
            return bool(pcbnew.ImportSpecctraSES(str(path)))
        except TypeError:
            pass
    try:
        ok = pcbnew.ImportSpecctraSES(board, str(path))
    except TypeError:  # KiCad 6
        ok = pcbnew.ImportSpecctraSES(str(path))
    return bool(ok)


def refill_zones(board):
    """Refills all copper zones so they clear the newly routed tracks."""
    zones = board.Zones()
    if len(zones) == 0:
        return
    filler = pcbnew.ZONE_FILLER(board)
    filler.Fill(zones)


def _dsn_quote(name):
    return '"' + name.replace('"', "") + '"'


def _convex_hull(points):
    """Andrew's monotone chain; returns the hull counter-clockwise."""
    pts = sorted(set(points))
    if len(pts) <= 2:
        return pts

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    lower, upper = [], []
    for p in pts:
        while len(lower) >= 2 and cross(lower[-2], lower[-1], p) <= 0:
            lower.pop()
        lower.append(p)
    for p in reversed(pts):
        while len(upper) >= 2 and cross(upper[-2], upper[-1], p) <= 0:
            upper.pop()
        upper.append(p)
    return lower[:-1] + upper[:-1]


def _text_glyph_hulls(item):
    """Convex hulls (in nm) of the stroked glyphs of a copper text."""
    polys = pcbnew.SHAPE_POLY_SET()
    try:
        item.TransformTextToPolySet(polys, 0, pcbnew.FromMM(0.005), pcbnew.ERROR_INSIDE)
    except Exception:
        box = item.GetBoundingBox()
        return [[(box.GetX(), box.GetY()), (box.GetRight(), box.GetY()),
                 (box.GetRight(), box.GetBottom()), (box.GetX(), box.GetBottom())]]
    hulls = []
    for i in range(polys.OutlineCount()):
        outline = polys.Outline(i)
        pts = [(outline.CPoint(k).x, outline.CPoint(k).y) for k in range(outline.PointCount())]
        hull = _convex_hull(pts)
        if len(hull) >= 3:
            hulls.append(hull)
    return hulls


def add_copper_text_keepouts(board, dsn_path):
    """Adds keepouts for texts on copper layers, which KiCad's DSN export omits.

    Each stroked glyph becomes a convex polygon keepout, so traces may pass
    close to the text but not through it. Coordinates follow the export:
    micrometres with the y axis flipped. Returns the number of keepouts added.
    """
    keepouts = []
    for item in board.GetDrawings():
        if item.GetClass() not in ("PCB_TEXT", "PCB_TEXTBOX"):
            continue
        layers = [l for l in item.GetLayerSet().Seq() if pcbnew.IsCopperLayer(l)]
        if not layers:
            continue
        hulls = _text_glyph_hulls(item)
        for layer in layers:
            name = _dsn_quote(board.GetLayerName(layer))
            for hull in hulls:
                coords = " ".join(f"{x / 1000.0:.3f} {-y / 1000.0:.3f}" for x, y in hull)
                keepouts.append(f'    (keepout "" (polygon {name} 0 {coords}))\n')
    if not keepouts:
        return 0
    text = Path(dsn_path).read_text(encoding="utf-8", errors="replace")
    # Keepouts may only use layers defined before them: insert after the
    # layer definitions, i.e. in front of the boundary.
    at = text.find("(boundary")
    if at < 0:
        return 0
    at = text.rfind("\n", 0, at) + 1
    Path(dsn_path).write_text(text[:at] + "".join(keepouts) + text[at:], encoding="utf-8")
    return len(keepouts)


def _scope_end(text, start):
    """Index just past the S-expression starting with '(' at `start`."""
    depth, k, quoted = 0, start, False
    while k < len(text):
        c = text[k]
        if c == '"':
            quoted = not quoted
        elif not quoted and c == "(":
            depth += 1
        elif not quoted and c == ")":
            depth -= 1
            if depth == 0:
                return k + 1
        k += 1
    return len(text)


def strip_unlocked_wiring(dsn_path):
    """Removes unlocked wires and vias from the DSN `(wiring ...)` scope.

    Locked tracks are exported with `(type fix)` (or `protect`) and kept. The
    router then routes from scratch, and KiCad's session import replaces all
    tracks and vias with the result, so nothing has to be deleted on the board.
    Returns the number of removed entries.
    """
    text = Path(dsn_path).read_text(encoding="utf-8", errors="replace")
    w = text.find("(wiring")
    if w < 0:
        return 0
    end = _scope_end(text, w)
    body_start = w + len("(wiring")
    out, i, removed = [text[:body_start]], body_start, 0
    while True:
        j = text.find("(", i, end - 1)
        if j < 0:
            out.append(text[i:end])
            break
        k = _scope_end(text, j)
        entry = text[j:k]
        keep = not entry.startswith(("(wire", "(via")) or "(type fix)" in entry or "(type protect)" in entry
        if keep:
            out.append(text[i:k])
        else:
            removed += 1
        i = k
    out.append(text[end:])
    Path(dsn_path).write_text("".join(out), encoding="utf-8")
    return removed


def _polygon_area(coords):
    pts = list(zip(coords[0::2], coords[1::2]))
    return abs(sum(x1 * y2 - x2 * y1 for (x1, y1), (x2, y2) in zip(pts, pts[1:] + pts[:1]))) / 2.0


def _numbers_after(text, pattern):
    """Numbers following a regex match (the coordinate list of a shape)."""
    m = re.search(pattern, text)
    if not m:
        return []
    rest = text[m.end():]
    end = rest.find(")")
    return [float(v) for v in rest[: end if end >= 0 else len(rest)].split()]


# A power-type layer counts as a real plane layer if one plane covers at
# least this fraction of the board.
PLANE_LAYER_MIN_COVERAGE = 0.5


def _outline_key(points):
    """Order-independent key of polygon corners in DSN micrometres."""
    return frozenset((round(x), round(y)) for x, y in points)


_DRU_RULE = re.compile(r"\(rule\s+\"((?:[^\"\\]|\\.)*)\"(.*?)(?=\(rule\s+\"|\Z)", re.S)
_DRU_TERM = re.compile(r"([AB])\.NetClass\s*(==|!=)\s*'([^']*)'")


def add_dru_class_clearances(board, dsn_path):
    """Carries simple net-class clearance rules of the board's .kicad_dru into the DSN.

    KiCad's DSN export has one clearance per net class; custom rules are lost.
    Rules whose condition only compares net classes (A.NetClass == 'X' combined
    with B.NetClass == / != 'Y' terms, joined by &&) and whose constraint is a
    minimum clearance become Specctra class_class clearances. Returns the rules
    that were carried over (name, pairs).
    """
    try:
        dru = Path(board.GetFileName()).with_suffix(".kicad_dru")
        rules_text = dru.read_text(encoding="utf-8", errors="replace")
    except Exception:
        return []
    text = Path(dsn_path).read_text(encoding="utf-8", errors="replace")
    classes = re.findall(r"\(class\s+\"?([^\s\"()]+)", text)
    dsn_name = {c: c for c in classes}
    dsn_name["Default"] = "kicad_default" if "kicad_default" in classes else "Default"
    unit_um = 1.0
    m = re.search(r"\(resolution\s+(\w+)", text)
    if m and m.group(1) == "mm":
        unit_um = 1000.0
    pairs = {}  # (class a, class b) -> clearance in DSN units
    carried = []
    for name, body in _DRU_RULE.findall(rules_text):
        cond = re.search(r"\(condition\s+\"([^\"]*)\"", body)
        cons = re.search(r"\(constraint\s+clearance\s+\(min\s+([\d.]+)\s*(mm|mil|um)?\)", body)
        if not cond or not cons:
            continue
        terms = [t.strip() for t in cond.group(1).split("&&")]
        parsed = [_DRU_TERM.fullmatch(t) for t in terms]
        if not all(parsed):
            continue
        a_eq = [p.group(3) for p in parsed if p.group(1) == "A" and p.group(2) == "=="]
        if len(a_eq) != 1 or any(p.group(1) == "A" and p.group(2) == "!=" for p in parsed):
            continue
        b_eq = [p.group(3) for p in parsed if p.group(1) == "B" and p.group(2) == "=="]
        b_ne = {p.group(3) for p in parsed if p.group(1) == "B" and p.group(2) == "!="}
        value, unit = float(cons.group(1)), cons.group(2) or "mm"
        um = value * {"mm": 1000.0, "mil": 25.4, "um": 1.0}[unit]
        a = dsn_name.get(a_eq[0])
        others = [dsn_name.get(b) for b in b_eq] if b_eq else [c for c in classes if c not in {dsn_name.get(x) for x in b_ne}]
        if a is None:
            continue
        done = []
        for b in others:
            if b is None:
                continue
            key = tuple(sorted((a, b)))
            clearance = um / unit_um
            if clearance > pairs.get(key, 0.0):
                pairs[key] = clearance
            done.append(b)
        if done:
            carried.append((name, done))
    if not pairs:
        return carried
    start = text.find("(network")
    if start < 0:
        return []
    end = _scope_end(text, start)
    block = "".join(
        f"    (class_class (classes {a} {b}) (rule (clearance {c:g})))\n" for (a, b), c in sorted(pairs.items())
    )
    text = text[: end - 1] + block + "  " + text[end - 1 :]
    Path(dsn_path).write_text(text, encoding="utf-8")
    return carried


_DRU_NET_TERM = re.compile(r"A\.(NetClass|NetName)\s*==\s*'([^']*)'")
_DRU_DIFF_TERM = re.compile(r"A\.inDiffPair\('([^']*)'\)")


def _pair_base(name):
    """KiCad's differential pair naming: (base, '+' or '-') for names ending in +/- or P/N."""
    if len(name) > 1 and name[-1] in "+P":
        return name[:-1], "+"
    if len(name) > 1 and name[-1] in "-N":
        return name[:-1], "-"
    return None, None


def _dru_matches(cond, name, cls):
    """True/False if the net matches a .kicad_dru condition; None if the condition uses
    anything but A.NetClass == '...', A.NetName == '...' and A.inDiffPair('...') joined by
    && or ||."""
    for alt in cond.split("||"):
        terms = [t.strip().strip("()") for t in alt.split("&&")]
        ok = True
        for t in terms:
            m = _DRU_NET_TERM.fullmatch(t)
            d = _DRU_DIFF_TERM.fullmatch(t + ")" if t.startswith("A.inDiffPair(") and not t.endswith(")") else t)
            if m:
                key, val = m.groups()
                ok = ok and (cls == val if key == "NetClass" else fnmatch.fnmatchcase(name, val))
            elif d:
                base, _ = _pair_base(name)
                ok = ok and base is not None and fnmatch.fnmatchcase(base, d.group(1))
            else:
                return None  # unsupported condition
        if ok:
            return True
    return False


def _net_class_name(net):
    for getter in ("GetNetClassName", "GetNetClassSlow"):
        try:
            v = getattr(net, getter)()
            return v if isinstance(v, str) else v.GetName()
        except Exception:
            continue
    return ""


def write_tune_file(board, path):
    """Writes a fastroute --tune file from the length rules of the board's .kicad_dru.

    `(constraint skew (max X))` makes the matching nets one group (matched to the longest
    net, X allowed); with `(within_diff_pairs)` each differential pair is its own group;
    `(constraint length (min A) ...)` brings every matching net to at least A.
    Conditions: A.NetClass == 'X' and A.NetName == 'pattern' (* = any text), joined by &&
    or ||. Returns the number of groups (0: no file written).
    """
    try:
        dru = Path(board.GetFileName()).with_suffix(".kicad_dru")
        rules_text = dru.read_text(encoding="utf-8", errors="replace")
    except Exception:
        return 0
    nets = {}
    try:
        for name, net in board.GetNetsByName().items():
            name = str(name)
            if name:
                nets[name] = _net_class_name(net)
    except Exception:
        return 0

    def mm(value, unit):
        return float(value) * {"mm": 1.0, "mil": 0.0254, "um": 0.001}[unit or "mm"]

    groups = []
    for rule_name, body in _DRU_RULE.findall(rules_text):
        cond = re.search(r"\(condition\s+\"([^\"]*)\"", body)
        skew = re.search(r"\(constraint\s+skew\s+\(max\s+([\d.]+)\s*(mm|mil|um)?\)", body)
        length = re.search(r"\(constraint\s+length\s+\(min\s+([\d.]+)\s*(mm|mil|um)?\)", body)
        if not cond or not (skew or length):
            continue
        members = []
        for name, cls in sorted(nets.items()):
            r = _dru_matches(cond.group(1), name, cls)
            if r is None:
                members = []
                break
            if r:
                members.append(name)
        if not members:
            continue
        tag = re.sub(r"\W+", "_", rule_name).strip("_") or "rule"
        if skew and re.search(r"\(within_diff_pairs\)", body):
            # KiCad: `(within_diff_pairs)` limits the skew inside each pair; without it the
            # skew is between all matching nets
            pairs = {}
            for name in members:
                base, _ = _pair_base(name)
                pairs.setdefault(base, []).append(name)
            for base, pair_nets in sorted(pairs.items()):
                if len(pair_nets) == 2:
                    ptag = tag + "_" + (re.sub(r"\W+", "_", base).strip("_") or "pair")
                    groups.append((ptag, f"tolerance={mm(*skew.groups()):g}", sorted(pair_nets)))
        elif skew:
            groups.append((tag, f"tolerance={mm(*skew.groups()):g}", members))
        if length:
            groups.append((tag + "_min", f"tolerance=0 target={mm(*length.groups()):g}", members))
    if not groups:
        return 0
    lines = []
    for tag, opts, members in groups:
        lines.append(f"group {tag} {opts}")
        lines.extend(f"  {n}" for n in members)
    Path(path).write_text("\n".join(lines) + "\n", encoding="utf-8")
    return len(groups)


def _board_layer_name(board, layer):
    """The board's own name of a layer given by its canonical name ("In1.Cu")."""
    try:
        lid = board.GetLayerID(layer)
        if lid >= 0:
            return board.GetLayerName(lid)
    except Exception:
        pass
    return layer


def layer_heights_mm(board):
    """Height of each copper layer from the top of the board (stackup), for via lengths in
    length matching; [] if the stackup cannot be read."""
    try:
        import impedance

        layers, _defined = impedance.read_stackup(Path(board.GetFileName()))
    except Exception:
        return []
    heights, z = [], 0.0
    for layer in layers:
        if layer.kind == "copper":
            heights.append(z)
        z += layer.thickness
    return heights


def write_pairs_file(board, path):
    """Writes a fastroute --pairs file from the diff_pair_gap rules of the board's .kicad_dru.

    Every rule with `(constraint diff_pair_gap ...)` makes the matching nets differential
    pairs, paired by KiCad's naming (NAME+/NAME-, NAMEP/NAMEN), with the rule's opt gap
    (min if there is no opt), for one layer if the rule has `(layer "...")`. Returns the
    number of pairs (0: no file written).
    """
    try:
        dru = Path(board.GetFileName()).with_suffix(".kicad_dru")
        rules_text = dru.read_text(encoding="utf-8", errors="replace")
        nets = {str(n): _net_class_name(net) for n, net in board.GetNetsByName().items() if str(n)}
    except Exception:
        return 0
    pairs = {}  # base -> {"+": name, "-": name, "gap": mm, "layers": {layer: mm}}
    for _rule_name, body in _DRU_RULE.findall(rules_text):
        cond = re.search(r"\(condition\s+\"([^\"]*)\"", body)
        gap = re.search(r"\(constraint\s+diff_pair_gap\b([^\n]*)", body)
        if not cond or not gap:
            continue
        value = re.search(r"\(opt\s+([\d.]+)\s*(mm|mil|um)?\)", gap.group(1)) or re.search(
            r"\(min\s+([\d.]+)\s*(mm|mil|um)?\)", gap.group(1)
        )
        mm = float(value.group(1)) * {"mm": 1.0, "mil": 0.0254, "um": 0.001}[value.group(2) or "mm"] if value else None
        layer = re.search(r"\(layer\s+\"?([^\"\s)]+)\"?\)", body)
        for name, cls in nets.items():
            base, pol = _pair_base(name)
            if base is None or not _dru_matches(cond.group(1), name, cls):
                continue
            entry = pairs.setdefault(base, {"gap": None, "layers": {}})
            entry[pol] = name
            if mm is not None:
                if layer:
                    entry["layers"][_board_layer_name(board, layer.group(1))] = mm
                else:
                    entry["gap"] = mm
    lines = []
    for base, e in sorted(pairs.items()):
        if "+" not in e or "-" not in e:
            continue
        opts = ([f"gap={e['gap']:g}"] if e["gap"] is not None else []) + [f"gap@{l}={g:g}" for l, g in sorted(e["layers"].items())]
        lines.append(" ".join(["pair", e["+"], e["-"]] + opts))
    if not lines:
        return 0
    Path(path).write_text("\n".join(lines) + "\n", encoding="utf-8")
    return len(lines)


def add_dru_layer_widths(board, dsn_path):
    """Carries per-layer track widths of the .kicad_dru (e.g. the controlled-impedance rules
    written by impedance_cli.py) into the DSN as class layer rules. Returns
    [(class, layer, width mm)]."""
    try:
        import impedance

        dru = Path(board.GetFileName()).with_suffix(".kicad_dru")
        widths = impedance.layer_widths_from_dru(dru.read_text(encoding="utf-8", errors="replace"))
    except Exception:
        return []
    if not widths:
        return []
    text = Path(dsn_path).read_text(encoding="utf-8", errors="replace")
    unit_um = 1000.0 if re.search(r"\(resolution\s+mm", text) else 1.0
    done = []
    for (cls, layer), w in sorted(widths.items()):
        dsn_cls = "kicad_default" if cls == "Default" and "(class kicad_default" in text else cls
        m = re.search(r"\(class\s+\"?" + re.escape(dsn_cls) + r"\"?[\s)]", text)
        if not m:
            continue
        end = _scope_end(text, m.start())
        # the DSN uses the board's layer names ("L1-Signal"), the rules may use canonical ones
        dsn_layer = layer
        try:
            lid = board.GetLayerID(layer)
            if lid >= 0:
                dsn_layer = board.GetLayerName(lid)
        except Exception:
            pass
        if re.search(r"[\s()]", dsn_layer):
            dsn_layer = f'"{dsn_layer}"'
        rule = f" (layer_rule {dsn_layer} (rule (width {w * 1000.0 / unit_um:g})))"
        text = text[: end - 1] + rule + text[end - 1 :]
        done.append((cls, layer, w))
    Path(dsn_path).write_text(text, encoding="utf-8")
    return done


def fix_rule_area_keepouts(board, dsn_path):
    """Corrects the keepouts KiCad exports for rule areas.

    KiCad writes every rule area as a Specctra keepout, including areas that
    forbid neither tracks nor vias (e.g. KiCad 10's multichannel
    "auto-placement-area" regions); the router then blocks everything inside,
    so whole channels become unroutable. Such keepouts are removed; areas that
    only forbid vias become via keepouts. Returns (removed, converted).
    """
    nonblocking, via_only = set(), set()
    for zone in board.Zones():
        if not zone.GetIsRuleArea():
            continue
        tracks, vias = zone.GetDoNotAllowTracks(), zone.GetDoNotAllowVias()
        if tracks:
            continue
        outline = zone.Outline()
        for i in range(outline.OutlineCount()):
            chain = outline.Outline(i)
            key = _outline_key(
                (chain.CPoint(k).x / 1000.0, -chain.CPoint(k).y / 1000.0)
                for k in range(chain.PointCount())
            )
            (via_only if vias else nonblocking).add(key)
    if not nonblocking and not via_only:
        return 0, 0
    text = Path(dsn_path).read_text(encoding="utf-8", errors="replace")
    out, i, removed, converted = [], 0, 0, 0
    while True:
        j = text.find("(keepout", i)
        if j < 0:
            out.append(text[i:])
            break
        k = _scope_end(text, j)
        scope = text[j:k]
        nums = _numbers_after(scope, r"\(polygon\s+(?:\"[^\"]*\"|\S+)")
        key = _outline_key(zip(nums[1::2], nums[2::2])) if len(nums) >= 7 else None
        out.append(text[i:j])
        if key is not None and key in nonblocking:
            removed += 1
        elif key is not None and key in via_only:
            out.append("(via_keepout" + scope[len("(keepout"):])
            converted += 1
        else:
            out.append(scope)
        i = k
    Path(dsn_path).write_text("".join(out), encoding="utf-8")
    return removed, converted


def strip_planes(dsn_path):
    """Prepares the zones of a KiCad DSN export for routing.

    KiCad exports every copper zone as a plane covering the zone outline, so the
    router treats every pad of the zone's net inside the outline as connected.
    The real fill leaves clearance around the new tracks and can cut pads off
    into islands, so zones are dropped and their nets routed with tracks; the
    refilled zones then only add copper.

    Exception: a power-type layer with a plane covering most of the board is a
    real plane layer. Its plane is kept (pads reach it reliably through vias)
    and the router keeps the layer free of tracks. Other power-type layers
    (e.g. a top layer typed "power" with a few pours) become signal layers,
    since Freerouting never routes on power layers. Returns the number of
    removed planes.
    """
    text = Path(dsn_path).read_text(encoding="utf-8", errors="replace")
    boundary = _numbers_after(text, r"\(boundary\s*\(path\s+pcb\s+[\d.]+")
    board_area = _polygon_area(boundary) if len(boundary) >= 6 else 0.0

    planes = []  # (start, end, layer, area)
    i = 0
    while True:
        j = text.find("(plane ", i)
        if j < 0:
            break
        k = _scope_end(text, j)
        scope = text[j:k]
        m = re.match(r'\(plane\s+(?:"[^"]*"|\S+)\s+\((polygon|rect)\s+"?([^\s"()]+)"?', scope)
        layer, area = None, 0.0
        if m:
            layer = m.group(2)
            nums = _numbers_after(scope, r"\((?:polygon|rect)\s+\S+")
            if m.group(1) == "polygon" and len(nums) >= 7:
                area = _polygon_area(nums[1:])  # skip the aperture width
            elif m.group(1) == "rect" and len(nums) >= 4:
                area = abs(nums[2] - nums[0]) * abs(nums[3] - nums[1])
        planes.append((j, k, layer, area))
        i = k

    power_layers = set(
        m.group(1)
        for m in re.finditer(r'\(layer\s+"?([^\s"()]+)"?\s*\(type\s+power\)', text)
    )
    plane_layers = {
        layer
        for _, _, layer, area in planes
        if layer in power_layers and board_area > 0 and area >= PLANE_LAYER_MIN_COVERAGE * board_area
    }

    out, pos, removed = [], 0, 0
    for j, k, layer, _ in planes:
        out.append(text[pos:j])
        if layer in plane_layers:
            out.append(text[j:k])
        else:
            removed += 1
        pos = k
    out.append(text[pos:])
    result = "".join(out)
    for layer in power_layers - plane_layers:
        result = re.sub(
            r'(\(layer\s+"?' + re.escape(layer) + r'"?\s*\(type\s+)power\)', r"\1signal)", result
        )
    Path(dsn_path).write_text(result, encoding="utf-8")
    return removed


def copper_edge_clearance_nm(board):
    """The board's copper-to-edge clearance constraint (0 if unset)."""
    try:
        return int(board.GetDesignSettings().m_CopperEdgeClearance)
    except Exception:
        return 0


def min_track_width_nm(board):
    """The board's minimum track width constraint (0 if unset)."""
    try:
        return int(board.GetDesignSettings().m_TrackMinWidth)
    except Exception:
        return 0


class RouteResult:
    def __init__(self):
        self.ok = False
        self.cancelled = False
        # True if fastroute stopped early (time limit, error) and only its best board so far
        # (the checkpoint session it keeps rewriting) was imported.
        self.partial = False
        self.message = ""
        self.score = None
        self.unrouted = None
        self.violations = None
        self.log = []


class Router:
    """Runs one export → route → import cycle; can be cancelled from another thread."""

    def __init__(
        self,
        board,
        binary=None,
        extra_args=(),
        work_dir=None,
        refill=True,
        route_zone_nets=True,
        text_keepouts=True,
        clear_tracks=False,
        in_editor=False,
        max_time=None,
    ):
        self.board = board
        self.clear_tracks = clear_tracks
        self.in_editor = in_editor
        self.text_keepouts = text_keepouts
        self.refill = refill
        self.route_zone_nets = route_zone_nets
        self.binary = Path(binary) if binary else find_binary()
        self.extra_args = list(extra_args)
        if max_time:
            # fastroute stops by itself and writes its best result (no kill needed)
            self.extra_args.append(f"--max-time={int(max_time)}")
        self.work_dir = Path(work_dir) if work_dir else None
        self._proc = None
        self._cancel = threading.Event()
        self.dru_rules = []
        self.layer_widths = []

    def cancel(self):
        self._cancel.set()
        proc = self._proc
        if proc and proc.poll() is None:
            proc.terminate()

    def run(self, on_line=None):
        """Routes the board in place. `on_line(level, text)` receives progress lines.

        GUI callers should instead call `prepare()` and `finish()` on the GUI
        thread and only `route()` in a worker thread (pcbnew is not thread-safe).
        """
        result = self.prepare()
        if result is not None:
            return result
        return self.finish(self.route(on_line))

    def prepare(self):
        """Exports the board. Returns a failed RouteResult, or None on success."""
        if self.binary is None:
            result = RouteResult()
            result.message = (
                "fastroute executable not found (set FASTROUTE_BIN, bundle it in the "
                "plugin's bin/ directory, or put it on PATH)"
            )
            return result
        tmp = self.work_dir or Path(tempfile.mkdtemp(prefix="fastroute_"))
        self._dsn, self._ses = tmp / "board.dsn", tmp / "board.ses"
        for f in (self._dsn, self._ses):
            f.unlink(missing_ok=True)
        if not export_dsn(self.board, self._dsn):
            result = RouteResult()
            result.message = (
                "KiCad could not export the board as Specctra DSN"
                + ("" if has_board_outline(self.board) else " (the board has no outline on Edge.Cuts)")
            )
            return result
        fix_rule_area_keepouts(self.board, self._dsn)
        self.dru_rules = add_dru_class_clearances(self.board, self._dsn)
        self.layer_widths = add_dru_layer_widths(self.board, self._dsn)
        self._tune = self._dsn.with_name("tune.txt")
        self._tune.unlink(missing_ok=True)
        self.tune_groups = 0
        if not any(a.startswith("--tune=") for a in self.extra_args):
            self.tune_groups = write_tune_file(self.board, self._tune)
        self._pairs = self._dsn.with_name("pairs.txt")
        self._pairs.unlink(missing_ok=True)
        self.diff_pairs = 0
        if not any(a.startswith("--pairs=") for a in self.extra_args):
            self.diff_pairs = write_pairs_file(self.board, self._pairs)
        if self.clear_tracks:
            strip_unlocked_wiring(self._dsn)
        if self.route_zone_nets:
            strip_planes(self._dsn)
        if self.text_keepouts:
            add_copper_text_keepouts(self.board, self._dsn)
        return None

    def route(self, on_line=None):
        """Runs fastroute on the exported file (does not touch the board)."""
        result = RouteResult()
        cmd = [str(self.binary), "-de", str(self._dsn), "-do", str(self._ses)] + self.extra_args
        classes = sorted({c for c, _, _ in getattr(self, "layer_widths", [])})
        if classes and not any(x.startswith("--no-neckdown-classes=") for x in self.extra_args):
            cmd.append("--no-neckdown-classes=" + ",".join(classes))
        if getattr(self, "tune_groups", 0):
            cmd.append(f"--tune={self._tune}")
            heights = layer_heights_mm(self.board)
            if heights and not any(x.startswith("--layer-heights=") for x in self.extra_args):
                cmd.append("--layer-heights=" + ",".join(f"{h:.4f}" for h in heights))
        if getattr(self, "diff_pairs", 0):
            cmd.append(f"--pairs={self._pairs}")
        kwargs = {}
        if os.name == "nt":
            kwargs["creationflags"] = 0x08000000  # CREATE_NO_WINDOW
        self._proc = subprocess.Popen(
            cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            **kwargs,
        )
        for raw in self._proc.stderr:
            line = raw.rstrip("\n")
            result.log.append(line)
            m = _LOG_LINE.match(line)
            level, text = (m.group(1), m.group(2)) if m else ("INFO", line)
            for pattern in _SCORES:
                s = pattern.search(text)
                if s:
                    result.score = float(s.group(1))
                    result.unrouted = int(s.group(2))
                    result.violations = int(s.group(3))
            if on_line:
                on_line(level, text)
        code = self._proc.wait()
        if self._cancel.is_set():
            result.cancelled = True
            result.message = "cancelled"
        elif code != 0 and self._ses.is_file():
            # fastroute rewrites the session with its best board after every improvement:
            # keep that result even if the run ended abnormally (killed, crashed).
            result.ok = True
            result.partial = True
            tail = "\n".join(result.log[-5:])
            result.message = f"fastroute ended with exit code {code}; imported its best result so far\n{tail}"
        elif code != 0 or not self._ses.is_file():
            tail = "\n".join(result.log[-5:])
            result.message = f"fastroute failed (exit code {code})\n{tail}"
        else:
            result.ok = True
        return result

    def finish(self, result):
        """Imports the routed session into the board (if routing succeeded)."""
        if not result.ok:
            return result
        if not import_ses(self.board, self._ses, self.in_editor):
            result.ok = False
            result.message = "KiCad could not import the routed session"
            return result
        if self.refill:
            refill_zones(self.board)
        if not result.partial:
            result.message = "routed"
        return result


def mode_args(
    mode, max_passes=0, threads=0, neckdown=True, min_track_width_nm=0, edge_clearance_nm=0
):
    """Command-line options for the plugin's routing modes.

    fast  - default: parallel optimizer, wall-clock time limits.
    exact - bit-identical to Freerouting's deterministic mode (slower).
    neckdown=False forbids narrowing traces at pins (keeps KiCad's minimum width).
    quick - autorouter only, no optimizer pass.
    """
    args = []
    if mode == "exact":
        args.append("--parity")
    elif mode == "quick":
        args.append("--router.optimizer.enabled=false")
    if max_passes and max_passes > 0:
        args += ["-mp", str(max_passes)]
    if threads and threads > 0:
        args.append(f"--router.optimizer.max_threads={threads}")
    if not neckdown:
        # Neck-down narrows traces at pins below the net's width.
        args.append("--router.automatic_neckdown=false")
    if min_track_width_nm > 0:
        width_um = f"{min_track_width_nm / 1000.0:g}"
        # KiCad's board minimum track width is not part of the DSN export;
        # this fastroute option also bounds the fanout's escape traces.
        args.append(f"--router.min_trace_width_um={width_um}")
        # Connections whose path is found but whose trace does not fit are
        # retried with traces of the minimum width (Freerouting's "necked
        # retry", off by default there).
        args.append(f"--router.neck_width_um={width_um}")
    if edge_clearance_nm > 0:
        # Board Setup > Constraints > Copper to edge clearance (not in the DSN
        # export; Freerouting's own default is 250 um).
        args.append(f"--router.copper_to_edge_clearance_um={edge_clearance_nm / 1000.0:g}")
    return args
