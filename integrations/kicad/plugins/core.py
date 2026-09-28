"""GUI-independent fastroute <-> KiCad bridge.

Routing uses the Specctra file exchange that KiCad supports natively:
export the board as DSN, run `fastroute -de board.dsn -do board.ses`,
import the SES session back. Only the `pcbnew` Python module is needed, so
this works both inside the PCB editor and headless (see route_cli.py).
"""

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


def export_dsn(board, path):
    """Writes the board as a Specctra DSN file. Returns True on success."""
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


def strip_planes(dsn_path):
    """Removes `(plane ...)` scopes from a DSN file. Returns the number removed.

    KiCad exports copper zones as planes covering the zone outline, so the
    router treats every pad of the zone's net inside the outline as connected.
    The real fill leaves clearance around the new tracks and can cut pads off
    into islands. Without the planes the zone nets are routed with tracks;
    the refilled zones then only add copper.
    """
    text = Path(dsn_path).read_text(encoding="utf-8", errors="replace")
    out, i, removed = [], 0, 0
    while True:
        j = text.find("(plane ", i)
        if j < 0:
            out.append(text[i:])
            break
        out.append(text[i:j])
        depth, k, quoted = 0, j, False
        while k < len(text):
            c = text[k]
            if c == '"':
                quoted = not quoted
            elif not quoted and c == "(":
                depth += 1
            elif not quoted and c == ")":
                depth -= 1
                if depth == 0:
                    k += 1
                    break
            k += 1
        i = k
        removed += 1
    Path(dsn_path).write_text("".join(out), encoding="utf-8")
    return removed


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
    ):
        self.board = board
        self.clear_tracks = clear_tracks
        self.in_editor = in_editor
        self.text_keepouts = text_keepouts
        self.refill = refill
        self.route_zone_nets = route_zone_nets
        self.binary = Path(binary) if binary else find_binary()
        self.extra_args = list(extra_args)
        self.work_dir = Path(work_dir) if work_dir else None
        self._proc = None
        self._cancel = threading.Event()

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
            result.message = "KiCad could not export the board as Specctra DSN"
            return result
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
        result.message = "routed"
        return result


def mode_args(mode, max_passes=0, threads=0, neckdown=True, min_track_width_nm=0):
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
        # KiCad's board minimum track width is not part of the DSN export;
        # this fastroute option also bounds the fanout's escape traces.
        args.append(f"--router.min_trace_width_um={min_track_width_nm / 1000.0:g}")
    return args
