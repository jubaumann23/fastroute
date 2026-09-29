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
| Greedy optimizer passes | Java evaluates every candidate but applies only the single best one per pass, so on larger boards a pass rarely clears the 2.5 % continuation threshold. After the winner, the other improving candidates are re-routed best-first on the current board and kept if the optimizer score improves without more unrouted connections or violations. |
| Optimizer re-routes only the ripped item's nets | Java's re-route runs full autorouter passes over every unrouted connection on the board for each candidate, retrying hopeless connections hundreds of times. |
| A failing connection rips its net only once | Java rips every unfixed trace/via of the net on each failure from the second one on. A connection that can never be routed (e.g. an enclosed GND pin) then tears down its whole net every other pass: lora oscillated between 12 and 19 unrouted. Unrouted after the change: bm05 10 → 7, complex_hierarchy 10 → 8, lora 12 → 11, StickHub 2 → 3. |
| Parallel multi-start routing (`--multi-start=N`, default 4) | If the autorouter leaves connections unrouted, it is rerun N−1 more times in parallel with shuffled (seeded) first-pass orders and the best board is kept (fewest unrouted, then violations, then router score). Boards that route completely pay nothing; others take about twice as long. Unrouted: bm04 1 → 0, bm05 7 → 2, CM5 8 → 7, StickHub 3 → 1. |
| Connections may end in a via into the net's plane | A found path that ends on the net's plane (an inactive power layer) is rejected as "some of their layers are disabled", so plane nets only reached their plane in the fanout stage and the rest stayed unrouted. On a 4-layer ESP32-S3 board with GND and 3V3 planes: 24 → 0 unrouted. |
| Unclamped optimizer score | The V2 score is clamped at 0; on bm01 the excess length/vias push it below 0, so no candidate could ever be accepted. The optimizer compares unclamped values (and the pass improvement relative to the magnitude). |

## KiCad plugin

| Change | Why |
|---|---|
| Board minimum track width → `min_trace_width_um` and `neck_width_um` | The minimum is not in the DSN export. Neck-down stays enabled (fine-pitch pads need it) but stops at the minimum; failed insertions are retried with minimum-width traces (Freerouting's necked retry, off by default there). lora: 49 → 19 unrouted. |
| Board copper-to-edge clearance → `copper_to_edge_clearance_um` | Not in the DSN export; Freerouting's default is 250 µm. |
| Zone handling (`strip_planes`) | KiCad exports each zone as a plane covering its outline, so pads inside count as connected even if the refilled zone cuts them off. Pours on signal layers are removed and their nets routed with tracks. A power-type layer with one plane covering ≥ 50 % of the board is kept as a plane layer (pads connect through vias; no tracks on it). Other power-type layers become signal layers — Freerouting never routes on power layers, which made the 2-layer complex_hierarchy demo effectively single-layer. |
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
