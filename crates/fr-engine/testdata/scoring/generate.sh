#!/bin/sh
# Regenerates the DRC/scoring vectors (porting unit U6) with ScoreGen.java and the parity jar.
# Run from the workspace root. Replayed by `cargo test -p fr-engine --test board_replay scoring`.
# Larger boards (not committed) are listed at the end; check them with
#   FR_SCORING_VECTORS=<file>[:<file>...] cargo test --release -p fr-engine --test board_replay scoring
set -e
J=reference/jdk25/bin
D=crates/fr-engine/testdata
F=reference/freerouting/fixtures
P=reference/parity-baseline
T=${TMPDIR:-/tmp}/scoregen
mkdir -p "$T"
$J/javac -cp reference/bin/freerouting-parity.jar -d "$T" $D/board/BoardGen.java $D/scoring/ScoreGen.java
run() { out=$1; shift; $J/java -cp reference/bin/freerouting-parity.jar:"$T" ScoreGen "$@" > "$out"; }

# boards routed by the parity jar (reference/parity-baseline) and fixtures with wiring
for b in bm02 bm04 bm05 bm06 bm07 bm09 bm11; do
  run $D/scoring/dac_$b.txt $F/Issue508-DAC2020/DAC2020_$b/DAC2020_$b.unrouted.dsn $P/DAC2020_$b.ses 11
done
run $D/scoring/dac_bm08.txt $F/Issue508-DAC2020_bm08.dsn $P/DAC2020_bm08.ses 1
run $D/scoring/issue313.txt $F/Issue313-FastTest.dsn $F/Issue313-FastTest.ses 2
run $D/scoring/issue026.txt $F/Issue026-J2_reference.dsn $F/Issue026-J2_reference.ses 3
run $D/scoring/issue690.txt $F/Issue690-ecc83.dsn $F/Issue690-ecc83.ses 10
run $D/scoring/issue742.txt $F/Issue742-tastexx-pcb.dsn $F/Issue742-tastexx-pcb.ses 12
run $D/scoring/issue555.txt $F/Issue555-BBD_Mars-64.dsn $F/Issue555-BBD_Mars-64-current.ses 13
# boards with pre-existing clearance violations, clearance compensation, zero tolerance
run $D/scoring/issue575.txt "$F/Issue575-drc_BBD_Mars-64_6_track_1_hole_clearance_violations.dsn" - 4
run $D/scoring/issue413_comp.txt $F/Issue413-test.dsn - 7 c
run $D/scoring/issue110_tol0.txt $F/Issue110-testPCBSpecctraFile.dsn - 8 tol=0
# unrouted
run $D/scoring/issue229.txt $F/Issue229-display-8-digit-hc595.dsn - 6
run $D/scoring/issue143.txt $F/Issue143-rpi_splitter.dsn - 9
run $D/scoring/scoring_mixed.txt $F/scoring-mixed-layer.dsn - 14

# not committed (size):
#   run $T/dac_bm01.txt $F/Issue508-DAC2020/DAC2020_bm01/DAC2020_bm01.unrouted.dsn $P/DAC2020_bm01.ses 11
#   run $T/dac_bm10.txt $F/Issue508-DAC2020/DAC2020_bm10/DAC2020_bm10.unrouted.dsn $P/DAC2020_bm10.ses 11
#   run $T/issue103_routed.txt $F/Issue103-Board-Routed.dsn - 5
#   run $T/cm5.txt reference/freerouting/scripts/benchmark/fixtures/KiCad_10_demos/CM5_MINIMA_3.dsn $P/CM5_MINIMA_3.ses 15
