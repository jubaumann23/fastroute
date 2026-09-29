"""Controlled-impedance trace widths for a KiCad board.

Run with the Python that ships with KiCad (any Python 3 works too: only the
.kicad_pcb file is read):

    python3 impedance_cli.py board.kicad_pcb --class RF=50 --class USB=90d [--gap 0.15] [--write]

A target ending in "d" is differential. Without --gap, pairs use gap = width.
--write puts the widths (and pair gaps) into the board's .kicad_dru as rules
per net class and layer (KiCad's DRC checks them; the fastroute plugin routes
with them).
"""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import impedance  # noqa: E402


def main(argv=None):
    ap = argparse.ArgumentParser(description="Controlled-impedance trace widths from the board stackup")
    ap.add_argument("board", type=Path)
    ap.add_argument("--class", dest="targets", action="append", default=[], metavar="NAME=OHM[d]",
                    help="net class and target impedance, e.g. RF=50 or USB=90d (repeatable)")
    ap.add_argument("--gap", type=float, help="differential pair gap in mm (default: equal to the width)")
    ap.add_argument("--tolerance", type=float, default=0.1, help="width window for the DRC rules (default 0.1 = +-10%%)")
    ap.add_argument("--write", action="store_true", help="write the rules into the board's .kicad_dru")
    args = ap.parse_args(argv)

    if not args.board.is_file():
        print(f"error: {args.board}: no such file", file=sys.stderr)
        return 2
    layers, defined = impedance.read_stackup(args.board)
    print("stackup" + ("" if defined else " (not defined in the board: KiCad default FR4 assumed)") + ":")
    for l in layers:
        if l.kind == "copper":
            print(f"  {l.name:10} copper     {l.thickness * 1000:5.0f} um")
        else:
            print(f"  {'':10} dielectric {l.thickness:6.3f} mm  er {l.epsilon_r:.2f}")
    if not args.targets:
        print("\nno --class targets given")
        return 0
    targets = [impedance.parse_target(t) for t in args.targets]
    rows = impedance.compute(layers, targets, gap=args.gap)
    print(f"\n{'class':10} {'target':>10} {'layer':8} {'type':10} {'width mm':>9} {'gap mm':>7} {'result':>8}")
    for name, ohm, diff, layer, w, g, z in rows:
        geo = impedance.geometry(layers, layer)[0]
        target = f"{ohm:g}{' diff' if diff else ''}"
        if w is None:
            print(f"{name:10} {target:>10} {layer:8} {geo:10} {'not reachable':>9}")
        else:
            print(f"{name:10} {target:>10} {layer:8} {geo:10} {w:9.3f} {(f'{g:.3f}' if g else '-'):>7} {z:8.1f}")
    print("\nNote: solder mask not included (coated microstrips are 2-4 ohm lower); check against the"
          " fabricator's stackup before production.")
    if args.write:
        dru = args.board.with_suffix(".kicad_dru")
        impedance.write_dru(dru, impedance.dru_block(rows, args.tolerance))
        print(f"\nrules written to {dru}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
