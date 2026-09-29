#!/bin/sh
# A/B comparison of fastroute with and without its non-Freerouting improvements
# (default parallel mode) on the benchmark boards.
# Usage: scripts/ab-enhancements.sh [outdir] [extra fastroute args...]
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=${FASTROUTE_BIN:-${CARGO_TARGET_DIR:-$ROOT/target}/release/fastroute}
OUT=${1:-/tmp/fastroute-ab}; [ $# -gt 0 ] && shift
mkdir -p "$OUT"
printf "%-28s %-4s %8s %9s %6s %6s %9s\n" board mode time_s router unrout viol opt_score
for dsn in "$ROOT"/reference/freerouting/scripts/benchmark/fixtures/DAC2020_boards/*.dsn \
           "$ROOT"/reference/freerouting/scripts/benchmark/fixtures/KiCad_10_demos/*.dsn; do
  n=$(basename "$dsn" .dsn)
  for mode in old new; do
    flag=""; [ $mode = old ] && flag="--no-enhancements"
    log="$OUT/$n.$mode.log"
    start=$(python3 -c 'import time;print(time.time())')
    "$BIN" $flag "$@" -de "$dsn" -do "$OUT/$n.$mode.ses" > "$log" 2>&1
    end=$(python3 -c 'import time;print(time.time())')
    python3 - "$log" "$n" "$mode" "$start" "$end" <<'PY'
import re, sys
log, n, mode, a, b = sys.argv[1:]
t = open(log).read()
r = re.findall(r"final router score: ([\d.]+), final optimizer score: ([\d.]+)", t)
s = re.findall(r"final score: ([\d.]+) \((\d+) unrouted and (\d+) violation", t)
inc = re.findall(r"router score: [\d.]+, incomplete connections: (\d+), clearance violations: (\d+)", t)
router = r[-1][0] if r else (s[-1][0] if s else "?")
opt = r[-1][1] if r else "-"
unr, vio = (inc[-1] if inc else (s[-1][1:] if s else ("?", "?")))
print(f"{n:28} {mode:4} {float(b)-float(a):8.1f} {router:>9} {unr:>6} {vio:>6} {opt:>9}")
PY
  done
done
