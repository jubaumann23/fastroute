"""Controlled impedance: trace widths from the board stackup.

The stackup is read from the .kicad_pcb file (the pcbnew Python binding does not
expose it). For every copper layer the reference planes are taken to be the
neighbouring copper layers:

* outer layer  -> microstrip over the next layer (Hammerstad-Jensen with the
  Wadell thickness correction, about 1 % for 0.1 < w/h < 10),
* inner layer  -> stripline between both neighbours (Wheeler's thick-strip
  formula; asymmetric striplines combine the two symmetric ones),
* differential -> edge coupling after IPC-2141 (about 5-10 %).

Solder mask is not included: coated microstrips come out 2-4 ohm lower. For a
production board use the fabricator's impedance calculator / stackup as the
reference; these values are a starting point that is usually within 10 %.
"""

import math
import re
from pathlib import Path

ETA0 = 376.730313668  # free-space impedance, ohm


# --- stackup -------------------------------------------------------------------------------


class Layer:
    def __init__(self, name, kind, thickness_mm, epsilon_r=None):
        self.name = name
        self.kind = kind  # "copper" or "dielectric"
        self.thickness = thickness_mm
        self.epsilon_r = epsilon_r

    def __repr__(self):
        return f"Layer({self.name!r}, {self.kind}, {self.thickness}, {self.epsilon_r})"


def _scope(text, start):
    depth, i, quoted = 0, start, False
    while i < len(text):
        c = text[i]
        if c == '"':
            quoted = not quoted
        elif not quoted and c == "(":
            depth += 1
        elif not quoted and c == ")":
            depth -= 1
            if depth == 0:
                return text[start : i + 1]
        i += 1
    return text[start:]


def read_stackup(pcb_path, default_epsilon_r=4.5):
    """The copper/dielectric sequence (top to bottom) and whether it was defined in the file."""
    text = Path(pcb_path).read_text(encoding="utf-8", errors="replace")
    i = text.find("(stackup")
    layers = []
    if i >= 0:
        stack = _scope(text, i)
        for m in re.finditer(r'\(layer\s+"([^"]+)"', stack):
            body = _scope(stack, m.start())
            kind = re.search(r'\(type\s+"([^"]+)"\)', body)
            kind = kind.group(1) if kind else ""
            t = re.search(r"\(thickness\s+([\d.]+)", body)
            er = re.search(r"\(epsilon_r\s+([\d.]+)", body)
            if kind == "copper":
                layers.append(Layer(m.group(1), "copper", float(t.group(1)) if t else 0.035))
            elif kind in ("core", "prepreg") or kind.lower().startswith("dielectric"):
                # a dielectric may consist of sublayers: sum the thicknesses, average epsilon_r
                ts = [float(x) for x in re.findall(r"\(thickness\s+([\d.]+)", body)]
                ers = [float(x) for x in re.findall(r"\(epsilon_r\s+([\d.]+)", body)]
                thick = sum(ts) if ts else 0.0
                e = sum(ers) / len(ers) if ers else default_epsilon_r
                if thick > 0:
                    layers.append(Layer(m.group(1), "dielectric", thick, e))
        if sum(1 for l in layers if l.kind == "copper") >= 2:
            return layers, True
    # no stackup: KiCad's default (1.6 mm, FR4, dielectrics evenly spread)
    n = len(re.findall(r'\(\d+\s+"(?:F|B|In\d+)\.Cu"\s+(?:signal|power|mixed|jumper)', text)) or 2
    board_t = re.search(r"\(general[^()]*\(thickness\s+([\d.]+)", text)
    board_t = float(board_t.group(1)) if board_t else 1.6
    cu = 0.035
    gap = (board_t - n * cu) / (n - 1)
    names = ["F.Cu"] + [f"In{i}.Cu" for i in range(1, n - 1)] + ["B.Cu"]
    layers = []
    for i, name in enumerate(names):
        layers.append(Layer(name, "copper", cu))
        if i < n - 1:
            layers.append(Layer(f"dielectric {i + 1}", "dielectric", gap, default_epsilon_r))
    return layers, False


def geometry(layers, copper_name):
    """('microstrip', h, t, er) or ('stripline', h1, h2, t, er) for a copper layer."""
    idx = [i for i, l in enumerate(layers) if l.kind == "copper" and l.name == copper_name]
    if not idx:
        raise KeyError(copper_name)
    i = idx[0]
    t = layers[i].thickness

    def side(step):
        h, ers, j = 0.0, [], i + step
        while 0 <= j < len(layers) and layers[j].kind != "copper":
            h += layers[j].thickness
            ers.append((layers[j].thickness, layers[j].epsilon_r or 4.5))
            j += step
        if not (0 <= j < len(layers)):
            return None
        er = sum(th * e for th, e in ers) / h if h > 0 else 4.5
        return h, er

    above, below = side(-1), side(+1)
    if above and below:
        (h1, e1), (h2, e2) = above, below
        er = (h1 * e1 + h2 * e2) / (h1 + h2)
        return ("stripline", h1, h2, t, er)
    h, er = above or below
    return ("microstrip", h, t, er)


# --- formulas --------------------------------------------------------------------------------


def _coth(x):
    return 1.0 / math.tanh(x)


def microstrip_z0(w, h, t, er):
    """Hammerstad-Jensen microstrip impedance with the thickness correction (Wadell)."""

    def z01(u):
        f = 6 + (2 * math.pi - 6) * math.exp(-((30.666 / u) ** 0.7528))
        return ETA0 / (2 * math.pi) * math.log(f / u + math.sqrt(1 + (2 / u) ** 2))

    def eeff(u):
        a = 1 + math.log((u**4 + (u / 52) ** 2) / (u**4 + 0.432)) / 49 + math.log(1 + (u / 18.1) ** 3) / 18.7
        b = 0.564 * ((er - 0.9) / (er + 3)) ** 0.053
        return (er + 1) / 2 + (er - 1) / 2 * (1 + 10 / u) ** (-a * b)

    u = w / h
    if t > 0:
        t1 = t / h
        du1 = t1 / math.pi * math.log(1 + 4 * math.e / (t1 * _coth(math.sqrt(6.517 * u)) ** 2))
        dur = 0.5 * (1 + 1 / math.cosh(math.sqrt(er - 1))) * du1
    else:
        du1 = dur = 0.0
    u1, ur = u + du1, u + dur
    e = eeff(ur) * (z01(u1) / z01(ur)) ** 2
    return z01(ur) / math.sqrt(e)


def stripline_symmetric_z0(w, b, t, er):
    """Wheeler's formula for a centred stripline of width w, thickness t, plane spacing b."""
    x = t / b
    if x > 0:
        m = 2 / (1 + 2 / 3 * x / (1 - x))
        dw = x / (math.pi * (1 - x)) * (
            1 - 0.5 * math.log((x / (2 - x)) ** 2 + (0.0796 * x / (w / b + 1.1 * x)) ** m)
        )
    else:
        dw = 0.0
    wp = w / (b - t) + dw
    k = 8 / (math.pi * wp)
    return 30 / math.sqrt(er) * math.log(1 + 4 / (math.pi * wp) * (k + math.sqrt(k * k + 6.27)))


def stripline_z0(w, h1, h2, t, er):
    """Asymmetric stripline (trace at h1 from one plane, h2 from the other)."""
    if abs(h1 - h2) < 1e-9:
        return stripline_symmetric_z0(w, 2 * h1 + t, t, er)
    za = stripline_symmetric_z0(w, 2 * h1 + t, t, er)
    zb = stripline_symmetric_z0(w, 2 * h2 + t, t, er)
    return 2 * za * zb / (za + zb)


def z_single(geo, w):
    if geo[0] == "microstrip":
        _, h, t, er = geo
        return microstrip_z0(w, h, t, er)
    _, h1, h2, t, er = geo
    return stripline_z0(w, h1, h2, t, er)


def z_diff(geo, w, s):
    z0 = z_single(geo, w)
    if geo[0] == "microstrip":
        h = geo[1]
        return 2 * z0 * (1 - 0.48 * math.exp(-0.96 * s / h))
    b = geo[1] + geo[2] + geo[3]
    return 2 * z0 * (1 - 0.347 * math.exp(-2.9 * s / b))


def solve_width(func, target, lo=0.02, hi=10.0):
    """Width (mm) where the (decreasing) impedance func(w) meets target; None if out of range."""
    if func(lo) < target or func(hi) > target:
        return None
    for _ in range(80):
        mid = math.sqrt(lo * hi)
        if func(mid) > target:
            lo = mid
        else:
            hi = mid
    return (lo + hi) / 2


def width_for(geo, target, diff=False, gap=None, gap_ratio=1.0):
    """(width, gap) in mm for a target impedance; gap is fixed or `gap_ratio * width`."""
    if not diff:
        w = solve_width(lambda w: z_single(geo, w), target)
        return (w, None)
    if gap is not None:
        w = solve_width(lambda w: z_diff(geo, w, gap), target)
        return (w, gap)
    w = solve_width(lambda w: z_diff(geo, w, gap_ratio * w), target)
    return (w, None if w is None else gap_ratio * w)


# --- targets, report, KiCad rules ------------------------------------------------------------

BEGIN_MARK = "# --- fastroute impedance (generated, do not edit between the marks) ---"
END_MARK = "# --- end fastroute impedance ---"


def parse_target(spec):
    """'USB=90d' / 'RF=50' -> (class, ohm, differential)."""
    name, _, value = spec.partition("=")
    value = value.strip().lower()
    diff = value.endswith("d")
    return name.strip(), float(value.rstrip("d").rstrip("ohm").strip()), diff


def compute(layers, targets, gap=None, gap_ratio=1.0):
    """[(class, ohm, diff, layer, width, gap, achieved ohm)] for every copper layer."""
    rows = []
    coppers = [l.name for l in layers if l.kind == "copper"]
    for name, ohm, diff in targets:
        for layer in coppers:
            geo = geometry(layers, layer)
            w, g = width_for(geo, ohm, diff=diff, gap=gap, gap_ratio=gap_ratio)
            if w is None:
                rows.append((name, ohm, diff, layer, None, None, None))
                continue
            z = z_diff(geo, w, g) if diff else z_single(geo, w)
            rows.append((name, ohm, diff, layer, w, g, z))
    return rows


def dru_block(rows, tolerance=0.1):
    """KiCad DRC rules: per class and layer a track width window (and the pair gap)."""
    out = [BEGIN_MARK]
    for name, ohm, diff, layer, w, g, _ in rows:
        if w is None:
            continue
        kind = "differential" if diff else "single-ended"
        out.append(f'(rule "impedance {name} {ohm:g} ohm {kind} {layer}"')
        out.append(f'  (layer "{layer}")')
        out.append(f"  (condition \"A.NetClass == '{name}'\")")
        out.append(
            f"  (constraint track_width (min {w * (1 - tolerance):.4f}mm) (opt {w:.4f}mm) (max {w * (1 + tolerance):.4f}mm))"
        )
        if diff and g is not None:
            out.append(f"  (constraint diff_pair_gap (min {g * (1 - tolerance):.4f}mm) (opt {g:.4f}mm) (max {g * (1 + tolerance):.4f}mm))")
        out.append(")")
    out.append(END_MARK)
    return "\n".join(out) + "\n"


def write_dru(dru_path, block):
    """Puts the block into the .kicad_dru (replacing an earlier one); creates the file."""
    p = Path(dru_path)
    text = p.read_text(encoding="utf-8") if p.is_file() else "(version 1)\n"
    if BEGIN_MARK in text and END_MARK in text:
        a = text.index(BEGIN_MARK)
        b = text.index(END_MARK) + len(END_MARK)
        text = text[:a] + block.rstrip("\n") + text[b:]
    else:
        text = text.rstrip("\n") + "\n" + block
    p.write_text(text, encoding="utf-8")


def layer_widths_from_dru(dru_text):
    """{(net class, layer): width mm} from track_width rules with (layer ...) and a NetClass
    condition (the `opt` value, else the middle of min/max)."""
    widths = {}
    for m in re.finditer(r'\(rule\s+"', dru_text):
        body = _scope(dru_text, m.start())
        layer = re.search(r'\(layer\s+"([^"]+)"\)', body)
        cond = re.search(r"\(condition\s+\"A\.NetClass\s*==\s*'([^']+)'\"\)", body)
        tw = re.search(r"\(constraint\s+track_width([^()]*(?:\([^()]*\)[^()]*)*)\)", body)
        if not (layer and cond and tw):
            continue
        vals = dict(re.findall(r"\((min|opt|max)\s+([\d.]+)mm\)", tw.group(1)))
        if "opt" in vals:
            w = float(vals["opt"])
        elif "min" in vals and "max" in vals:
            w = (float(vals["min"]) + float(vals["max"])) / 2
        elif "min" in vals:
            w = float(vals["min"])
        else:
            continue
        widths[(cond.group(1), layer.group(1))] = w
    return widths
