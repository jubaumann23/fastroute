#!/bin/sh
# Generates the Java ground truth of fr-io's parity tests with the source build
# reference/bin/freerouting-parity.jar.
#   run_java_dump.sh OUT_DIR BASE_DIR DSN...          board dumps (DumpBoard, tests/java_parity.rs)
#   run_java_dump.sh --ses OUT_DIR BASE_DIR ENTRY...  SES / post-load / session ground truth
#                                                     (SesHarness, tests/ses_parity.rs);
#                                                     ENTRY = DSN or DSN::SES
# Names are the DSN paths relative to BASE_DIR ('/' -> '__', without .dsn).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
JDK="$ROOT/reference/jdk25/bin"
JAR="$ROOT/reference/bin/freerouting-parity.jar"
BUILD="${TMPDIR:-/tmp}/fr-io-java-harness"
mkdir -p "$BUILD"
"$JDK/javac" -nowarn -cp "$JAR" -d "$BUILD" "$HERE/java/DumpBoard.java" "$HERE/java/SesHarness.java"
MAIN=DumpBoard
if [ "$1" = "--ses" ]; then
  MAIN=SesHarness
  shift
fi
exec "$JDK/java" -Xss64m -cp "$BUILD:$JAR" "$MAIN" "$@" 2>/dev/null
