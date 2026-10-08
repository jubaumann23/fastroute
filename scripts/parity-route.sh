#!/bin/sh
# Routes one board with the Java parity build and with fastroute --parity (same extra
# arguments), then compares the SES files byte by byte and the per-pass results
# (fanout / autorouter / optimizer pass lines) of both logs. Reports wall time and peak RSS.
#
# Usage: scripts/parity-route.sh board.dsn outdir [extra freerouting args]
#   e.g. scripts/parity-route.sh fixtures/x.dsn /tmp/p --router.optimizer.enabled=false -mp 3
# Env: FASTROUTE (default $CARGO_TARGET_DIR/release/fastroute), SKIP_JAVA=1 (reuse the Java
# output of a previous run in outdir).
# Exit status: 0 if SES and pass lines are identical, 1 otherwise.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
DSN=$1; OUT=$2; shift 2
mkdir -p "$OUT"
# macOS: time -l (RSS in bytes). GNU time (Linux): -f prints the same two line shapes (RSS in kB).
if /usr/bin/time -l true >/dev/null 2>&1; then
  TIMECMD="/usr/bin/time -l"; RSS_DIV=1048576
else
  TIMECMD="/usr/bin/time -f %e_real\n%M_maximum_resident_set_size"; RSS_DIV=1024
fi
export RSS_DIV
NAME=$(basename "$DSN" .dsn)
FASTROUTE=${FASTROUTE:-${CARGO_TARGET_DIR:-$ROOT/target}/release/fastroute}

if [ "${SKIP_JAVA:-0}" != 1 ] || [ ! -f "$OUT/$NAME.java.ses" ]; then
  $TIMECMD "$ROOT/scripts/java-parity.sh" "$DSN" "$OUT/$NAME.java.ses" "$@" > "$OUT/$NAME.java.log" 2>&1
fi
$TIMECMD "$FASTROUTE" -de "$DSN" -do "$OUT/$NAME.rust.ses" --parity "$@" > "$OUT/$NAME.rust.log" 2>&1

python3 - "$OUT/$NAME" <<'EOF'
import re, sys
base = sys.argv[1]
def passes(path):
    out = []
    for line in open(path, errors="replace"):
        line = line.rstrip("\n")
        line = re.sub(r" on board '[0-9a-f]+'", "", line)
        m = re.search(r"(Fanout pass #\d+) completed in [0-9.]+ seconds (with .*)$", line)
        if m:
            out.append(m.group(1) + " " + m.group(2)); continue
        m = re.search(r"(Auto-routing pass #\d+) was completed in [0-9.]+ seconds with score ([0-9.]+ \([^)]*\))", line)
        if m:
            out.append(m.group(1) + " " + m.group(2)); continue
        m = re.search(r"(Optimizer pass #\d+) was completed in [0-9.]+ seconds.* with the score of ([0-9.]+ \([^)]*\))", line)
        if m:
            out.append(m.group(1) + " " + m.group(2)); continue
        m = re.search(r"(Optimizer pass #\d+: optimizer score .*)$", line)
        if m:
            out.append(m.group(1)); continue
        m = re.search(r"(Optimizer pass #\d+ candidate rejected: \w+)", line)
        if m:
            out.append(m.group(1)); continue
    return out
def usage(path):
    real = rss = None
    for line in open(path, errors="replace"):
        m = re.match(r"\s*([0-9.]+)[ _]real", line)
        if m: real = float(m.group(1))
        m = re.match(r"\s*(\d+)[\s_]+maximum[\s_]+resident[\s_]+set[\s_]+size", line)
        if m: rss = int(m.group(1)) / float(__import__("os").environ.get("RSS_DIV", "1048576"))
    return real, rss
j, r = passes(base + ".java.log"), passes(base + ".rust.log")
sj = open(base + ".java.ses", "rb").read() if __import__("os").path.exists(base + ".java.ses") else b""
sr = open(base + ".rust.ses", "rb").read() if __import__("os").path.exists(base + ".rust.ses") else b""
ok = True
n = max(len(j), len(r))
first = None
for i in range(n):
    a = j[i] if i < len(j) else "<missing>"
    b = r[i] if i < len(r) else "<missing>"
    if a != b and first is None:
        first = (i, a, b)
if first:
    ok = False
    print(f"PASS LINES DIFFER at #{first[0]}:\n  java: {first[1]}\n  rust: {first[2]}")
ses_ok = sj == sr and len(sj) > 0
if not ses_ok:
    ok = False
tl = sum(1 for l in open(base + ".java.log", errors="replace") if "PARITY_TIME_LIMIT" in l)
tlr = sum(1 for l in open(base + ".rust.log", errors="replace") if "PARITY_TIME_LIMIT" in l)
(jr, jm), (rr, rm) = usage(base + ".java.log"), usage(base + ".rust.log")
name = base.rsplit("/", 1)[-1]
print(f"{name}\tses={'IDENTICAL' if ses_ok else 'DIFFERENT'}\tpasses={len(j)}/{len(r)} {'same' if not first else 'differ'}"
      f"\tjava={jr}s/{jm:.0f}MB\trust={rr}s/{rm:.0f}MB\ttime_limit_marks java={tl} rust={tlr}")
if len(j) > 0:
    print("  last java:", j[-1])
    print("  last rust:", r[-1] if r else "-")
sys.exit(0 if ok else 1)
EOF
