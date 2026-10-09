#!/usr/bin/env bash
# pcbkit-rebase.sh: replay the pcbkit patch set onto another upstream tag as ONE net patch.
#
# Usage: pcbkit-rebase.sh [--quick | --full] [--resume] [--force] [--fetch] [--base TAG] [--src REF] <target-tag>
#   default   worktree + stock build + net patch + build + conformance t1/t2 + gate phase tests-core
#   --quick   same without tests-core (conformance only; a few minutes)
#   --full    every test phase and `final` of scripts/pcbkit-gate.sh (about 30 min; each phase is a
#             separate foreground call, so run it with --resume per phase set if your shell has a time limit)
#   --resume  reuse an existing rebase worktree (skip create, stock build and patch apply)
#   --force   remove an existing rebase worktree and its branch first (only ones this script made)
#   --fetch   run `git fetch upstream --tags` first (read-only network access, off by default)
#   --base    tag the patch set was made against (default v0.1.13)
#   --src     ref that holds the patch set (default pcbkit)
#   --self-check  test the report helper and exit
#
# Procedure (docs/PCBKIT-REBASE.md section 1):
#   1. worktree $PCBKIT_WT_ROOT/rebase-script-<tag> on branch pcbkit-rebase-script-<tag> at <tag>
#   2. build the STOCK binary of <tag> (conformance --stock-cli, STOCK_CLI_SHA256 re-pin)
#   3. `git diff BASE SRC | git apply --3way --index`; report per file and per PATCH LEDGER hook site
#   4. commit the patch (and the STOCK_CLI_SHA256 re-pin), build the shipped binary (no test-hooks)
#   5. router_conformance.py at threads 1 and 2, then the gate phases of the chosen mode
# It only reads pcbkit (SRC), writes only its own worktree + branch, and never pushes or fetches
# unless --fetch is given. Exit 0 only if every executed step passed; 1 on conflicts or failures; 2 usage.
# Env: PCBKIT_CONFORMANCE (required: path to the toolkit's router_conformance.py),
#      CARGO_SLOT (cargo wrapper, default plain `cargo`), PCBKIT_WT_ROOT (default <checkout>/../fastroute-wt),
#      PCBKIT_JAVA (passed through to the gate).
set -u
MODE=default RESUME=0 FORCE=0 FETCH=0 BASE=v0.1.13 SRC=pcbkit TARGET=""
HERE=$(cd "$(dirname "$0")" && pwd)
REPORT=$HERE/pcbkit/rebase_report.py
die() { echo "pcbkit-rebase: $*" >&2; exit 2; }
while [ $# -gt 0 ]; do
  case "$1" in
    --quick) MODE=quick ;; --full) MODE=full ;; --resume) RESUME=1 ;; --force) FORCE=1 ;; --fetch) FETCH=1 ;;
    --base) BASE=${2:-}; [ -n "$BASE" ] || die "--base needs a tag"; shift ;;
    --src) SRC=${2:-}; [ -n "$SRC" ] || die "--src needs a ref"; shift ;;
    --self-check) python3 "$REPORT" --self-check; exit $? ;;
    -h|--help) sed -n '2,26p' "$0"; exit 0 ;;
    -*) die "unknown option $1" ;;
    *) [ -z "$TARGET" ] || die "one target tag only"; TARGET=$1 ;;
  esac
  shift
done
[ -n "$TARGET" ] || die "usage: $0 [--quick|--full] [--resume] [--force] [--fetch] <target-tag>"
[ "$RESUME" = 1 ] && [ "$FORCE" = 1 ] && die "--resume and --force are exclusive"
[ -n "${PCBKIT_CONFORMANCE:-}" ] || die "PCBKIT_CONFORMANCE is required (path to router_conformance.py)"
[ -f "$PCBKIT_CONFORMANCE" ] || die "PCBKIT_CONFORMANCE not found: $PCBKIT_CONFORMANCE"
CARGO_SLOT=${CARGO_SLOT:-cargo}
case "$TARGET" in *[!A-Za-z0-9._-]*|-*) die "bad tag name '$TARGET'" ;; esac

MAIN=$(dirname "$(git -C "$HERE" rev-parse --path-format=absolute --git-common-dir)")
WT_ROOT=${PCBKIT_WT_ROOT:-$(dirname "$MAIN")/fastroute-wt}
WT=$WT_ROOT/rebase-script-$TARGET
BRANCH=pcbkit-rebase-script-$TARGET
cd "$MAIN" || exit 2
[ "$FETCH" = 1 ] && { git fetch upstream --tags >&2 || die "git fetch upstream --tags failed"; }
git rev-parse -q --verify "refs/tags/$TARGET^{commit}" >/dev/null || die "tag $TARGET not found locally (try --fetch)"
git rev-parse -q --verify "refs/tags/$BASE^{commit}" >/dev/null || die "base tag $BASE not found"
git rev-parse -q --verify "$SRC^{commit}" >/dev/null || die "source ref $SRC not found"
SRC_SHA=$(git rev-parse --short "$SRC")

ROWS=() FAIL=0
row() { ROWS+=("$(printf '%-24s %-5s %s' "$1" "$2" "$3")"); [ "$2" = FAIL ] && FAIL=1; return 0; }
finish() {
  echo; echo "== pcbkit-rebase $BASE+$SRC($SRC_SHA) -> $TARGET ($MODE) =="
  printf '%-24s %-5s %s\n' STEP RESULT DETAIL
  printf '%s\n' "${ROWS[@]}"
  echo "worktree: $WT (branch $BRANCH); $SRC and all remotes untouched"
  if [ "$FAIL" = 0 ]; then echo "REBASE: OK"; exit 0; else echo "REBASE: FAIL"; exit 1; fi
}

LEDGER=$(mktemp "${TMPDIR:-/tmp}/pcbkit-ledger.XXXXXX")
trap 'rm -f "$LEDGER"' EXIT
git show "$SRC:docs/PCBKIT.md" >"$LEDGER" || die "$SRC has no docs/PCBKIT.md"
STOCK=$WT/target/rebase/stock-$TARGET

# --- 1. worktree -------------------------------------------------------------------------------
if [ "$RESUME" = 1 ]; then
  [ -d "$WT" ] || die "--resume: no worktree $WT"
  row worktree PASS "reused $WT"
else
  if [ -e "$WT" ] || git rev-parse -q --verify "refs/heads/$BRANCH" >/dev/null; then
    [ "$FORCE" = 1 ] || die "$WT or branch $BRANCH exists (use --resume or --force)"
    git worktree remove --force "$WT" 2>/dev/null || rm -rf "$WT"
    git worktree prune
    git branch -D "$BRANCH" >/dev/null 2>&1
  fi
  git worktree add "$WT" -b "$BRANCH" "$TARGET" >/dev/null 2>&1 || die "git worktree add failed"
  row worktree PASS "$BRANCH at $TARGET ($(git rev-parse --short "$TARGET"))"
fi
cd "$WT" || exit 2
[ -e reference ] || { [ -d "$MAIN/reference" ] && ln -s "$MAIN/reference" reference; }

# --- 2. stock build + 3. net patch ---------------------------------------------------------------
if [ "$RESUME" = 0 ]; then
  mkdir -p target/rebase
  if "$CARGO_SLOT" build --release -p fastroute >target/rebase/stock-build.log 2>&1; then
    cp target/release/fastroute "$STOCK"
    row stock-build PASS "$($STOCK --version 2>&1 | head -1), sha256 $(sha256sum "$STOCK" | cut -c1-12)"
  else
    row stock-build FAIL "see $WT/target/rebase/stock-build.log"; finish
  fi

  # Upstream files that differ between the two tags AND carry a ledger hook: the only places a
  # human merge can ever be needed.
  echo "== upstream drift $BASE..$TARGET at hook files =="
  DRIFT=$(git diff --name-only "$BASE" "$TARGET" | tr '\n' ' ')
  # shellcheck disable=SC2086
  python3 "$REPORT" --ledger "$LEDGER" --files $DRIFT | awk -F'\t' '$4 !~ /^NOT A LEDGER/ {print "  " $1 "  -> " $4}'
  NDRIFT=$(echo "$DRIFT" | wc -w)

  git diff "$BASE" "$SRC" >target/rebase/net.patch
  NPATCH=$(grep -c '^diff --git' target/rebase/net.patch)
  git apply --3way --index target/rebase/net.patch 2>target/rebase/apply.log
  APPLY_RC=$?
  UNMERGED=$(git diff --name-only --diff-filter=U)
  echo "== net patch: $NPATCH files, apply rc=$APPLY_RC =="
  python3 "$REPORT" --ledger "$LEDGER" --apply-log target/rebase/apply.log | sed 's/^/  /'
  if [ -n "$UNMERGED" ]; then
    echo "== conflicts per file and hook site (file, hunks, line spans, ledger hooks) =="
    # shellcheck disable=SC2086
    python3 "$REPORT" --ledger "$LEDGER" --files $UNMERGED | awk -F'\t' '{printf "  %s\n    hunks=%s spans=%s\n    hook site: %s\n", $1, $2, $3, $4}'
    row net-patch FAIL "$(echo "$UNMERGED" | wc -l) of $NPATCH files conflict; upstream drift $NDRIFT files; resolve in $WT"
    finish
  elif [ "$APPLY_RC" != 0 ]; then
    cat target/rebase/apply.log >&2
    row net-patch FAIL "git apply failed without conflict markers (log: $WT/target/rebase/apply.log)"; finish
  fi
  row net-patch PASS "$NPATCH files, 0 conflicts; upstream drift $NDRIFT files"
  git -c user.name="pcbkit-rebase" -c user.email="pcbkit-rebase@localhost" commit -q \
    -m "feat(pcbkit): patch set $BASE..$SRC_SHA on $TARGET" -m "Net patch applied by scripts/pcbkit-rebase.sh." || { row commit FAIL "commit failed"; finish; }
  # Re-pin the stock binary hash for the new base so the gate's stock-pin step compares against $TARGET.
  NEWSHA=$(sha256sum "$STOCK" | cut -d' ' -f1)
  if grep -q '^STOCK_CLI_SHA256:' docs/PCBKIT.md; then
    sed -i "s/^STOCK_CLI_SHA256: .*/STOCK_CLI_SHA256: $NEWSHA/" docs/PCBKIT.md
    git -c user.name="pcbkit-rebase" -c user.email="pcbkit-rebase@localhost" commit -q -am "chore(pcbkit): re-pin STOCK_CLI_SHA256 for $TARGET" || { row repin FAIL "commit failed"; finish; }
    row repin PASS "STOCK_CLI_SHA256 -> ${NEWSHA:0:12}"
  else
    row repin FAIL "no STOCK_CLI_SHA256 line in docs/PCBKIT.md"; finish
  fi
fi
[ -x "$STOCK" ] || { row stock-bin FAIL "missing $STOCK (run without --resume first)"; finish; }

# --- 4. build the shipped binary ---------------------------------------------------------------
PLAIN=$WT/target/pcbkit-plain/release/fastroute
if CARGO_TARGET_DIR=$WT/target/pcbkit-plain "$CARGO_SLOT" build --release -p fastroute >target/rebase/build.log 2>&1; then
  row build PASS "$(basename "$PLAIN") with serve, no test-hooks"
else
  row build FAIL "see $WT/target/rebase/build.log"; finish
fi

# --- 5. conformance + gate ---------------------------------------------------------------------
EXTRA=()
python3 "$PCBKIT_CONFORMANCE" --help 2>&1 | grep -q -- '--stock-cli' && EXTRA=(--stock-cli "$STOCK")
for th in 1 2; do
  if python3 "$PCBKIT_CONFORMANCE" --server "$PLAIN serve" --threads "$th" "${EXTRA[@]}" >"target/rebase/conf-$th.log" 2>&1; then
    row "conformance-t$th" PASS "$(tail -n1 "target/rebase/conf-$th.log" | cut -c1-70)"
  else
    row "conformance-t$th" FAIL "see $WT/target/rebase/conf-$th.log"
  fi
done

run_gate() { # phase...
  local ph
  for ph in "$@"; do
    if CARGO_SLOT=$CARGO_SLOT PCBKIT_BASE=$TARGET PCBKIT_STOCK_CLI=$STOCK scripts/pcbkit-gate.sh --phase "$ph" >"target/rebase/gate-$ph.log" 2>&1; then
      row "gate-$ph" PASS "$(grep -E 'PASS' "target/rebase/gate-$ph.log" | tail -n1 | cut -c1-60)"
    else
      row "gate-$ph" FAIL "see $WT/target/rebase/gate-$ph.log"
    fi
  done
}
case "$MODE" in
  quick) ;;
  default) run_gate tests-core ;;
  full) run_gate tests-core tests-serve-a tests-serve-b tests-serve-c tests-serve-d tests-serve-e tests-serve-f final ;;
esac
finish
