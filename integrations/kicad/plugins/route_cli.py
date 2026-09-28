"""Headless autorouting of a .kicad_pcb file with fastroute.

Run with the Python interpreter that ships with KiCad (it provides `pcbnew`):

    macOS:   /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3
    Linux:   python3 (with the distribution's KiCad Python module installed)
    Windows: "C:\\Program Files\\KiCad\\<ver>\\bin\\python.exe"

    python3 route_cli.py board.kicad_pcb [-o routed.kicad_pcb] [--clear] [--mode fast|exact|quick]
                         [--max-passes N] [--threads N] [--fastroute PATH] [-- extra fastroute args]
"""

import argparse
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import pcbnew  # noqa: E402

import core  # noqa: E402


def main(argv=None):
    ap = argparse.ArgumentParser(description="Autoroute a KiCad board with fastroute")
    ap.add_argument("board", type=Path)
    ap.add_argument("-o", "--output", type=Path, help="output board (default: overwrite input)")
    ap.add_argument("--clear", action="store_true", help="remove unlocked tracks and vias first")
    ap.add_argument("--mode", choices=("fast", "exact", "quick"), default="fast")
    ap.add_argument("--max-passes", type=int, default=0)
    ap.add_argument("--threads", type=int, default=0)
    ap.add_argument(
        "--neckdown", choices=("on", "off"), default="on", help="narrow traces at pins"
    )
    ap.add_argument(
        "--ignore-min-width",
        action="store_true",
        help="allow neck-down below the board's minimum track width",
    )
    ap.add_argument("--no-refill", action="store_true", help="do not refill zones afterwards")
    ap.add_argument(
        "--zones-as-planes",
        action="store_true",
        help="treat zone nets as connected by the zone (Freerouting's behaviour) instead of routing them",
    )
    ap.add_argument(
        "--no-text-keepouts", action="store_true", help="do not keep traces away from copper texts"
    )
    ap.add_argument("--fastroute", type=Path, help="path to the fastroute executable")
    ap.add_argument("--work-dir", type=Path, help="keep the DSN/SES files in this directory")
    ap.add_argument("-q", "--quiet", action="store_true")
    ap.add_argument("extra", nargs="*", help="extra fastroute arguments (after --)")
    args = ap.parse_args(argv)

    board = pcbnew.LoadBoard(str(args.board))
    if args.work_dir:
        args.work_dir.mkdir(parents=True, exist_ok=True)

    min_width = 0 if args.ignore_min_width else core.min_track_width_nm(board)
    neckdown = args.neckdown == "on"
    router = core.Router(
        board,
        binary=args.fastroute,
        extra_args=core.mode_args(
            args.mode, args.max_passes, args.threads, neckdown, min_width,
            core.copper_edge_clearance_nm(board),
        )
        + args.extra,
        work_dir=args.work_dir,
        refill=not args.no_refill,
        route_zone_nets=not args.zones_as_planes,
        text_keepouts=not args.no_text_keepouts,
        clear_tracks=args.clear,
    )

    def show(level, text):
        if not args.quiet or level in ("WARN", "ERROR"):
            print(f"{level:5} {text}", file=sys.stderr)

    t = time.monotonic()
    result = router.run(show)
    if not result.ok:
        print(f"error: {result.message}", file=sys.stderr)
        return 1
    out = args.output or args.board
    pcbnew.SaveBoard(str(out), board)
    print(
        f"routed in {time.monotonic() - t:.1f} s: score {result.score}, "
        f"{result.unrouted} unrouted, {result.violations} violations -> {out}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
