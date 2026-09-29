#!/bin/sh
# Cross-compiles fastroute for Linux x64 and arm64 with cargo-zigbuild, linked against
# glibc 2.17 so that the binaries run on practically every distribution since 2014
# (`cargo install cargo-zigbuild`, `brew install zig`,
#  `rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu`).
#
# Usage: scripts/build-linux.sh   -> dist/linux-x64/fastroute, dist/linux-arm64/fastroute
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
for pair in x86_64-unknown-linux-gnu:linux-x64 aarch64-unknown-linux-gnu:linux-arm64; do
  triple=${pair%:*}; tag=${pair#*:}
  (cd "$ROOT" && CARGO_PROFILE_RELEASE_STRIP=symbols cargo zigbuild --release -p fastroute --target "$triple.2.17")
  mkdir -p "$ROOT/dist/$tag"
  cp "$TARGET/$triple/release/fastroute" "$ROOT/dist/$tag/fastroute"
  echo "wrote dist/$tag/fastroute ($(du -h "$ROOT/dist/$tag/fastroute" | cut -f1))"
done
