#!/usr/bin/env bash
# A/B determinism tool: routes a fixed corpus with two fastroute binaries and cmp's the SES.
# Usage: scripts/pcbkit-ab.sh [--parity] [--quick] [--shard i/n] <base-bin> <new-bin> [threads...]   (default threads: 1 4)
#        scripts/pcbkit-ab.sh --summarize [--parity] [--quick] --shard-count n <base-bin> <new-bin> [threads...]
#   Corpus: crates/fr-io/testdata/dsn/*.dsn plus reference/pcbkit-corpus/**/*.dsn.
#   Every route runs with --no-time-limits and both thread flags set to N.
#   Byte-identical DSNs in the corpus are routed once (sha256 dedupe).
#   --parity adds --parity to every route. --quick skips reference/pcbkit-corpus/runs/ (the full
#   corpus takes about an hour: boards need up to ~20 s per route, times 2 binaries, times threads).
#   --shard i/n (1 <= i <= n) routes every n-th file (the i-th, i+n-th, ...) of the sorted, deduplicated
#   list and also writes the result table to
#   target/pcbkit-ab/<base-sha>-<new-sha>-<mode>-<i>of<n>.txt, where <mode> is parity|default plus
#   -t<threads joined by '+'> (so t1 and t4 runs do not collide) and the shas are PCBKIT_AB_BASE_SHA /
#   PCBKIT_AB_NEW_SHA or else the first 12 hex of sha256 of the binary.
#   --summarize (with --shard-count n and the same binaries, mode and threads) fails unless all n shard
#   files exist, together hold exactly one row per (file, thread count) of the whole list, and every
#   row is SAME or SAME-ERR; it routes nothing.
# Env: PCBKIT_AB_TIMEOUT (seconds per route, default 600).
# Exit 0 only if every file is SAME at every thread count.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
EXTRA=()
QUICK=0
SHARD=""
SHARDS=0
SUMMARIZE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --parity) EXTRA=(--parity); shift ;;
    --quick) QUICK=1; shift ;;
    --shard) SHARD=${2:-}; shift 2 ;;
    --shard-count) SHARDS=${2:-}; shift 2 ;;
    --summarize) SUMMARIZE=1; shift ;;
    *) break ;;
  esac
done
if [ $# -lt 2 ]; then echo "usage: $0 [--parity] [--quick] [--shard i/n | --summarize --shard-count n] <base-bin> <new-bin> [threads...]" >&2; exit 2; fi
SHARD_I=0; SHARD_N=1
if [ -n "$SHARD" ]; then
  if ! [[ "$SHARD" =~ ^([0-9]+)/([0-9]+)$ ]]; then echo "bad --shard '$SHARD' (want i/n)" >&2; exit 2; fi
  SHARD_I=${BASH_REMATCH[1]}; SHARD_N=${BASH_REMATCH[2]}
  if [ "$SHARD_N" -lt 1 ] || [ "$SHARD_I" -lt 1 ] || [ "$SHARD_I" -gt "$SHARD_N" ]; then echo "--shard needs 1 <= i <= n" >&2; exit 2; fi
fi
if [ "$SUMMARIZE" = 1 ] && ! [[ "$SHARDS" =~ ^[1-9][0-9]*$ ]]; then echo "--summarize needs --shard-count n" >&2; exit 2; fi
BASE=$1; NEW=$2; shift 2
THREADS=("$@"); [ ${#THREADS[@]} -eq 0 ] && THREADS=(1 4)
TMO=${PCBKIT_AB_TIMEOUT:-600}
for b in "$BASE" "$NEW"; do [ -x "$b" ] || { echo "not executable: $b" >&2; exit 2; }; done
binsha() { printf '%s' "$(sha256sum "$1" | cut -c1-12)"; }
BASE_SHA=${PCBKIT_AB_BASE_SHA:-$(binsha "$BASE")}
NEW_SHA=${PCBKIT_AB_NEW_SHA:-$(binsha "$NEW")}
MODE=default; [ ${#EXTRA[@]} -gt 0 ] && MODE=parity
TJOIN=$(IFS=+; echo "${THREADS[*]}")
MODE="$MODE-t$TJOIN"
TABLE_DIR="$ROOT/target/pcbkit-ab"
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

if [ "$SUMMARIZE" = 1 ]; then
  rm -rf "$OUT"
  want=$(( ${#FILES[@]} * ${#THREADS[@]} ))
  bad=0; got=0
  for ((i = 1; i <= SHARDS; i++)); do
    tf="$TABLE_DIR/$BASE_SHA-$NEW_SHA-$MODE-${i}of${SHARDS}.txt"
    if [ ! -f "$tf" ]; then echo "MISSING shard $i/$SHARDS: $tf"; bad=1; continue; fi
    if ! grep -q '^AB-SHARD-DONE' "$tf"; then echo "INCOMPLETE shard $i/$SHARDS: $tf"; bad=1; fi
    n=$(grep -cE '^(SAME|SAME-ERR|DIFF|DIFF-ERR) ' "$tf"); got=$((got + n))
    nb=$(grep -cE '^(DIFF|DIFF-ERR) ' "$tf")
    [ "$nb" -gt 0 ] && { echo "shard $i/$SHARDS has $nb DIFF row(s):"; grep -E '^(DIFF|DIFF-ERR) ' "$tf"; bad=1; }
  done
  [ "$got" -eq "$want" ] || { echo "row count $got != expected $want (files ${#FILES[@]} x threads ${#THREADS[@]})"; bad=1; }
  echo "AB summary $BASE_SHA..$NEW_SHA $MODE shards=$SHARDS rows=$got/$want"
  if [ "$bad" -eq 0 ]; then echo "AB: ALL SAME"; exit 0; fi
  echo "AB: DIFFERENCES OR GAPS"; exit 1
fi

if [ -n "$SHARD" ]; then
  SEL=()
  for idx in "${!FILES[@]}"; do [ $((idx % SHARD_N)) -eq $((SHARD_I - 1)) ] && SEL+=("${FILES[$idx]}"); done
  FILES=("${SEL[@]}")
  mkdir -p "$TABLE_DIR"
  TABLE="$TABLE_DIR/$BASE_SHA-$NEW_SHA-$MODE-${SHARD_I}of${SHARD_N}.txt"
  : >"$TABLE"
else
  TABLE=/dev/null
fi

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
    printf '%-8s %-6s %7s %7s  %s\n' "$r" "$t" "$tb" "$tn" "$rel" | tee -a "$TABLE"
  done
done
echo "outputs: $OUT"
[ "$TABLE" != /dev/null ] && echo "AB-SHARD-DONE $SHARD_I/$SHARD_N rc=$ALL" >>"$TABLE"
[ "$ALL" -eq 0 ] && echo "AB: ALL SAME" || echo "AB: DIFFERENCES"
exit "$ALL"
