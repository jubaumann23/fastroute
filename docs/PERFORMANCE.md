# Performance work (result-neutral)

Goal: make `fastroute` faster **without changing any result**. Every step below keeps the SES
output byte-identical to the Java parity build (`fastroute --parity`) and keeps the parallel
optimizer mode deterministic and identical to its previous output.

## Methodology

* Machine: M-series Mac, 10 cores. Release build (`lto = "thin"`, `codegen-units = 1`).
* Profiling: `samply record --save-only --unstable-presymbolicate -o prof.json.gz -- <cmd>`,
  then `python3 scripts/prof-top.py prof.json.gz 40` (self + inclusive).
* Timings in the step log: single runs, wall clock (`/usr/bin/time`), machine otherwise idle
  unless noted; per-step numbers are indicative (±2 %). The final table is a serial run of all
  configurations with the pre-change binary and the final binary back to back.
* Parity gate after every step:
  * `cargo test --release --workspace` (includes `crates/fastroute/tests/pipeline_parity.rs`
    and the Java replay vector tests);
  * full pipeline `--parity` on bm02, bm06, bm07, bm08, bm09, bm11, pic_programmer, ecc83-pp,
    sonde xilinx, interf_u against `reference/parity-baseline/<board>.ses`;
  * autorouter only (`--router.optimizer.enabled=false`) on CM5_MINIMA_3 (`-mp 3`) and bm04
    against the pre-change binary, which was verified to be identical to the Java parity build
    for exactly these flags;
  * parallel optimizer (`--optimizer-mode=parallel --router.optimizer.max_threads=1` and `=8`)
    on bm11 and pic_programmer: 1 and 8 threads identical, and identical to the pre-change
    output;
  * the final binary additionally reproduces `reference/parity-baseline` on all 20 boards.
* `FASTROUTE_VERIFY_CACHES=1` (new): every memoized / indexed result is recomputed the old way
  and compared (contacts cache, per-net incomplete counts, indexed overlap queries against the
  Java traversal, leaf masks against the current shape layers). The whole gate passes in this
  mode.

## Optimizations (in the order applied)

| # | Change | Where | Measured effect |
|---|---|---|---|
| 1 | `Simplex` intersection tests without building the intersection: sort line *indices* with the ported TimSort and run the (unchanged) redundant-line algorithm on stack buffers; no `Line` clones (each clone went through `OnceLock` initialization) | `fr-geom/simplex.rs` (`redundant_lines_core`, `intersection_nonempty`), `java_sort::sort_indices_by` | CM5 AR `-mp 3`: 30.7 s → 25.1 s |
| 2 | Canonical sort of `overlaps` results by a precomputed `u64` key (kind, descending id, shape index) with `sort_unstable`; falls back to the comparator sort in traversal order if ids span ≥ 2^31, a shape index is negative or keys are not unique | `search_tree.rs` `sort_leaves_canonical` | CM5: 25.1 → 22.7 s |
| 3 | Memoized normal contacts: per-thread cache keyed by content epochs of the item repository and the default tree's item entries (epochs are process-wide unique numbers renewed by every `&mut` access, so clones share them only while their content is equal) | `board/epoch.rs`, `connectivity.rs`, `item_list.rs`, `search_tree.rs` | CM5: 22.7 → 13.7 s (fanout 18.9 s → 1.8 s) |
| 4 | Secondary spatial index for the sorted `overlaps` queries: hierarchical uniform grid over the leaf x/y ranges (flat arrays, cheap to clone), maintained on leaf insert/remove; the exact same leaf test is applied | `datastructures/leaf_grid.rs`, `MinAreaTree::overlapping_leaves_indexed` | CM5: 13.7 → 13.4 s, interf_u AR 8.9 → 8.2 s, bm04 AR 43.5 → 41.5 s |
| 5 | Layer masks in the search tree: each leaf carries the bit of its shape layer, inner nodes the union; queries that drop other layers (overlap queries with `layer >= 0`, the three `completeShape` variants) skip subtrees without leaves on that layer. The 45/90 degree `completeShape` walks see exactly the same sequence of relevant leaves (skipped leaves are ignored by the loop and never change the query) | `min_area_tree.rs` (`mask`, `next_leaf_masked`), `search_tree.rs`, `complete_shape.rs` | CM5: 13.4 → 10.5 s (only 7–17 % of the leaves visited by `completeShape` are on the room layer) |
| 6 | Shove checks compute only the needed tree shape of a substitute trace instead of all of them per index (was O(n²)); `IntOctagon::to_simplex` without the intermediate simplex, plus a small per-thread memo (Java memorizes it in the octagon) | `trace_shover.rs`, `int_octagon.rs` | bm04 AR: ~33 → 27.8 s |
| 7 | `Line` direction cache in atomics instead of `OnceLock` (cheap init and clone) | `fr-geom/line.rs` | ~1 %, removes `Once::call` (3–5 %) from profiles |
| 8 | One grid per layer (`LayeredGrid`) | `leaf_grid.rs` | bm04 `-mp 3`: 10.35 → 9.80 s user, CM5 `-mp 3`: 9.64 → 9.21 s |
| 9 | Compact structure-of-arrays copy of the node data for the traversals, specialized box/octagon tests (trees are homogeneous) | `min_area_tree.rs` (`HotNode`, `walk`) | ~1 % |
| 10 | mimalloc as global allocator of the CLI | `fastroute` | interf_u full: 17.3 → 15.7 s, bm04 full: 25.5 → 24.1 s |
| 11 | Per-net memo of the incomplete counts used by the pipeline (`calculateIncompleteCount`, called before and after every optimizer candidate), keyed by the keys and content versions of the net's items | `pipeline/stats.rs`, `item_list.rs` (`version`) | `NetIncompletes` 8.5 % → 4 % of bm11 full; ~1.5 % wall |
| 12 | `Simplex ∩ IntOctagon` uses the memoized octagon simplex; small per-thread memo for `Simplex::enlarge` (keyed by line-array identity, the memo holds the `Arc`) | `simplex.rs` | ~2 % |

Tried and rejected / not done:
* Single uniform grid with a "big leaves" list: slower than the tree (planes decompose into
  many large tiles) → replaced by the hierarchical grid.
* Replacing the maze `TreeSet` by a heap: the tie-break of the Java comparator uses door ids
  that change while elements are queued (incomplete rooms become complete), so the Java tree
  order cannot be reproduced by a heap.
* Memoizing shove / forced-via checks: they call count-mode `TimeLimit`s, whose call counts
  decide results in parity mode.

## Results

Serial runs, pre-change binary vs final binary (wall clock seconds, peak RSS). `full` =
`fastroute --parity` (fanout, autorouter, java-compat optimizer); `ar` = autorouter only
(`--router.optimizer.enabled=false`, CM5 with `-mp 3`); `parN` = `--parity
--optimizer-mode=parallel --router.optimizer.max_threads=N`. Java parity = the Java parity build
(`reference/parity-baseline/summary.tsv`, same machine, JVM start-up included). All 20 full
runs reproduce `reference/parity-baseline/<board>.ses` byte for byte; the `ar`/`par` runs are
identical to the pre-change binary (and `par1` == `par8`).

| config | board | Java parity (s) | before (s) | after (s) | speed-up | RSS before/after (MB) | SES |
|---|---|---|---|---|---|---|---|
| full | DAC2020_bm01 | 959.59 | 687.49 | 458.88 | 1.50x | 366/378 | identical to Java |
| full | DAC2020_bm02 | 7.21 | 0.88 | 0.64 | 1.38x | 17/22 | identical to Java |
| full | DAC2020_bm04 | 137.82 | 49.22 | 25.59 | 1.92x | 552/342 | identical to Java |
| full | DAC2020_bm05 | 36.90 | 20.74 | 15.16 | 1.37x | 54/66 | identical to Java |
| full | DAC2020_bm06 | 28.31 | 9.62 | 6.35 | 1.51x | 26/32 | identical to Java |
| full | DAC2020_bm07 | 15.70 | 3.47 | 2.40 | 1.45x | 21/28 | identical to Java |
| full | DAC2020_bm08 | 3.58 | 0.14 | 0.09 | 1.56x | 7/8 | identical to Java |
| full | DAC2020_bm09 | 6.13 | 0.86 | 0.45 | 1.91x | 18/21 | identical to Java |
| full | DAC2020_bm10 | 112.81 | 34.60 | 17.06 | 2.03x | 240/316 | identical to Java |
| full | DAC2020_bm11 | 21.80 | 6.46 | 3.44 | 1.88x | 80/92 | identical to Java |
| full | CM5_MINIMA_3 | 409.82 | 65.68 | 26.70 | 2.46x | 344/539 | identical to Java |
| full | StickHub | 58.06 | 17.10 | 9.82 | 1.74x | 167/202 | identical to Java |
| full | complex_hierarchy | 10.49 | 1.08 | 0.88 | 1.23x | 42/60 | identical to Java |
| full | ecc83-pp | 3.26 | 0.01 | 0.00 | — | 6/6 | identical to Java |
| full | ecc83-pp_v2 | 3.17 | 0.01 | 0.01 | — | 6/7 | identical to Java |
| full | interf_u | 97.50 | 25.09 | 17.08 | 1.47x | 164/212 | identical to Java |
| full | multichannel_mixer-unrouted | 109.61 | 18.29 | 15.13 | 1.21x | 413/422 | identical to Java |
| full | multichannel_mixer | 69.71 | 10.29 | 8.30 | 1.24x | 521/540 | identical to Java |
| full | pic_programmer | 4.70 | 0.18 | 0.14 | 1.29x | 13/17 | identical to Java |
| full | sonde xilinx | 6.19 | 0.39 | 0.24 | 1.63x | 13/18 | identical to Java |
| ar | CM5_MINIMA_3 |  | 30.85 | 9.22 | 3.35x | 298/339 | identical to pre-change |
| ar | DAC2020_bm04 |  | 49.13 | 25.77 | 1.91x | 488/343 | identical to pre-change |
| ar | interf_u |  | 9.43 | 7.00 | 1.35x | 154/201 | identical to pre-change |
| par1 | DAC2020_bm11 |  | 6.24 | 3.17 | 1.97x | 70/83 | identical to pre-change |
| par1 | interf_u |  | 24.83 | 17.07 | 1.45x | 163/211 | identical to pre-change |
| par1 | DAC2020_bm06 |  | 9.50 | 6.28 | 1.51x | 27/34 | identical to pre-change |
| par8 | DAC2020_bm11 |  | 2.54 | 1.50 | 1.69x | 136/179 | identical to pre-change |
| par8 | interf_u |  | 13.56 | 9.74 | 1.39x | 251/345 | identical to pre-change |
| par8 | DAC2020_bm06 |  | 4.02 | 2.89 | 1.39x | 80/123 | identical to pre-change |
| full | **sum of 20 boards** | 2102.4 | 951.6 | 608.4 | 1.56x | | |

Peak RSS grows by 10–30 % on most boards and by 57 % on CM5_MINIMA_3 (mimalloc, the grid index,
the compact node copy, the per-thread caches); bm04 uses less.


## Next ideas

* The remaining profile is flat: 45-degree `completeShape` tree walk (~10–16 %, order-exact
  traversal of the Java-shaped tree), grid scans (~7 %), `Simplex` redundancy/intersection
  (~10 %), maze `TreeSet` inserts (~6 %), allocation (~8 %).
* Cache directions of box/octagon border lines at construction (known unit directions).
* Reuse buffers in `completeShape`/`restrainShape` (per-obstacle `Vec`s).
* Profile-guided optimization of the release build.
