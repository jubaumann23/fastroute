#!/usr/bin/env bash
# pcbkit fork gate: FAST unit/contract tests only (target: well under 60 s on a warm build).
# Owner rule (2026-10-09): the test suite is unit tests that check "x does y", not runs of real
# boards. Anything that routes corpus boards, sweeps determinism, starts many servers, runs the
# Java parity, builds a second binary or runs the toolkit conformance lives in
# scripts/pcbkit-bench.sh (on demand), never here.
#
# Usage: pcbkit-gate.sh          (no arguments)
#   0. ledger: changed paths vs the PATCH LEDGER in docs/PCBKIT.md
#   1. cargo test --release --workspace (cheaper release flavour, own target dir), skipping SKIP_UPSTREAM (upstream-owned slow tests, run by
#      `pcbkit-bench.sh upstream`) and not running #[ignore]d tests (run by `pcbkit-bench.sh ignored`)
#   2. no test may skip itself for a missing input (skip lines in the test output)
# Env: CARGO_SLOT  cargo wrapper (shared build slots); CARGO_SLOT=cargo for plain cargo (required)
# Optional: PCBKIT_BASE (ledger base, default v0.1.13).
set -u
if [ $# -ne 0 ]; then
  echo "pcbkit-gate: takes no arguments (usage: $0); on-demand checks are in scripts/pcbkit-bench.sh" >&2
  exit 2
fi
if [ -z "${CARGO_SLOT:-}" ]; then
  echo "pcbkit-gate: required env not set: CARGO_SLOT (CARGO_SLOT=cargo for plain cargo)" >&2
  exit 2
fi
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT" || exit 2

# The main checkout's reference/ (git-ignored; parity baselines the upstream tests read) is shared
# by every worktree.
SHARED_REF=$(dirname "$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir)")/reference
if [ ! -e reference ] && [ -d "$SHARED_REF" ]; then
  ln -s "$SHARED_REF" reference
fi

# Upstream tests (files this fork did not add; kept unedited so the rebase stays cheap) that route
# real boards or sweep determinism and take over ~1 s. They run in `pcbkit-bench.sh upstream`.
SKIP_UPSTREAM=(
  autoroute_operations_match_java       # fr-engine board_replay: replays a Java trace on a real board, ~6 s
  parallel_optimizer_is_deterministic   # fastroute pipeline_parity: repeated optimizer runs, ~1.8 s
  bm02_full_pipeline_matches_java       # fastroute pipeline_parity: full DAC2020 bm02 route, ~1 s
)

# The tiny-fixture tests do not need the shipped profile (thin LTO, one codegen unit, debug info),
# which makes a rebuild after a one-file edit take over a minute. The gate builds its own cheaper
# release flavour in its own target dir, so target/release keeps the real profile for the bench.
export CARGO_TARGET_DIR="$ROOT/target/pcbkit-gate"
export CARGO_PROFILE_RELEASE_LTO=off
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
export CARGO_PROFILE_RELEASE_DEBUG=0
export CARGO_PROFILE_RELEASE_INCREMENTAL=false

LOGDIR=$(mktemp -d "${TMPDIR:-/tmp}/pcbkit-gate.XXXXXX")
ROWS=()
FAIL=0
row() { # name status detail
  ROWS+=("$(printf '%-16s %-6s %s' "$1" "$2" "$3")")
  [ "$2" = FAIL ] && FAIL=1
}

# 0. patch ledger: every changed upstream path must be listed in docs/PCBKIT.md.
step_ledger() {
  local BASE=${PCBKIT_BASE:-v0.1.13} CHANGED LEDGER UNLISTED f CORE
  if CHANGED=$(git diff --name-only "$BASE"..HEAD 2>"$LOGDIR/ledger.err"); then
    LEDGER=$(awk '/^## PATCH LEDGER/{f=1} f && /^\|/' docs/PCBKIT.md)
    UNLISTED=""
    while IFS= read -r f; do
      [ -z "$f" ] && continue
      case "$f" in
        crates/fr-serve/*|crates/*/tests/pcbkit_*.rs|crates/fastroute/tests/serve_*.rs|scripts/pcbkit-*.sh|scripts/pcbkit/*|docs/PCBKIT.md|docs/PCBKIT-*.md|Cargo.lock) continue ;;
      esac
      printf '%s' "$LEDGER" | grep -qF -- "$f" || UNLISTED="$UNLISTED $f"
    done <<<"$CHANGED"
    if [ -z "$UNLISTED" ]; then row ledger PASS "all changed paths listed or exempt"; else row ledger FAIL "unlisted:$UNLISTED"; fi
    CORE=$(printf '%s\n' "$CHANGED" | grep -v -e '^crates/fr-serve/' -e '^crates/[^/]*/tests/' -e '^scripts/pcbkit-' -e '^scripts/pcbkit/' -e '^docs/PCBKIT.md$' -e '^docs/PCBKIT-[A-Z-]*\.md$' -e '^Cargo.lock$' -e '^$')
    row core-files INFO "$(printf '%s\n' "$CORE" | grep -c .) vs $BASE: $(printf '%s\n' "$CORE" | paste -sd' ')"
  else
    row ledger FAIL "git diff $BASE..HEAD failed: $(head -1 "$LOGDIR/ledger.err")"
  fi
}

# 1 + 2. the fast tests, and the missing-input skip rule over their output.
step_tests() {
  local tlog=$LOGDIR/tests.log rc=0 skips=() s executed ignored BADSKIP
  for s in "${SKIP_UPSTREAM[@]}"; do skips+=(--skip "$s"); done
  "$CARGO_SLOT" test --release --workspace --no-fail-fast -- --nocapture "${skips[@]}" >"$tlog" 2>&1 || rc=1
  executed=$(awk '/^test result:/ {for(i=1;i<=NF;i++) if($i=="passed;") s+=$(i-1)} END{print s+0}' "$tlog")
  ignored=$(awk '/^test result:/ {for(i=1;i<=NF;i++) if($i=="ignored;") s+=$(i-1)} END{print s+0}' "$tlog")
  if [ "$rc" -ne 0 ]; then
    row cargo-test FAIL "see $tlog"
  else
    row cargo-test PASS "executed=$executed, on-demand (ignored)=$ignored, upstream-skipped=${#SKIP_UPSTREAM[@]}"
  fi
  # A skip is only legitimate for a documented lexer divergence; the rest are missing-input skips.
  BADSKIP=$(grep -E 'skipped|skipping' "$tlog" | grep -v 'known fr-dsn lexer divergence' | grep -v '^test ' || true)
  if [ -n "$BADSKIP" ]; then
    row skips FAIL "$(printf '%s\n' "$BADSKIP" | grep -c .) skip line(s); first: $(printf '%s\n' "$BADSKIP" | head -1)"
  else
    row skips PASS "no test skipped itself"
  fi
}

START=$(date +%s)
step_ledger
step_tests
echo "== pcbkit-gate =="
printf '%s\n' "${ROWS[@]}"
echo "wall: $(($(date +%s) - START)) s   logs: $LOGDIR"
if [ "$FAIL" -eq 0 ]; then echo "GATE: PASS"; exit 0; fi
echo "GATE: FAIL"; exit 1
