#!/usr/bin/env bash
# A/B determinism tool: routes a fixed corpus with two fastroute binaries and cmp's the SES.
# Usage: scripts/pcbkit-ab.sh [--parity] [--quick] <base-bin> <new-bin> [threads...]   (default threads: 1 4)
#   Corpus: crates/fr-io/testdata/dsn/*.dsn plus reference/pcbkit-corpus/**/*.dsn.
#   Every route runs with --no-time-limits and both thread flags set to N.
#   Byte-identical DSNs in the corpus are routed once (sha256 dedupe).
#   --parity adds --parity to every route. --quick skips reference/pcbkit-corpus/runs/ (the full
#   corpus takes about an hour: boards need up to ~20 s per route, times 2 binaries, times threads).
# Env: PCBKIT_AB_TIMEOUT (seconds per route, default 600).
# Exit 0 only if every file is SAME at every thread count.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
EXTRA=()
QUICK=0
while [ $# -gt 0 ]; do
  case "$1" in
    --parity) EXTRA=(--parity); shift ;;
    --quick) QUICK=1; shift ;;
    *) break ;;
  esac
done
if [ $# -lt 2 ]; then echo "usage: $0 [--parity] [--quick] <base-bin> <new-bin> [threads...]" >&2; exit 2; fi
BASE=$1; NEW=$2; shift 2
THREADS=("$@"); [ ${#THREADS[@]} -eq 0 ] && THREADS=(1 4)
TMO=${PCBKIT_AB_TIMEOUT:-600}
for b in "$BASE" "$NEW"; do [ -x "$b" ] || { echo "not executable: $b" >&2; exit 2; }; done
OUT=$(mktemp -d "${TMPDIR:-/tmp}/pcbkit-ab.XXXXXX")

CORPUS_FIND=(find "$ROOT"/reference/pcbkit-corpus/ -name '*.dsn')
[ "$QUICK" = 1 ] && CORPUS_FIND+=(-not -path '*/runs/*')
declare -A SEEN
FILES=()
while IFS= read -r f; do
  h=$(sha256sum "$f" | cut -d' ' -f1)
  [ -n "${SEEN[$h]:-}" ] && continue
  SEEN[$h]=1
  FILES+=("$f")
done < <({ ls "$ROOT"/crates/fr-io/testdata/dsn/*.dsn 2>/dev/null; "${CORPUS_FIND[@]}" 2>/dev/null; } | sort)
[ ${#FILES[@]} -gt 0 ] || { echo "no corpus files" >&2; exit 2; }

route() { # bin dsn ses threads -> prints wall seconds
  local s e
  s=$(date +%s.%N)
  timeout "$TMO" "$1" -de "$2" -do "$3" --no-time-limits "${EXTRA[@]}" \
    --router.autorouter.max_threads="$4" --router.optimizer.max_threads="$4" >"$3.log" 2>&1
  e=$(date +%s.%N)
  awk -v s="$s" -v e="$e" 'BEGIN{printf "%.1f", e-s}'
}

ALL=0
printf '%-8s %-6s %7s %7s  %s\n' RESULT THREADS BASE_s NEW_s FILE
for f in "${FILES[@]}"; do
  rel=${f#"$ROOT"/}
  tag=$(printf '%s' "$rel" | tr '/ ' '__')
  for t in "${THREADS[@]}"; do
    # Both routes run concurrently (output is deterministic per thread count; only wall time shifts).
    route "$BASE" "$f" "$OUT/$tag.$t.base.ses" "$t" >"$OUT/$tag.$t.tb" &
    route "$NEW" "$f" "$OUT/$tag.$t.new.ses" "$t" >"$OUT/$tag.$t.tn" &
    wait
    tb=$(cat "$OUT/$tag.$t.tb"); tn=$(cat "$OUT/$tag.$t.tn")
    if [ -s "$OUT/$tag.$t.base.ses" ] && cmp -s "$OUT/$tag.$t.base.ses" "$OUT/$tag.$t.new.ses"; then r=SAME
    elif [ ! -s "$OUT/$tag.$t.base.ses" ] && [ ! -s "$OUT/$tag.$t.new.ses" ]; then
      # Both reject the input (e.g. a deliberately invalid DSN): same only if the error line matches.
      if [ "$(tail -n 1 "$OUT/$tag.$t.base.ses.log")" = "$(tail -n 1 "$OUT/$tag.$t.new.ses.log")" ]; then r=SAME-ERR
      else r=DIFF-ERR; ALL=1; fi
    else r=DIFF; ALL=1; fi
    printf '%-8s %-6s %7s %7s  %s\n' "$r" "$t" "$tb" "$tn" "$rel"
  done
done
echo "outputs: $OUT"
[ "$ALL" -eq 0 ] && echo "AB: ALL SAME" || echo "AB: DIFFERENCES"
exit "$ALL"
