#!/bin/sh
# Cross-compiles fastroute.exe for Windows x64 (MinGW-w64 toolchain, e.g.
# `brew install mingw-w64` + `rustup target add x86_64-pc-windows-gnu`).
# The binary only depends on system DLLs (UCRT, Windows 10+).
#
# Usage: scripts/build-windows.sh   -> dist/windows/fastroute.exe
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}
(cd "$ROOT" && cargo build --release -p fastroute --target x86_64-pc-windows-gnu)
mkdir -p "$ROOT/dist/windows"
x86_64-w64-mingw32-strip -o "$ROOT/dist/windows/fastroute.exe" "$TARGET/x86_64-pc-windows-gnu/release/fastroute.exe"
echo "wrote dist/windows/fastroute.exe ($(du -h "$ROOT/dist/windows/fastroute.exe" | cut -f1))"
