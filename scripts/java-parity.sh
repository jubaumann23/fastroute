#!/bin/sh
# Runs the deterministic parity build of freerouting (built from the exact
# reference commit with docs/parity/TimeLimit.patch applied) on one DSN.
# Deterministic = time limits disabled + single-threaded optimizer.
# Usage: scripts/java-parity.sh board.dsn out.ses [extra freerouting args]
# Stderr lines "PARITY_TIME_LIMIT" mark places where a wall-clock limit
# would have changed the result in a normal run.
#
# Building the jar (once):
#   cp -R reference/freerouting reference/freerouting-parity
#   (cd reference/freerouting-parity && patch -p1 < ../../docs/parity/TimeLimit.patch)
#   (cd reference/freerouting-parity && JAVA_HOME=../jdk25 ./gradlew -q executableJar \
#      -x test -x spotlessCheck -x checkstyleMain)
#   cp reference/freerouting-parity/build/libs/freerouting-current-executable.jar \
#      reference/bin/freerouting-parity.jar
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
DSN=$1; SES=$2; shift 2
exec "$ROOT/reference/jdk25/bin/java" -Dfreerouting.parity.disableTimeLimits=true \
  -jar "$ROOT/reference/bin/freerouting-parity.jar" --gui.enabled=false \
  --router.optimizer.max_threads=1 -de "$DSN" -do "$SES" "$@"
