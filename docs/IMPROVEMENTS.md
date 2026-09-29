# Improvements over Freerouting

fastroute started as a faithful port (`--parity` reproduces Freerouting's
deterministic results byte for byte). Changes that alter results are listed
here. The engine-side ones are active unless `--parity` is given
(`PipelineContext::enhancements`); the KiCad-side ones live in the plugin
(`integrations/kicad/plugins/core.py`).

## Engine

| Change | Why |
|---|---|
| Final tail removal also after the autorouter stopped itself | Freerouting skips `removeTails` when the stagnation rule stops the router, leaving unused fanout escapes (via + stub) on the board. KiCad reports them as dangling vias (50 on the lora board). |
| `router.min_trace_width_um` (option, unset by default) | Neck-down (at pins, fanout "micro neck-down", necked retry) never goes below this width; narrower candidates are raised to it. |
| The autorouter's own stop (stagnation, max passes) is withdrawn before the optimizer | Java keeps the stop request set, so every optimizer candidate fails to route and the optimizer gives up after its failure limit. |
| Optimizer also runs on partly routed boards | Java skips it whenever a connection is unrouted; the acceptance rules already reject candidates that add unrouted connections. |
| Greedy optimizer passes | Java evaluates every candidate but applies only the single best one per pass, so on larger boards a pass rarely clears the 2.5 % continuation threshold. After the winner, the other improving candidates are re-routed best-first on the current board and kept if the optimizer score improves without more unrouted connections or violations. This phase runs on one thread and clones the board per candidate, so it is limited to the pass's evaluation time (2–60 s) and ends after 10 rejected candidates in a row; unlimited, a 874-part 6-layer board spent ~7 min per pass in it (1258 candidates) and looked hung. |
| Optimizer re-routes only the ripped item's nets | Java's re-route runs full autorouter passes over every unrouted connection on the board for each candidate, retrying hopeless connections hundreds of times. |
| A failing connection rips its net only once | Java rips every unfixed trace/via of the net on each failure from the second one on. A connection that can never be routed (e.g. an enclosed GND pin) then tears down its whole net every other pass: lora oscillated between 12 and 19 unrouted. Unrouted after the change: bm05 10 → 7, complex_hierarchy 10 → 8, lora 12 → 11, StickHub 2 → 3. |
| Parallel multi-start routing (`--multi-start=N`, default 4) | If the autorouter leaves connections unrouted, it is rerun N−1 more times in parallel with shuffled (seeded) first-pass orders and the best board is kept (fewest unrouted, then violations, then router score). Boards that route completely pay nothing; others take about twice as long. Unrouted: bm04 1 → 0, bm05 7 → 2, CM5 8 → 7, StickHub 3 → 1. |
| Connections may end in a via into the net's plane | A found path that ends on the net's plane (an inactive power layer) is rejected as "some of their layers are disabled", so plane nets only reached their plane in the fanout stage and the rest stayed unrouted. On a 4-layer ESP32-S3 board with GND and 3V3 planes: 24 → 0 unrouted. |
| Parallel autorouting pass (`router.autorouter.max_threads`) | Freerouting routes the connections of a pass one after the other. fastroute routes up to 2 connections per thread at a time, each on a clone of the board as it was when the connection was started (preferring connections away from the ones in flight, never two of one net), and commits the results in start order by copying the changed traces/vias onto the board. A result is only copied if the items it replaced are unchanged and the copies have no new clearance violations; otherwise the connection is retried (twice) and then routed sequentially. Commit order and start decisions only depend on the commits, so results do not depend on thread timing (they do depend on the thread count). 6-layer test board (874 parts, 8 classes, planes): routing stage 1413 s → 508 s with 8 threads, 165 → 153 unrouted; failing connections (most of the later passes) run fully in parallel (pass time 163 s → 37 s). |
| Undo a bad autorouting pass | A pass that ends with far more unrouted items than the best pass so far (> best + max(20, 30 %)) is undone and the next pass starts from the best board (at most 3 times). Java keeps routing from the worse board: a 6-layer test board went from 291 to 552 unrouted in pass 2 when a failing connection ripped whole nets. |
| Slow-pass stagnation stop | When a pass takes 20 s or more, the autorouter stops once the last 3 passes reduced the unrouted items by less than 2 % (at least one). Java's rules only start after 8 passes, which is 80+ minutes at 10 minutes per pass. |
| Multi-start skipped after long runs | The parallel variants each take as long as the first run; after a first run longer than 10 minutes they are skipped. Variants also stop on a job stop (time limit, Ctrl+C) now. |
| No fanout on ignored net classes | Pins of classes in `router.autorouter.ignore_net_classes` get no fanout stubs (Java fans them out, so nets that must stay unrouted got 72 stubs on the test board). |
| Fanout stops when it only re-fans | A fanout pass that only fans out pins the previous pass fanned out as well (ripped again in between) ends the fanout stage; Java needed 3 identical passes. |
| Unclamped optimizer score | The V2 score is clamped at 0; on bm01 the excess length/vias push it below 0, so no candidate could ever be accepted. The optimizer compares unclamped values (and the pass improvement relative to the magnitude). |

## Length matching (`--tune`, new)

Freerouting has no length matching. After routing and optimizing, fastroute can match groups
of nets (e.g. a memory bus): the target is the longest net of a group (or a given length);
shorter nets get serpentine meanders on their longest straight segments (both sides or
alternating, amplitudes 2 mm down to 0.45 mm, leg pitch 3× the width where it fits, else
width + clearance). Every meander is made on a board clone and kept only if a slightly wider
probe trace (+5 µm) has no clearance violation, so the result stays DRC-clean in KiCad too.
Lengths count traces only (vias are not in the DSN's geometry). Nets that have no room left
are reported with the missing length.

The KiCad plugin writes the groups from the board's `.kicad_dru`: `skew` constraints (group
matched to its longest net) and `length (min …)` constraints, for conditions on
`A.NetClass == '…'` / `A.NetName == '…'` (wildcards) joined by `&&` / `||`.

## KiCad plugin

| Change | Why |
|---|---|
| Board minimum track width → `min_trace_width_um` and `neck_width_um` | The minimum is not in the DSN export. Neck-down stays enabled (fine-pitch pads need it) but stops at the minimum; failed insertions are retried with minimum-width traces (Freerouting's necked retry, off by default there). lora: 49 → 19 unrouted. |
| Board copper-to-edge clearance → `copper_to_edge_clearance_um` | Not in the DSN export; Freerouting's default is 250 µm. |
| Zone handling (`strip_planes`) | KiCad exports each zone as a plane covering its outline, so pads inside count as connected even if the refilled zone cuts them off. Pours on signal layers are removed and their nets routed with tracks. A power-type layer with one plane covering ≥ 50 % of the board is kept as a plane layer (pads connect through vias; no tracks on it). Other power-type layers become signal layers — Freerouting never routes on power layers, which made the 2-layer complex_hierarchy demo effectively single-layer. |
| `.kicad_dru` net-class clearances → DSN `class_class` rules | KiCad's DSN export has one clearance per class; custom rules are lost. Rules whose condition only compares net classes (`A.NetClass == 'X' && B.NetClass != 'Y' ...`) with a minimum clearance constraint are written as class-pair clearances. |
| Copper texts → keepouts (glyph convex hulls) | Omitted by KiCad's DSN export. |
| Rule-area keepouts corrected | KiCad exports every rule area as a keepout, even areas that forbid nothing (KiCad 10's multichannel "auto-placement-area" regions). The router then blocked whole channels: multichannel_mixer went from 160 unrouted (Freerouting) to 0. Areas forbidding neither tracks nor vias are dropped; via-only areas become via keepouts. |
| Zone refill after import | Stale fills overlapped the new tracks. |
| "Remove existing tracks" drops unlocked wires/vias from the DSN | Deleting board items inside the action plugin crashed pcbnew (undo snapshot with freed items); locked (`type fix`) tracks are kept. |

## Results (KiCad 10 DRC after re-routing from scratch)

Unconnected items / routing-related violations; see `integrations/kicad/tests/e2e.sh`.

| Board | Original design | Plain Freerouting flow | fastroute plugin |
|---|---|---|---|
| lora_node (4 layers, 445 SMD pads) | 0 / 0 | — | 1 / 0 (53 s incl. optimizer) |
| multichannel_mixer | 0 / 0 | 160 unrouted in the benchmark | 0 / 0 (3.3 s) |
| complex_hierarchy | 0 / 1 | 83 / 3 | 0 / 0 (0.7 s) |
| interf_u | 0 / 3 | 8 / 346 | 0 / 2 (5.6 s) |
| pic_programmer | 0 / 8 | 0 / 118 | 0 / 0 (0.7 s) |
| ecc83-pp | 0 / 0 | 0 / 6 | 0 / 0 (0.3 s) |
| sonde xilinx | 0 / 38 | 1 / 122 | 0 / 36 (0.4 s) |

Remaining violations exist in the original designs as well (pads close to the
board edge, thermal spokes). The lora board's last unconnected item is a GND
zone-to-zone link.

## Optimizer results (benchmark boards, default parallel mode)

`scripts/ab-enhancements.sh` runs each board with and without the improvements
(`--no-enhancements`). Unrouted connections, Freerouting behaviour → fastroute:
bm04 1 → 0, bm05 11 → 2, CM5 8 → 7, StickHub 2 → 1, complex_hierarchy 10 → 8,
all other boards unchanged (no board got worse). Optimizer scores:

| Board | Freerouting behaviour | fastroute |
|---|---|---|
| DAC2020_bm06 | 617 | 718 |
| DAC2020_bm07 | 657 | 752 |
| DAC2020_bm10 | 641 | 711 |
| DAC2020_bm11 | 768 | 859 |
| interf_u | 585 | 619 |
| sonde xilinx | 877 | 928 |
| bm04 / bm05 / CM5 / StickHub (partly routed before) | optimizer skipped | 955 / 569 / 905 / 822 (bm05 now has 2 instead of 11 unrouted, so the optimizer works on a different board) |
| bm01 (score below 0) | no improvement possible (91 s) | −182 → 107 in 4 passes (69 s) |
| lora_node | 328 (+0.45 %) | 435 (+33 %) |

## Open ideas

- CLI users feeding KiCad DSN files directly get none of the plugin's export
  fixes (rule-area keepouts, zone handling, board constraints). A `--kicad`
  preprocessing mode could apply the DSN-only parts (rule areas cannot be
  recognised without the board, though).
- Connections that can never be routed (e.g. an enclosed pin) are retried every
  pass; they could be skipped after the net was ripped once.
- Footprint-local clearances (e.g. a 0.5 mm-pitch LGA with 0.127 mm) are not in
  the DSN export; the router uses the net-class clearance there.
