#!/bin/sh
# Profile-guided optimized release build of fastroute.
#
#   scripts/build-pgo.sh [--native]
#
# 1. builds an instrumented binary (target dir $PGO_DIR/gen),
# 2. runs it on a few benchmark boards (training set below),
# 3. merges the profiles with llvm-profdata and builds the optimized binary
#    ($PGO_DIR/use/release/fastroute).
# --native additionally passes `-C target-cpu=native` (the binary then only runs on CPUs like
# the build machine). Needs `rustup component add llvm-tools-preview` and the benchmark
# fixtures in reference/freerouting. PGO does not change results (same code, different
# optimization decisions; float semantics are unaffected).
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
PGO_DIR=${PGO_DIR:-$ROOT/target/pgo}
FX=$ROOT/reference/freerouting/scripts/benchmark/fixtures
EXTRA=""
if [ "${1:-}" = "--native" ]; then EXTRA="-C target-cpu=native"; fi
PROFDATA=$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-profdata
[ -x "$PROFDATA" ] || { echo "llvm-profdata not found: rustup component add llvm-tools-preview" >&2; exit 1; }

rm -rf "$PGO_DIR/profiles"; mkdir -p "$PGO_DIR/profiles"
echo "== instrumented build"
RUSTFLAGS="-C profile-generate=$PGO_DIR/profiles $EXTRA" CARGO_TARGET_DIR="$PGO_DIR/gen" \
  cargo build --release -p fastroute --manifest-path "$ROOT/Cargo.toml"
BIN="$PGO_DIR/gen/release/fastroute"
OUT=$(mktemp -d)
train() { echo "== training: $*"; "$BIN" -do "$OUT/x.ses" --parity "$@" > /dev/null 2>&1; }
train -de "$FX/DAC2020_boards/DAC2020_bm06.dsn"
train -de "$FX/DAC2020_boards/DAC2020_bm11.dsn"
train -de "$FX/DAC2020_boards/DAC2020_bm11.dsn" --optimizer-mode=parallel --router.optimizer.max_threads=4
train -de "$FX/KiCad_10_demos/interf_u.dsn" --router.optimizer.enabled=false
train -de "$FX/KiCad_10_demos/CM5_MINIMA_3.dsn" --router.optimizer.enabled=false -mp 1
train -de "$FX/KiCad_10_demos/multichannel_mixer.dsn" --router.optimizer.enabled=false
rm -rf "$OUT"
"$PROFDATA" merge -o "$PGO_DIR/merged.profdata" "$PGO_DIR/profiles"
echo "== optimized build"
RUSTFLAGS="-C profile-use=$PGO_DIR/merged.profdata -Cllvm-args=-pgo-warn-missing-function=false $EXTRA" \
  CARGO_TARGET_DIR="$PGO_DIR/use" cargo build --release -p fastroute --manifest-path "$ROOT/Cargo.toml"
echo "built $PGO_DIR/use/release/fastroute"
