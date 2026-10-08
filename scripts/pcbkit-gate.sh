#!/usr/bin/env bash
# pcbkit fork gate. Run from any fork worktree; exit 0 only if every applicable step passes.
# Usage: pcbkit-gate.sh [--phase <name>]
#   (no flag)        everything in one call, as before (can exceed a 600 s foreground limit)
#   --phase tests-core     workspace tests except the fastroute serve_* corpus targets
#   --phase tests-serve-a  fastroute serve_* targets except move and snapshot
#   --phase tests-serve-b  fastroute serve_move
#   --phase tests-serve-c  fastroute serve_snapshot
#   --phase final          ledger, parity-skips, parity-route, stock-pin, conformance t1/t2, and
#                          refusal unless fresh records of all test phases cover the workspace
# Each test phase writes target/pcbkit-gate/<phase>.<HEAD sha>.{log,rc} (rc holds rc, executed
# count, dirty flag and the sha256 of target/release/fastroute). Run the test phases first.
#   0. ledger: changed paths vs the PATCH LEDGER in docs/PCBKIT.md
#   1. cargo test --release --workspace (no parity test may be silently skipped)
#   2. scripts/parity-route.sh on the pipeline_parity fixtures (needs the Java parity jar)
#   3. router_conformance.py at --threads 1 and 2 (only if the built binary has `serve`)
# Env (required, no machine-specific defaults; test phases need only CARGO_SLOT):
#   CARGO_SLOT          cargo wrapper (shared build slots); set CARGO_SLOT=cargo for plain cargo
#   PCBKIT_CONFORMANCE  path to the toolkit's scripts/router_conformance.py
#   PCBKIT_STOCK_CLI    pinned stock fastroute v0.1.13 binary; its sha256 must equal the
#                       STOCK_CLI_SHA256 line in docs/PCBKIT.md (stock-equivalence reference)
# Optional: PCBKIT_JAVA (java binary), PCBKIT_BASE (ledger base, default v0.1.13).
set -u
PHASE=""
case "${1:-}" in
  "") ;;
  --phase) PHASE=${2:-}; [ $# -eq 2 ] || { echo "pcbkit-gate: --phase needs exactly one name" >&2; exit 2; } ;;
  *) echo "pcbkit-gate: usage: $0 [--phase tests-core|tests-serve-a|tests-serve-b|tests-serve-c|final]" >&2; exit 2 ;;
esac
TEST_PHASES="tests-core tests-serve-a tests-serve-b tests-serve-c"
case " $TEST_PHASES final " in
  *" $PHASE "*) ;;
  *) [ -z "$PHASE" ] || echo "pcbkit-gate: unknown phase '$PHASE' (tests-core tests-serve-a tests-serve-b tests-serve-c final)" >&2
     [ -z "$PHASE" ] || exit 2 ;;
esac
ROOT=$(cd "$(dirname "$0")/.." && pwd)
# The main checkout's reference/ (git-ignored) is shared by every worktree.
SHARED_REF=$(dirname "$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir)")/reference
MISSING=""
REQ="CARGO_SLOT PCBKIT_CONFORMANCE PCBKIT_STOCK_CLI"
case "$PHASE" in tests-*) REQ="CARGO_SLOT" ;; final) REQ="CARGO_SLOT PCBKIT_CONFORMANCE PCBKIT_STOCK_CLI" ;; esac
for v in $REQ; do
  [ -n "${!v:-}" ] || MISSING="$MISSING $v"
done
if [ -n "$MISSING" ]; then
  echo "pcbkit-gate: required env not set:$MISSING (see the header of $0 and docs/PCBKIT.md)" >&2
  exit 2
fi
CONF=${PCBKIT_CONFORMANCE:-}
STOCK=${PCBKIT_STOCK_CLI:-}
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

BIN=$ROOT/target/release/fastroute
REC=$ROOT/target/pcbkit-gate

# --- steps (each appends rows) -------------------------------------------------------------

# 0. patch ledger: every changed upstream path must be listed in docs/PCBKIT.md.
step_ledger() {
  local BASE=${PCBKIT_BASE:-v0.1.13} CHANGED LEDGER UNLISTED f
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
}

# Executed-test count over one or more cargo logs.
count_passed() {
  awk '/^test result:/ {for(i=1;i<=NF;i++) if($i=="passed;") s+=$(i-1)} END{print s+0}' "$@"
}

# Skip-line rule, applied to every test log: a skip is only legit for a documented lexer
# divergence; everything else is a missing-input skip. Sets SKIP_FAIL=1 on failure.
SKIP_FAIL=0
skip_row() { # rowname log...
  local name=$1 BADSKIP NSKIP NDIV
  shift
  BADSKIP=$(cat "$@" | grep -E 'skipped|skipping' | grep -v 'known fr-dsn lexer divergence' | grep -v '^test ' || true)
  NSKIP=$(printf '%s' "$BADSKIP" | grep -c . || true)
  NDIV=$(cat "$@" | grep -c 'known fr-dsn lexer divergence' || true)
  if [ "$NSKIP" -gt 0 ]; then
    SKIP_FAIL=1
    if [ -d reference/freerouting/scripts/benchmark/fixtures ]; then
      row "$name" FAIL "$NSKIP skip line(s) with inputs present; first: $(printf '%s' "$BADSKIP" | head -1)"
    else
      row "$name" FAIL "$NSKIP skip line(s); reference/freerouting missing (see docs/PCBKIT.md)"
    fi
  else
    row "$name" PASS "skipped=0 (documented lexer-divergence skips: $NDIV)"
  fi
}

# Cargo argument sets per test phase. Each line is one cargo invocation.
# fastroute serve_* corpus targets are split by measured wall time (docs/PCBKIT.md).
phase_cargo_args() { # phase
  local f t others=""
  case "$1" in
    tests-core)
      echo "--workspace --exclude fastroute"
      for f in crates/fastroute/tests/*.rs; do
        t=$(basename "$f" .rs)
        case "$t" in serve_*) ;; *) others="$others --test $t" ;; esac
      done
      echo "-p fastroute --bins$others" ;;
    tests-serve-a)
      for f in crates/fastroute/tests/serve_*.rs; do
        t=$(basename "$f" .rs)
        case "$t" in serve_move|serve_snapshot) ;; *) others="$others --test $t" ;; esac
      done
      echo "-p fastroute$others" ;;
    tests-serve-b) echo "-p fastroute --test serve_move" ;;
    tests-serve-c) echo "-p fastroute --test serve_snapshot" ;;
  esac
}

bin_sha() { if [ -f "$BIN" ]; then sha256sum "$BIN" | cut -d' ' -f1; else echo none; fi; }
dirty_flag() { if [ -n "$(git status --porcelain)" ]; then echo 1; else echo 0; fi; }

# One cargo test phase (or, with phase "all", the whole workspace in one call).
step_tests() { # phase
  local phase=$1 head tlog rc=0 args executed
  head=$(git rev-parse HEAD)
  tlog=$LOGDIR/$phase.log
  : >"$tlog"
  if [ "$phase" = all ]; then
    "$CARGO_SLOT" test --release --workspace --no-fail-fast -- --nocapture >"$tlog" 2>&1 || rc=1
  else
    while IFS= read -r args; do
      # shellcheck disable=SC2086 # args is a deliberate word list
      "$CARGO_SLOT" test --release --no-fail-fast $args -- --nocapture >>"$tlog" 2>&1 || rc=1
    done < <(phase_cargo_args "$phase")
  fi
  executed=$(count_passed "$tlog")
  if [ "$rc" -ne 0 ]; then
    row "$( [ "$phase" = all ] && echo cargo-test || echo "$phase" )" FAIL "see $tlog"
  else
    row "$( [ "$phase" = all ] && echo cargo-test || echo "$phase" )" PASS "executed=$executed tests"
  fi
  skip_row parity-skips "$tlog"
  if [ "$phase" != all ]; then
    mkdir -p "$REC"
    rm -f "$REC/$phase".*   # records of older HEADs are stale by definition
    cp "$tlog" "$REC/$phase.$head.log"
    {
      echo "phase=$phase"
      echo "head=$head"
      echo "rc=$((rc | SKIP_FAIL))"
      echo "executed=$executed"
      echo "dirty=$(dirty_flag)"
      echo "binsha=$(bin_sha)"
    } >"$REC/$phase.$head.rc"
  fi
}

# 2. Java parity routes.
step_parity_route() {
  local JAVA=${PCBKIT_JAVA:-reference/jdk25/bin/java} FIX P_OK spec out parts extra
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
      FASTROUTE=$BIN scripts/parity-route.sh "$FIX/${parts[0]}" "$out" "${extra[@]}" >"$out.log" 2>&1
      grep -q 'ses=IDENTICAL' "$out.log" || P_OK=0
    done
    if [ "$P_OK" = 1 ]; then row parity-route PASS "java vs rust SES identical"; else row parity-route FAIL "see $LOGDIR/parity-*.log"; fi
  else
    row parity-route SKIP "no Java parity jar/jdk in reference/ (docs/PCBKIT.md)"
  fi
}

# 3. protocol conformance, stock equivalence against the pinned stock v0.1.13 binary.
step_stock_pin() {
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

step_conformance() {
  local EXTRA=() th
  # `serve` is not listed in --help; a binary with serve exits 0 on an empty stdin, one
  # without it rejects `serve` as a CLI argument.
  if "$BIN" serve </dev/null >/dev/null 2>&1; then
    if [ ! -f "$CONF" ]; then
      row conformance FAIL "runner missing: $CONF"
    else
      python3 "$CONF" --help 2>&1 | grep -q -- '--stock-cli' && EXTRA=(--stock-cli "$STOCK")
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
}

# --- final: refuse unless fresh, complete test-phase records exist --------------------------

# Normalise test names to "<binary id>::<test name>" from either a `-- --list` output or a
# `-- --nocapture` run log. Binary id = crate/test target (hash stripped) or doc:<crate>.
norm_tests() { # list|run file...
  local mode=$1
  shift
  awk -v mode="$mode" '
    /^ *(Running|Doc-tests) / {
      if ($1 == "Doc-tests") { id = "doc:" $2 }
      else if (match($0, /deps\/[^)]*\)/)) {
        id = substr($0, RSTART + 5, RLENGTH - 6); sub(/-[0-9a-f]+$/, "", id)
      }
      next
    }
    mode == "list" && /: test$/ { n = $0; sub(/: test$/, "", n); print id "::" n; next }
    mode == "run" {
      if (id ~ /^doc:/) { if (match($0, /^test .* \.\.\. /)) { n = substr($0, 6, RLENGTH - 10); print id "::" n } }
      else { r = $0; while (match(r, /test [^ ]+( - should panic)? \.\.\. /)) { n = substr(r, RSTART + 5, RLENGTH - 5); sub(/ .*$/, "", n); print id "::" n; r = substr(r, RSTART + RLENGTH) } }
    }' "$@" | sort
}

step_final_records() {
  local head ph rcf logf key val bad=0 shas="" logs=() total=0 cur
  head=$(git rev-parse HEAD)
  for ph in $TEST_PHASES; do
    rcf=$REC/$ph.$head.rc
    logf=$REC/$ph.$head.log
    if [ ! -f "$rcf" ] || [ ! -f "$logf" ]; then
      row "phase-$ph" FAIL "no record for HEAD ${head:0:12} (run --phase $ph)"; bad=1; continue
    fi
    local p_rc="" p_exec="" p_dirty="" p_sha="" p_head=""
    while IFS='=' read -r key val; do
      case "$key" in rc) p_rc=$val ;; executed) p_exec=$val ;; dirty) p_dirty=$val ;; binsha) p_sha=$val ;; head) p_head=$val ;; esac
    done <"$rcf"
    if [ "$p_head" != "$head" ]; then row "phase-$ph" FAIL "record is for ${p_head:0:12}, HEAD is ${head:0:12}"; bad=1; continue; fi
    if [ "$p_rc" != 0 ]; then row "phase-$ph" FAIL "record rc=$p_rc (see $logf)"; bad=1; continue; fi
    if [ "$p_dirty" != 0 ]; then row "phase-$ph" FAIL "record was taken on a dirty worktree"; bad=1; continue; fi
    row "phase-$ph" PASS "executed=$p_exec"
    shas="$shas $p_sha"
    logs+=("$logf")
    total=$((total + p_exec))
  done
  if [ -n "$(git status --porcelain)" ]; then row worktree-clean FAIL "worktree is dirty"; bad=1; else row worktree-clean PASS ""; fi
  cur=$(bin_sha)
  local s sha_bad=0
  for s in $shas; do [ "$s" = "$cur" ] || sha_bad=1; done
  if [ "$cur" = none ] || [ "$sha_bad" = 1 ]; then
    row binary-sha FAIL "target/release/fastroute (${cur:0:12}) differs from the recorded sha(s) or is missing"; bad=1
  else
    row binary-sha PASS "${cur:0:12}"
  fi
  if [ "${#logs[@]}" -eq 0 ]; then
    row cargo-test FAIL "no usable phase records"; return
  fi
  # Coverage: every test cargo lists for the workspace must have run in some phase.
  "$CARGO_SLOT" test --release --workspace -- --list >"$LOGDIR/list.all" 2>&1
  norm_tests list "$LOGDIR/list.all" >"$LOGDIR/list.names"
  norm_tests run "${logs[@]}" >"$LOGDIR/run.names"
  local missing nlist
  nlist=$(grep -c . "$LOGDIR/list.names" || true)
  missing=$(comm -23 "$LOGDIR/list.names" "$LOGDIR/run.names")
  if [ "$nlist" -eq 0 ]; then
    row coverage FAIL "cargo --list produced no test names ($LOGDIR/list.all)"; bad=1
  elif [ -n "$missing" ]; then
    row coverage FAIL "$(printf '%s\n' "$missing" | grep -c .) of $nlist listed test(s) never executed; first: $(printf '%s\n' "$missing" | head -1)"; bad=1
  else
    row coverage PASS "all $nlist listed tests executed by the phases"
  fi
  if [ "$bad" -eq 0 ]; then row cargo-test PASS "executed=$total tests (phases)"; else row cargo-test FAIL "phase records incomplete or stale"; fi
  skip_row parity-skips "${logs[@]}"
}

# --- dispatch ---------------------------------------------------------------------------------

case "$PHASE" in
  "")
    step_ledger
    step_tests all
    step_parity_route
    step_stock_pin
    step_conformance ;;
  tests-*)
    step_tests "$PHASE" ;;
  final)
    step_final_records
    step_ledger
    step_parity_route
    step_stock_pin
    step_conformance ;;
esac

echo "== pcbkit-gate${PHASE:+ --phase $PHASE} =="
printf '%s\n' "${ROWS[@]}"
echo "logs: $LOGDIR"
if [ "$FAIL" -eq 0 ]; then echo "GATE: PASS"; exit 0; fi
echo "GATE: FAIL"; exit 1
