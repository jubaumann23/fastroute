#!/bin/sh
# End-to-end check of the KiCad integration: clears the tracks of each board,
# routes it headless through KiCad's pcbnew (route_cli.py) and runs KiCad's
# DRC on the result.
#
# Usage: integrations/kicad/tests/e2e.sh [board.kicad_pcb ...]
# Env:   KICAD_APP   KiCad.app (default /Applications/KiCad/KiCad.app)
#        FASTROUTE_BIN   fastroute binary (default: release build)
#        FREEROUTING_CMD optional command running Freerouting with the same CLI
#                        (e.g. a wrapper around `java -jar freerouting.jar`);
#                        routed with the plain Freerouting-plugin flow for comparison
#        OUT             output directory (default /tmp/fastroute-kicad-e2e)
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
APP=${KICAD_APP:-/Applications/KiCad/KiCad.app}
PY="$APP/Contents/Frameworks/Python.framework/Versions/Current/bin/python3"
KCLI="$APP/Contents/MacOS/kicad-cli"
export FASTROUTE_BIN=${FASTROUTE_BIN:-${CARGO_TARGET_DIR:-$ROOT/target}/release/fastroute}
OUT=${OUT:-/tmp/fastroute-kicad-e2e}
mkdir -p "$OUT"
[ $# -gt 0 ] || set -- "$ROOT"/reference/kicad-demos/*.kicad_pcb

summary() {
  python3 - "$1" <<'EOF'
import json, sys
from collections import Counter
d = json.load(open(sys.argv[1]))
c = Counter(v["type"] for v in d["violations"])
routing = {k: v for k, v in c.items() if k in
           ("clearance", "track_width", "shorting_items", "hole_clearance", "via_diameter",
            "copper_edge_clearance", "starved_thermal", "tracks_crossing", "annular_width")}
print(f"unconnected={len(d['unconnected_items'])} routing_violations={sum(routing.values())} {dict(routing)}")
EOF
}

run() { # name board flavour args...
  name=$1 board=$2 flavour=$3; shift 3
  out="$OUT/$name.$flavour.kicad_pcb"
  start=$(python3 -c 'import time;print(time.time())')
  "$PY" "$ROOT/integrations/kicad/plugins/route_cli.py" "$board" -o "$out" --clear -q "$@" \
    2>&1 | grep -v "^swig\|assert\|^WARN" | tail -1
  end=$(python3 -c 'import time;print(time.time())')
  "$KCLI" pcb drc --format json -o "$out.drc.json" "$out" >/dev/null 2>&1 || true
  printf "%-22s %-12s %6.1fs  %s\n" "$name" "$flavour" "$(python3 -c "print($end-$start)")" \
    "$(summary "$out.drc.json")"
}

for board in "$@"; do
  name=$(basename "$board" .kicad_pcb)
  "$KCLI" pcb drc --format json -o "$OUT/$name.original.drc.json" "$board" >/dev/null 2>&1 || true
  printf "%-22s %-12s %7s  %s\n" "$name" "original" "" "$(summary "$OUT/$name.original.drc.json")"
  run "$name" "$board" fastroute
  if [ -n "${FREEROUTING_CMD:-}" ]; then
    run "$name" "$board" freerouting --fastroute "$FREEROUTING_CMD" \
      --zones-as-planes --no-refill --neckdown on --no-text-keepouts
  fi
done
