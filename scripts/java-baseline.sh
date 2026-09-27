#!/bin/sh
# Runs the Java freerouting jar on the benchmark fixtures and records wall time,
# peak RSS and the final score line. Usage: scripts/java-baseline.sh [outdir]
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
REF="$ROOT/reference"
OUT=${1:-"$REF/baseline"}
JAVA="$REF/jdk25/bin/java"
JAR="$REF/bin/freerouting-2.4.1.jar"
mkdir -p "$OUT"
SUMMARY="$OUT/summary.tsv"
printf "board\treal_s\tmax_rss_mb\tresult\n" > "$SUMMARY"
for dsn in "$REF"/freerouting/scripts/benchmark/fixtures/DAC2020_boards/*.dsn \
           "$REF"/freerouting/scripts/benchmark/fixtures/KiCad_10_demos/*.dsn; do
  name=$(basename "$dsn" .dsn)
  log="$OUT/$name.log"
  /usr/bin/time -l "$JAVA" -jar "$JAR" --gui.enabled=false -de "$dsn" -do "$OUT/$name.ses" > "$log" 2>&1
  real=$(awk '/ real /{print $1}' "$log")
  rss=$(awk '/maximum resident set size/{printf "%d", $1/1048576}' "$log")
  result=$(grep "Optimization stage completed" "$log" | sed 's/.*final score: //; s/, using.*//')
  [ -z "$result" ] && result=$(grep "Auto-routing stage completed" "$log" | sed 's/.*final score: //; s/, using.*//')
  printf "%s\t%s\t%s\t%s\n" "$name" "$real" "$rss" "$result" >> "$SUMMARY"
done
cat "$SUMMARY"
