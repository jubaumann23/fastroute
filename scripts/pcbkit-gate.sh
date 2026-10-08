#!/usr/bin/env bash
# pcbkit fork gate. Run from any fork worktree; exit 0 only if every applicable step passes.
#   0. ledger: changed paths vs the PATCH LEDGER in docs/PCBKIT.md
#   1. cargo test --release --workspace (no parity test may be silently skipped)
#   2. scripts/parity-route.sh on the pipeline_parity fixtures (needs the Java parity jar)
#   3. router_conformance.py at --threads 1 and 2 (only if the built binary has `serve`)
# Env: CARGO_SLOT (cargo wrapper), PCBKIT_CONFORMANCE (runner path), PCBKIT_JAVA (java binary).
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SHARED_REF=/home/jubau/coolProjects/fastroute/reference
SHARED_SLOT=/tmp/claude-1000/-home-jubau-coolProjects-claude-pcb-rules/09ebfdde-5d90-4ee9-adf5-b5e4340131ee/scratchpad/cargo_slot.sh
if [ -x "$SHARED_SLOT" ]; then DEFAULT_SLOT=$SHARED_SLOT; else DEFAULT_SLOT=cargo; fi
CARGO_SLOT=${CARGO_SLOT:-$DEFAULT_SLOT}
CONF=${PCBKIT_CONFORMANCE:-/home/jubau/coolProjects/claude-pcb-rules/.claude/worktrees/rust-protocol/scripts/router_conformance.py}
cd "$ROOT" || exit 2

if [ ! -e reference ] && [ -d "$SHARED_REF" ]; then
  ln -s "$SHARED_REF" reference
fi

LOGDIR=$(mktemp -d "${TMPDIR:-/tmp}/pcbkit-gate.XXXXXX")
ROWS=()
FAIL=0
row() { # name status detail
  ROWS+=("$(printf '%-28s %-6s %s' "$1" "$2" "$3")")
  [ "$2" = FAIL ] && FAIL=1
}

# 0. patch ledger: every changed upstream path must be listed in docs/PCBKIT.md.
BASE=${PCBKIT_BASE:-v0.1.13}
if CHANGED=$(git diff --name-only "$BASE"..HEAD 2>"$LOGDIR/ledger.err"); then
  LEDGER=$(awk '/^## PATCH LEDGER/{f=1} f && /^\|/' docs/PCBKIT.md)
  UNLISTED=""
  while IFS= read -r f; do
    [ -z "$f" ] && continue
    case "$f" in
      crates/fr-serve/*|crates/*/tests/pcbkit_*.rs|crates/fastroute/tests/serve_*.rs|scripts/pcbkit-*.sh|docs/PCBKIT.md|Cargo.lock) continue ;;
    esac
    printf '%s' "$LEDGER" | grep -qF -- "$f" || UNLISTED="$UNLISTED $f"
  done <<<"$CHANGED"
  if [ -z "$UNLISTED" ]; then row ledger PASS "all changed paths listed or exempt"; else row ledger FAIL "unlisted:$UNLISTED"; fi
else
  row ledger FAIL "git diff $BASE..HEAD failed: $(head -1 "$LOGDIR/ledger.err")"
fi

# 1. workspace tests, output kept so skip lines can be counted.
TLOG=$LOGDIR/test.log
if "$CARGO_SLOT" test --release --workspace --no-fail-fast -- --nocapture >"$TLOG" 2>&1; then
  T_RC=0
else
  T_RC=1
fi
PASSED=$(awk '/^test result:/ {for(i=1;i<=NF;i++) if($i=="passed;") s+=$(i-1)} END{print s+0}' "$TLOG")
# Legit skip: a documented lexer divergence; everything else is a missing-input skip.
BADSKIP=$(grep -E 'skipped|skipping' "$TLOG" | grep -v 'known fr-dsn lexer divergence' | grep -v '^test ' || true)
NSKIP=$(printf '%s' "$BADSKIP" | grep -c . || true)
NDIV=$(grep -c 'known fr-dsn lexer divergence' "$TLOG" || true)
if [ "$T_RC" -ne 0 ]; then
  row cargo-test FAIL "see $TLOG"
else
  row cargo-test PASS "executed=$PASSED tests"
fi
if [ "$NSKIP" -gt 0 ]; then
  if [ -d reference/freerouting/scripts/benchmark/fixtures ]; then
    row parity-skips FAIL "$NSKIP skip line(s) with inputs present; first: $(printf '%s' "$BADSKIP" | head -1)"
  else
    row parity-skips FAIL "$NSKIP skip line(s); reference/freerouting missing (see docs/PCBKIT.md)"
  fi
else
  row parity-skips PASS "skipped=0 (documented lexer-divergence skips: $NDIV)"
fi

# 2. Java parity routes.
JAVA=${PCBKIT_JAVA:-reference/jdk25/bin/java}
if [ -x "$JAVA" ] && [ -f reference/bin/freerouting-parity.jar ]; then
  FIX=reference/freerouting/scripts/benchmark/fixtures
  P_OK=1
  for spec in "DAC2020_boards/DAC2020_bm02.dsn bm02_full.ses" \
              "KiCad_10_demos/pic_programmer.dsn pic_programmer_noopt.ses --router.optimizer.enabled=false"; do
    read -ra parts <<<"$spec"
    out=$LOGDIR/parity-$(basename "${parts[0]}" .dsn)
    extra=("${parts[@]:2}")
    # parity-route.sh also diffs log pass lines (the Rust log may carry extra annotations);
    # the SES bytes are the contract, so judge on `ses=IDENTICAL`.
    FASTROUTE=$ROOT/target/release/fastroute scripts/parity-route.sh "$FIX/${parts[0]}" "$out" "${extra[@]}" >"$out.log" 2>&1
    grep -q 'ses=IDENTICAL' "$out.log" || P_OK=0
  done
  if [ "$P_OK" = 1 ]; then row parity-route PASS "java vs rust SES identical"; else row parity-route FAIL "see $LOGDIR/parity-*.log"; fi
else
  row parity-route SKIP "no Java parity jar/jdk in reference/ (docs/PCBKIT.md)"
fi

# 3. protocol conformance.
BIN=$ROOT/target/release/fastroute
# `serve` is not listed in --help; a binary with serve exits 0 on an empty stdin, one
# without it rejects `serve` as a CLI argument.
if "$BIN" serve </dev/null >/dev/null 2>&1; then
  if [ ! -f "$CONF" ]; then
    row conformance FAIL "runner missing: $CONF"
  else
    EXTRA=()
    python3 "$CONF" --help 2>&1 | grep -q -- '--stock-cli' && EXTRA=(--stock-cli "$BIN")
    for th in 1 2; do
      if python3 "$CONF" --server "$BIN serve" --threads "$th" "${EXTRA[@]}" >"$LOGDIR/conf-$th.log" 2>&1; then
        row "conformance-t$th" PASS ""
      else
        row "conformance-t$th" FAIL "see $LOGDIR/conf-$th.log"
      fi
    done
  fi
else
  row conformance SKIP "binary has no serve subcommand"
fi

echo "== pcbkit-gate =="
printf '%s\n' "${ROWS[@]}"
echo "logs: $LOGDIR"
if [ "$FAIL" -eq 0 ]; then echo "GATE: PASS"; exit 0; fi
echo "GATE: FAIL"; exit 1
