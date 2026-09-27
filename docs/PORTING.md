# Freerouting → Rust porting plan

Source of truth: `reference/freerouting/src/main/java/app/freerouting/` (Java, read-only).
Goal: faithful port of the headless routing engine (same algorithms; exact parity
in a deterministic mode), then speed-ups that do not change results.

## Crates

| Crate | Contents | Status |
|---|---|---|
| `fr-dsn` | S-expression lexer, typed DSN model | done (2489/2489 corpus DSNs load) |
| `fr-geom` | `geometry/planar` port | done (bit-exact vs Java golden data) |
| `fr-jcompat` | Java semantics helpers: `Random`, `Collections.shuffle`, `TreeMap` (red-black, for non-transitive comparators), `HashMap<Integer>` iteration order, compensated `DoubleStream.sum`, `String.compareTo` (UTF-16), `Math.round/rint` | done (verified vs JDK 25) |
| `fr-engine` | datastructures, rules, library, board (items, search trees, optimize), autoroute (expansion, maze, path, drill, pipeline), drc, scoring, settings | planned |
| `fr-io` | DSN model → board builder, post-load overrides, SES writer, SES reader (test infra) | planned |
| `fastroute` | CLI | skeleton |

`board` and `autoroute` are mutually dependent in Java (`ShapeSearchTree.completeShape`
uses expansion rooms, `RoutingBoard` owns the `AutorouteEngine`), so they live in one crate.

## Headless call chain (Java)

`Freerouting` → `RoutingJobScheduler` → `HeadlessBoardManager.loadFromSpecctraDsn`
(`DsnReader`; items inserted while parsing; `normalizeAllTraces` after wiring)
→ post-load overrides (`applyRouterSettingsForLoadedBoard`, copper-to-edge, hole
clearance, plane nets, …) → `RoutingPipeline.run`:

1. `BatchAutorouter.runBatchLoop` → `AutorouteBatchLoop.run`: fanout (`BatchFanout`),
   then passes of `AutoroutePassRunner.runSingleThread` (autorouting is single-threaded;
   the multi-thread path is dead code). Per connection: `AutorouteEngine.autorouteConnection`
   (`MazeSearchEngine` → `FoundConnectionLocator*` → `FoundConnectionInserter`), then
   pull-tight `optChangedArea` (1000 ms limit). Stop rules: stagnation (0.5 score, 10 passes),
   `BoardHistory` restore every 4th pass from pass 8.
2. `BatchOptimizer.runBatchLoop`: up to 100 passes; each pass evaluates all route items
   on worker boards (thread pool) and applies only the single best candidate.
3. `SesWriter.write`.

## Porting units (bottom-up)

| Unit | Java | ~Lines | Notes |
|---|---|---|---|
| U0 jcompat | — | — | independent |
| U1 datastructures | `UndoableObjects`, `ShapeTree`, `MinAreaTree`, `PlanarDelaunayTriangulation`, `IdentifierType`, `IndentFileWriter`, `TimeLimit` | 2.3k | needs fr-geom |
| U2 rules | `ClearanceMatrix`, `BoardRules`, `NetClass(es)`, `Net(s)`, `ViaInfo(s)`, `ViaRule` | 2.0k | board back-refs become `Board` methods |
| U3 library | `core/library`: `Padstack(s)`, `Package(s)`, `BoardLibrary`, `LogicalPart(s)` | 0.9k | |
| U4 structure/state | `Layer(Structure)`, `Component(s)`, `ShapeEntrySide`, `Communication`, `ChangedArea`, `io/CoordinateTransform` | 1.1k | |
| U5 items + search trees + BasicBoard | `board/model/items`, `board/trace`, `board/searchtree`, `BasicBoard`, `BoardItemRepository` | 13k | port together |
| U6 DRC + scoring | `DesignRulesChecker` (needed half), `NetIncompletes`, `ClearanceViolation`, `BoardStatistics` | 2.8k | scores are `float`: reproduce casts |
| U7 optimize + RoutingBoard | `TraceTightener*`, `TraceShover`, `ViaOptimizer`, `Forced*`, `RoutingBoard` | 7k | |
| U8 autoroute core | `expansion`, `drill`, `maze`, `path` | 9k | port together, test layer by layer |
| U9 pipeline | `BatchAutorouter`, `AutorouteBatchLoop`, `AutoroutePassRunner` (single-thread), `BatchFanout`, `BatchOptimizer`, `RoutingPipeline` | 3.5k | skip `BatchAutorouterThread` (dead) |
| U10 settings | `RouterSettings` & sub-settings, `DefaultSettings`, `DsnFileSettings`, `SettingsMerger` | 1.5k | independent |
| U11 io | board-building halves of `Structure`/`Network`/`Library`/`Wiring`/…, `HeadlessBoardManager` post-load, `SesWriter`, `SesReader` | 5–6k | |

Order: U0, U1–U4, U10 in parallel → U5 + U11 loader/SES writer (milestone: load DSN,
write unrouted SES, diff against Java) → U6 (+SesReader: score Java-routed boards in Rust)
→ U7 → U8 → U9 → performance work.

## Rust representation (key decisions)

- **Items**: slot map (`ItemKey`) + `ItemId(i32)` (the Java id, the ordering key). The board
  item list iterates in **descending id** (Java `compareTo` is `other.id - id`); model it as
  `BTreeMap<Reverse<ItemId>, ItemKey>` with a cursor that pre-fetches the next entry (like
  `ConcurrentSkipListMap` iterators). `Item` is an enum with a common header; no board
  back-pointer — methods take `&Board`/`&mut Board`. Removed items stay as tombstones
  (`on_board = false`) until a safe point, because Java holds stale references.
- **Search trees**: arena per tree; per-tree side table `ItemKey → leaves + shapes`.
  `MinAreaTree.overlaps` sorts results canonically (rooms first, then items by descending
  id, then shape index). **But** `ShapeSearchTree45Degree/90Degree.completeShape` walk the
  tree themselves and shrink the query while walking, so visiting order changes the rooms
  produced. The Rust `MinAreaTree` therefore reproduces Java's exact tree shape (verified
  against the jar); a different spatial index is only allowed behind `overlaps`.
- **Expansion rooms/doors/drills**: per-connection arena in the `AutorouteEngine`,
  cleared between connections; complete rooms in a separate spatial index merged in front
  of item results. Java `getId()` values used for ordering are reproduced with i32
  wrapping arithmetic (`java_id()` methods), separate from arena indices.
- **Maze queue**: key `(sorting_value, expansion_value, door_id, section)`; Java `TreeSet.add`
  drops equal keys (keeps the old element) — use `BinaryHeap` + present-key set.
- **Non-transitive comparators** (`SortedRoomNeighbours`, fanout ordering, `NetIncompletes`)
  need an exact `java.util.TreeMap` emulation. Subtraction comparators use `wrapping_sub`.
- **Hash order**: only `BasicBoard.normalizeAllTraces` (`HashMap<Integer,…>`) affects results;
  emulate Java `HashMap` iteration. `NetIncompletes`' identity `HashSet` only affects airlines.
- **Undo/snapshots**: cheap `Board: Clone` (flat arenas, `Arc` shapes, `Arc` rules/library).
  `BoardHistory` stores clones; board hash is an incremental content hash. Optimizer undo
  restores a snapshot but carries over `id_generator`, `failure_log` and
  `normalize_suppressed_net_nos` (Java does not undo these).
- **Floats**: keep operation order, no `mul_add`; scoring uses `f32` with Java casts;
  compensated summation for `DoubleStream.sum`; `libm` for trig (fdlibm).
- **Threads**: autorouter single-threaded. Optimizer: `java-compat` mode (sequential, one
  reused worker board) and `parallel-deterministic` mode (fresh clone per candidate, rayon).

## Hot spots to fix without changing results

Linear `getItem(id)` and per-net scans (index them); connectivity recomputed through the
tree (cache by board revision); `BoardStatistics` + full DRC recomputed many times per pass
(memoize, incremental violations — keep summation order); Java serialization for
copies/history/hash (clone + incremental hash); eager trace-log string building (lazy
logging); locked, unbalanced `MinAreaTree` (lock-free, canonical sort); per-connection
engine/room churn (arena reuse, cached door shapes); maze `TreeSet` allocation (heap);
`BigInteger` fallbacks (i128 fast path); O(n²) `ReadSortedRouteItems` (single sort);
optimizer evaluates all candidates but applies one (parallel evaluation).

## Parity

- **Exact** (target): Java with `optimizer.max_threads=1`, time limits disabled/not firing.
  Expect identical ids, per-pass `f32` scores and byte-identical SES. Time limits become a
  pluggable `TimeBudget`; a run where any limit fired is flagged non-comparable.
- **Behavioural** (default config): compare final score, incompletes, clearance violations,
  vias, trace length over several Java runs; Rust must be no worse.
- Layered checks: L1 canonical board dump after load → L2 statistics bit-exact →
  L3 per-connection trace markers (reuse `scripts/tests/compare-logs-v5.py`) →
  L4 per-pass hash/score → L5 SES diff + Java DRC on the Rust SES.

## Baseline (Java 2.4.1, default settings, M-series Mac)

See `reference/baseline/summary.tsv` (generated by `scripts/java-baseline.sh`).
DAC2020_bm01: 154 s wall, 1.96 GB RSS, 1455 GB allocated in total; 8 unrouted, 2 violations.
