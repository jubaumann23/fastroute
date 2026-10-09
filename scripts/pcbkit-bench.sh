#!/usr/bin/env bash
# pcbkit on-demand checks: everything too slow for the gate (scripts/pcbkit-gate.sh, seconds).
# Owner rule (2026-10-09): the gate is unit tests only; anything that routes real/corpus boards,
# sweeps determinism, starts many servers, runs the Java parity, builds a second binary or runs the
# toolkit conformance against real boards is run here, by hand, one subcommand per foreground call.
#
# Usage: pcbkit-bench.sh <subcommand> [cargo test args for `ignored`]
#   ignored [args]  our #[ignore]d corpus/board/load tests (`cargo test --release -- --ignored`), all
#                   except the determinism sweep. Takes ~9 min in total: pass cargo args to split it,
#                   e.g. `ignored -p fastroute --test serve_snapshot` (see docs/PCBKIT.md for times)
#   determinism     serve_determinism_load: 8 oversubscribed concurrent servers, ~100 to 320 s
#   upstream        upstream's own slow tests skipped by the gate (SKIP_UPSTREAM in pcbkit-gate.sh):
#                   board_replay autoroute_operations_match_java, pipeline_parity
#                   parallel_optimizer_is_deterministic and bm02_full_pipeline_matches_java
#   parity-route    Java vs Rust SES identity on bm02 and pic_programmer (needs the Java parity jar)
#   stock-pin       the pinned stock v0.1.13 binary matches STOCK_CLI_SHA256 in docs/PCBKIT.md
#   plain-bin       build the shipped binary (no test-hooks) into target/pcbkit-plain; no
#                   FR_SERVE_TEST_PANIC string may be in it
#   conformance     router_conformance.py at --threads 1 and 2 on the plain binary (builds it), against
#                   the pinned toolkit contract (CONTRACT_COMMIT) and the pinned stock binary
#   all             every subcommand above in this order (far over a 600 s foreground limit)
# Env: CARGO_SLOT (required; CARGO_SLOT=cargo for plain cargo)
#      PCBKIT_CONFORMANCE  toolkit scripts/router_conformance.py   (conformance)
#      PCBKIT_STOCK_CLI    pinned stock fastroute v0.1.13 binary   (stock-pin, conformance)
#      PCBKIT_JAVA         java binary (parity-route; default reference/jdk25/bin/java)
set -u
SUBCOMMANDS="ignored determinism upstream parity-route stock-pin plain-bin conformance"
usage() { echo "usage: $0 {${SUBCOMMANDS// /|}|all} [cargo args for ignored]" >&2; exit 2; }
[ $# -ge 1 ] || usage
CMD=$1
shift
case " $SUBCOMMANDS all " in *" $CMD "*) ;; *) usage ;; esac
[ "$CMD" = ignored ] || [ $# -eq 0 ] || usage

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT" || exit 2
MISSING=""
need() { local v; for v in "$@"; do [ -n "${!v:-}" ] || MISSING="$MISSING $v"; done; }
need CARGO_SLOT
case "$CMD" in
  stock-pin) need PCBKIT_STOCK_CLI ;;
  conformance|all) need PCBKIT_CONFORMANCE PCBKIT_STOCK_CLI ;;
esac
if [ -n "$MISSING" ]; then
  echo "pcbkit-bench: required env not set:$MISSING (see the header of $0)" >&2
  exit 2
fi
CONF=${PCBKIT_CONFORMANCE:-}
STOCK=${PCBKIT_STOCK_CLI:-}

SHARED_REF=$(dirname "$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir)")/reference
if [ ! -e reference ] && [ -d "$SHARED_REF" ]; then
  ln -s "$SHARED_REF" reference
fi

LOGDIR=$(mktemp -d "${TMPDIR:-/tmp}/pcbkit-bench.XXXXXX")
ROWS=()
FAIL=0
row() { # name status detail
  ROWS+=("$(printf '%-24s %-6s %s' "$1" "$2" "$3")")
  [ "$2" = FAIL ] && FAIL=1
}

# Missing-input skip rule over a test log: a skip is legit only for a documented lexer divergence.
skip_row() { # rowname log
  local bad
  bad=$(grep -E 'skipped|skipping' "$2" | grep -v 'known fr-dsn lexer divergence' | grep -v '^test ' || true)
  if [ -n "$bad" ]; then
    row "$1" FAIL "$(printf '%s\n' "$bad" | grep -c .) skip line(s); first: $(printf '%s\n' "$bad" | head -1)"
  else
    row "$1" PASS "no test skipped itself"
  fi
}

# Runs cargo test, records the row and the skip rule.
cargo_rows() { # rowname log cargo-args...
  local name=$1 log=$2
  shift 2
  if "$CARGO_SLOT" test --release --no-fail-fast "$@" >"$log" 2>&1; then
    row "$name" PASS "$(awk '/^test result:/ {for(i=1;i<=NF;i++) {if($i=="passed;") p+=$(i-1); if($i=="ignored;") g+=$(i-1)}} END{print "passed=" p+0 ", ignored(not run)=" g+0}' "$log")"
  else
    row "$name" FAIL "see $log"
  fi
  skip_row "$name-skips" "$log"
}

run_ignored() {
  [ $# -gt 0 ] || set -- --workspace
  cargo_rows ignored "$LOGDIR/ignored.log" "$@" -- --ignored --nocapture --skip oversubscribed_concurrent_serves_are_byte_identical
}

run_determinism() {
  cargo_rows determinism "$LOGDIR/determinism.log" -p fastroute --test serve_determinism_load -- --ignored --nocapture
}

run_upstream() {
  # whole upstream files (the gate skips only their slow tests by name, SKIP_UPSTREAM in pcbkit-gate.sh)
  cargo_rows upstream-engine "$LOGDIR/upstream-engine.log" -p fr-engine --test board_replay -- --nocapture
  cargo_rows upstream-pipeline "$LOGDIR/upstream-pipeline.log" -p fastroute --test pipeline_parity -- --nocapture
}

run_parity_route() {
  local JAVA=${PCBKIT_JAVA:-reference/jdk25/bin/java} FIX P_OK spec out parts extra bin
  bin=$ROOT/target/release/fastroute
  if [ -x "$JAVA" ] && [ -f reference/bin/freerouting-parity.jar ]; then
    "$CARGO_SLOT" build --release -p fastroute >"$LOGDIR/parity-build.log" 2>&1 || { row parity-route FAIL "build failed, see $LOGDIR/parity-build.log"; return; }
    FIX=reference/freerouting/scripts/benchmark/fixtures
    P_OK=1
    for spec in "DAC2020_boards/DAC2020_bm02.dsn bm02_full.ses" \
                "KiCad_10_demos/pic_programmer.dsn pic_programmer_noopt.ses --router.optimizer.enabled=false"; do
      read -ra parts <<<"$spec"
      out=$LOGDIR/parity-$(basename "${parts[0]}" .dsn)
      extra=("${parts[@]:2}")
      # The SES bytes are the contract; parity-route.sh also diffs log pass lines, which may differ
      # by annotations, so judge on `ses=IDENTICAL`.
      FASTROUTE=$bin scripts/parity-route.sh "$FIX/${parts[0]}" "$out" "${extra[@]}" >"$out.log" 2>&1
      grep -q 'ses=IDENTICAL' "$out.log" || P_OK=0
    done
    if [ "$P_OK" = 1 ]; then row parity-route PASS "java vs rust SES identical"; else row parity-route FAIL "see $LOGDIR/parity-*.log"; fi
  else
    row parity-route SKIP "no Java parity jar/jdk in reference/ (docs/PCBKIT.md)"
  fi
}

run_stock_pin() {
  local WANT_SHA
  WANT_SHA=$(awk '/^STOCK_CLI_SHA256:/{print $2; exit}' docs/PCBKIT.md)
  if [ ! -x "$STOCK" ]; then
    row stock-pin FAIL "PCBKIT_STOCK_CLI not executable: $STOCK"
  elif [ -z "$WANT_SHA" ]; then
    row stock-pin FAIL "no STOCK_CLI_SHA256 line in docs/PCBKIT.md"
  elif [ "$(sha256sum "$STOCK" | cut -d' ' -f1)" != "$WANT_SHA" ]; then
    row stock-pin FAIL "sha256 of $STOCK differs from docs/PCBKIT.md STOCK_CLI_SHA256"
  else
    row stock-pin PASS "$("$STOCK" --version 2>&1 | head -1), sha256 ${WANT_SHA:0:12}"
  fi
}

# The shipped binary: cargo test builds target/release/fastroute WITH test-hooks, so the plain one
# goes into its own target dir.
PLAIN_DIR=$ROOT/target/pcbkit-plain
PLAIN=$PLAIN_DIR/release/fastroute
run_plain_bin() {
  local n
  if ! CARGO_TARGET_DIR=$PLAIN_DIR "$CARGO_SLOT" build --release -p fastroute >"$LOGDIR/plain-build.log" 2>&1; then
    row plain-bin FAIL "build failed, see $LOGDIR/plain-build.log"; return 1
  fi
  n=$(strings "$PLAIN" | grep -c FR_SERVE_TEST_PANIC || true)
  if [ "$n" -ne 0 ]; then
    row plain-bin FAIL "$n FR_SERVE_TEST_PANIC string(s) in the plain release binary"; return 1
  fi
  row plain-bin PASS "0 FR_SERVE_TEST_PANIC strings, sha256 $(sha256sum "$PLAIN" | cut -c1-12)"
}

# The conformance runner must come from the pinned toolkit contract commit (CONTRACT_COMMIT in
# docs/PCBKIT.md), with the runner file unmodified.
contract_pin_ok() {
  local WANT GOT DIR
  WANT=$(awk '/^CONTRACT_COMMIT:/{print $2; exit}' docs/PCBKIT.md)
  DIR=$(cd "$(dirname "$CONF")" && pwd)
  GOT=$(git -C "$DIR" rev-parse HEAD 2>/dev/null)
  if [ -z "$WANT" ]; then
    row contract-pin FAIL "no CONTRACT_COMMIT line in docs/PCBKIT.md"; return 1
  elif [ -z "$GOT" ]; then
    row contract-pin FAIL "runner $CONF is not in a git checkout"; return 1
  elif [ "$GOT" != "$WANT" ]; then
    row contract-pin FAIL "runner checkout at ${GOT:0:12}, pinned ${WANT:0:12} (re-pin docs/PCBKIT.md CONTRACT_COMMIT)"; return 1
  elif ! git -C "$DIR" diff --quiet HEAD -- "$(basename "$CONF")"; then
    row contract-pin FAIL "runner $CONF has local changes against ${GOT:0:12}"; return 1
  fi
  row contract-pin PASS "toolkit contract ${GOT:0:12} ($(git -C "$DIR" log -1 --format=%s HEAD | cut -c1-60))"
}

run_conformance() {
  local EXTRA=() th
  run_plain_bin || return
  if [ ! -f "$CONF" ]; then row conformance FAIL "runner missing: $CONF"; return; fi
  contract_pin_ok || { row conformance FAIL "not run: the runner is not the pinned contract"; return; }
  python3 "$CONF" --help 2>&1 | grep -q -- '--stock-cli' && EXTRA=(--stock-cli "$STOCK")
  for th in 1 2; do
    if python3 "$CONF" --server "$PLAIN serve" --threads "$th" "${EXTRA[@]}" >"$LOGDIR/conf-$th.log" 2>&1; then
      row "conformance-t$th" PASS ""
    else
      row "conformance-t$th" FAIL "see $LOGDIR/conf-$th.log"
    fi
  done
}

START=$(date +%s)
case "$CMD" in
  ignored) run_ignored "$@" ;;
  determinism) run_determinism ;;
  upstream) run_upstream ;;
  parity-route) run_parity_route ;;
  stock-pin) run_stock_pin ;;
  plain-bin) run_plain_bin ;;
  conformance) run_conformance ;;
  all)
    run_ignored
    run_determinism
    run_upstream
    run_parity_route
    run_stock_pin
    run_conformance ;;
esac

echo "== pcbkit-bench $CMD =="
printf '%s\n' "${ROWS[@]}"
echo "wall: $(($(date +%s) - START)) s   logs: $LOGDIR"
if [ "$FAIL" -eq 0 ]; then echo "BENCH: PASS"; exit 0; fi
echo "BENCH: FAIL"; exit 1
