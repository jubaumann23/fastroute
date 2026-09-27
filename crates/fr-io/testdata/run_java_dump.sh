#!/bin/sh
# Generates the Java ground-truth dumps of fr-io's parity test.
#   run_java_dump.sh OUT_DIR BASE_DIR DSN...
# Dump names are the DSN paths relative to BASE_DIR ('/' -> '__', without .dsn).
# Uses the JDK and jar under reference/ (see docs/PORTING.md).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
JDK="$ROOT/reference/jdk25/bin"
JAR="$ROOT/reference/bin/freerouting-parity.jar"
BUILD="${TMPDIR:-/tmp}/fr-io-java-harness"
mkdir -p "$BUILD"
"$JDK/javac" -nowarn -cp "$JAR" -d "$BUILD" "$HERE/java/DumpBoard.java"
exec "$JDK/java" -Xss64m -cp "$BUILD:$JAR" DumpBoard "$@" 2>/dev/null
