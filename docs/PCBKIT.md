# pcbkit branch

`pcbkit` is the claude-pcb-rules integration branch of this fork (origin `jubaumann23/fastroute`,
upstream `parisxmas/fastroute`), based on upstream tag `v0.1.13`. It adds a stdio protocol server
(`serve`, JSON lines, spec in the toolkit repo `docs/router-protocol/SPEC.md`) and a few default-off
hooks in core. The placer never links this code; the two meet only over the protocol.
Rule: the patch to upstream core stays small. Server logic lives in its own crate/module; core gets
only minimal, default-off hooks, each listed in the ledger below.

Worker branches are `pcbkit-<task>` worktrees under `/home/jubau/coolProjects/fastroute-wt/<task>`.

## Shared, gitignored `reference/`

`/home/jubau/coolProjects/fastroute/reference/` is shared by all worktrees (`/reference` is in
`.gitignore`; `scripts/pcbkit-gate.sh` symlinks `<worktree>/reference` to it when absent).

| Path | What | How it was made |
|---|---|---|
| `reference/freerouting` | Java freerouting at commit `aa909a3` (the parity source commit named in README.md) | `git clone https://github.com/freerouting/freerouting.git` then `git checkout aa909a3` |
| `reference/jdk25` | Temurin JDK 25 (system java is 17; the Java build needs 25) | Adoptium API `/v3/binary/latest/25/ga/linux/x64/jdk/hotspot/normal/eclipse`, extracted |
| `reference/freerouting-parity` | copy of the checkout with `docs/parity/TimeLimit.patch` applied | see `scripts/java-parity.sh` header |
| `reference/bin/freerouting-parity.jar` | the deterministic Java parity jar | `JAVA_HOME=reference/jdk25 ./gradlew -q executableJar -x test -x spotlessCheck -x checkstyleMain --no-daemon` in `freerouting-parity` |
| `reference/parity-baseline/DAC2020_bm08.ses` | `scripts/java-parity.sh <bm08.dsn> <out>`; equals `crates/fastroute/testdata/bm08_full.ses` | needed by `fr-io` `ses_and_post_load_parity` |
| `reference/parity/bm06_1.ses` | `scripts/java-parity.sh <bm06.dsn> <out> --router.optimizer.enabled=false` (found by matching the committed Java dump; the other variants tried, `-mp 1` with and without optimizer or fanout, did not match) | same test |
| `reference/pcbkit-corpus/` | stable DSN/SES corpus (toolkit-generated data, never committed) + `MANIFEST.txt` (sha256, published path, original source) | copied from the toolkit scratchpad (`locktest/det/*`, `locktest/runs/**`) plus `tiny.dsn` and `blocked.dsn` of the rust-protocol worktree |

No test is skipped when `reference/` is complete: the gate counts `skipped` lines and fails on any.
Not built (not needed by any test): `reference/bin/freerouting-2.4.1.jar` (only referenced in
comments of generator scripts) and the full 20-board `reference/parity-baseline`; regenerate
those with `scripts/java-baseline.sh` only if a generator is re-run.

### Corpus

`reference/pcbkit-corpus/{det,runs,protocol-fixtures}/...`. `det/<key>/board.{dsn,ses}` are the
determinism boards (hb200, heuristic-baseline-*, energy-12-*); `runs/<key>/<arm>/...` are the lock
experiment boards (66 DSNs, 61 distinct). Every task reads the corpus from this path. `MANIFEST.txt`
holds `sha256  path-in-corpus  original-source` per file.

## Scripts

* `scripts/pcbkit-gate.sh` (no arguments; run from any fork worktree): the FAST gate. **Owner rule
  (2026-10-09, binding): the test suite is unit tests that check "x does y", not runs of real boards;
  the gate runs in seconds.** It does exactly two things: the `ledger` step below, and
  `cargo test --release --workspace` with the upstream slow tests skipped by name (`SKIP_UPSTREAM`
  in the script) and every `#[ignore]`d test not run, plus the rule that no test may skip itself for a
  missing input. `CARGO_SLOT` is required (`CARGO_SLOT=cargo` for plain cargo). The gate builds a
  cheaper release flavour (no LTO, 16 codegen units, no debug info) into its own `target/pcbkit-gate`,
  so `target/release` keeps the shipped profile for the bench and a one-file edit rebuilds in seconds
  instead of over a minute. Measured 2026-10-09: 11 s warm (294 tests run, 25 ignored, 3 upstream
  skipped); a touched `fr-serve` source adds the rebuild, about 20 s in total, a touched `fr-engine`
  source about 35 s.
* `scripts/pcbkit-bench.sh <subcommand>`: everything that is not a unit test, run by hand, one foreground
  call per subcommand. Nothing in it is part of the gate.

  | subcommand | runs | measured wall time |
  |---|---|---|
  | `ignored [cargo args]` | all `#[ignore = "on-demand ..."]` tests except the determinism sweep: corpus boards routed, rip-up falsification, snapshot/scratch fidelity on hb200 and energy-12-1, stock equivalence on all fr-io test DSNs. Split with cargo args, e.g. `ignored -p fastroute --test serve_snapshot` | `serve_blockers` 59 s, `serve_move` 125 s, `serve_route` 34 s, `serve_settings` 50 s, `serve_baseline` 4 s, `serve_lock` 8 s, `serve_snapshot` + `serve_scratch_fidelity` 327 s; about 10 min in total |
  | `determinism` | `serve_determinism_load`: 8 concurrent servers at 2 threads on det/hb200 under `4 x cores` busy threads, two repetitions, SES bytes identical (F1); each wait has a 900 s hard timeout | 100 to 320 s |
  | `upstream` | the upstream test files the gate trims: `fr-engine` `board_replay` (`autoroute_operations_match_java`, 6.3 s) and `fastroute` `pipeline_parity` (`parallel_optimizer_is_deterministic` 1.8 s, `bm02_full_pipeline_matches_java` 1.0 s) | 59 s including the build |
  | `parity-route` | Java vs Rust SES identity on bm02 and pic_programmer (`scripts/parity-route.sh`, needs the Java parity jar) | 49 s |
  | `stock-pin` | `$PCBKIT_STOCK_CLI` sha256 equals the `STOCK_CLI_SHA256` line below | 0 s |
  | `plain-bin` | builds the shipped binary (no `test-hooks`) into `target/pcbkit-plain`; fails if `FR_SERVE_TEST_PANIC` is in it | 41 s |
  | `conformance` | `router_conformance.py` (`$PCBKIT_CONFORMANCE`, must be the `CONTRACT_COMMIT` checkout) at `--threads 1` and `2` on the plain binary, with `--stock-cli $PCBKIT_STOCK_CLI` | minutes |
  | `all` | every subcommand above in that order (over a 600 s foreground limit; call them one by one) | |

  Run the bench before a release and after a rebase (`scripts/pcbkit-rebase.sh --full` does), not on every
  commit. Where an `#[ignore]`d test was the only coverage of a behaviour, a small test on a tiny
  synthetic board stays in the gate: `serve_blockers` `named_blockers_are_causal_on_the_blocked_fixture`
  (causal blockers on blocked.dsn), `serve_move` `move_then_route_from_current_returns_and_is_deterministic_on_tiny`
  (F2 at threads 2 and 4) and `move_then_route_equals_reload_tiny`, `serve_lock`
  `locked_nets_survive_five_seeded_reroutes`, `serve_snapshot` `tiny_threads_{1,4}`, `serve_scratch_fidelity`
  `tiny_*`, `serve_route` `incremental_tiny` and `seeds_are_deterministic_across_processes_and_reported`,
  `serve_settings` (edge clearance and stock equivalence on tiny.dsn with a shrunk outline, written at test
  time), `serve_baseline` (stock equivalence on tiny and blocked), and the `fr-engine` unit tests
  `pcbkit_min_width_tests` for the minimum-width clamp. A new test that routes a corpus board gets
  `#[ignore = "on-demand (scripts/pcbkit-bench.sh): <why>"]`.

  Pinned stock binary: `cargo build --release` of tag v0.1.13 (6e14035) in its own worktree
  `fastroute-wt/base` (detached, clean), `target/release/fastroute`, `fastroute 0.1.13`:

STOCK_CLI_SHA256: 0e1bfbeed55916db2dffd87474e9d4e04ab7e4c824faf09469bb33b335cc929e

  Pinned toolkit contract (the `rust-protocol` commit whose `scripts/router_conformance.py` the
  bench `conformance` subcommand runs; `contract-pin` prints it and FAILs on any other commit or a modified runner):

CONTRACT_COMMIT: 3fc599c34fb1bea792e6e3a9dc4f43ba9ba73918

  Re-pin it here, in the same commit, when the toolkit publishes a contract patch the fork passes.
  The gate prints an INFO row `core-files` listing the non-exempt files changed against the base.

  Rebuilding that worktree changes the hash only if the toolchain changes; then re-pin here in the
  same commit.
  `ledger` step: every path in `git diff --name-only ${PCBKIT_BASE:-v0.1.13}..HEAD` must be exempt
  (`crates/fr-serve/**`, `crates/*/tests/pcbkit_*.rs`, `crates/fastroute/tests/serve_*.rs`,
  `scripts/pcbkit-*.sh`, `scripts/pcbkit/*`, `docs/PCBKIT.md`, `docs/PCBKIT-*.md`, `Cargo.lock`) or appear in the PATCH LEDGER table below.
  Last result: see the commit that last touched this line; conformance runs at t1 and t2 (SKIP only
  if the binary has no `serve`), exit 0.
* `scripts/pcbkit-ab.sh [--parity] [--quick] [--shard i/n] <base> <new> [threads...]`: routes the corpus
  (`crates/fr-io/testdata/dsn` + `reference/pcbkit-corpus`, sha256-deduplicated) with both binaries
  under `--no-time-limits`, thread flags set to N (default 1 and 4), `cmp`s the SES, prints
  SAME/DIFF and wall times, exit 0 only if everything is SAME. A DSN both binaries reject
  identically (`synth_pcb_keepout.dsn`) is `SAME-ERR`. The full corpus takes about an hour; `--quick`
  skips `runs/` (about 1 min per thread count).
  `--shard i/n` routes every n-th file of the sorted, deduplicated list and writes its table to
  `target/pcbkit-ab/<base-sha>-<new-sha>-<mode>-<i>of<n>.txt` (`<mode>` = `parity|default` plus
  `-t<threads>`; shas from `PCBKIT_AB_BASE_SHA`/`PCBKIT_AB_NEW_SHA`, else sha256 prefix of the
  binary). `--summarize --shard-count n` (same binaries, mode, threads) routes nothing and exits 0
  only if all n shard files are complete, hold one row per (file, thread count) of the whole list
  and every row is SAME or SAME-ERR.
  Full-corpus recipe: 12 shards, run 6 at a time per foreground call (one call per half; shard
  wall time is about the sum of its BASE_s column, 6-way contention on 32 cores), one call pair per
  mode x thread count, then four `--summarize` calls. Slowest shards were 440-490 s, so use 16
  shards if the box is slower.
  Full-corpus result (66 deduplicated DSNs, `reference/` is untracked and must be linked into the
  worktree): base stock v0.1.13 vs pcbkit tip 9940d10 (v0.1.13-32-g9940d10), 2026-10-08:
  default t1, default t4, parity t1, parity t4 all `AB: ALL SAME` (66/66 rows each, no DIFF).

### Fixes needed to make upstream's own checks run on Linux

* `scripts/parity-route.sh` assumed macOS `/usr/bin/time -l`; it now also handles GNU time (RSS
  divisor via `RSS_DIV`). Output on macOS is unchanged.
* `crates/fr-jcompat/tests/jdk_vectors.rs` `sum_matches_jdk` failed on x86: `inf - inf` yields a
  NaN with the sign bit set (0xfff8...). The generator `crates/fr-jcompat/java/MiscGen.java:65-67`
  writes `Double.doubleToRawLongBits` (not `doubleToLongBits`), so the vectors hold the positive
  quiet NaN 0x7ff8... only because they were generated on a platform where `inf - inf` is 0x7ff8...
  (ARM); on x86 Java and Rust alike give 0xfff8.... The test now compares NaN-ness only for NaN
  results (`canonical_nan_bits`); every non-NaN comparison is unchanged and bit-exact.
* Observation, not changed: `pic_programmer` parity log lines differ from Java's only by an
  added annotation ("1 of them pre-existing between fixed items"); the SES is byte-identical, so
  the bench judges on `ses=IDENTICAL`.

## Rebasing on a new upstream tag

`scripts/pcbkit-rebase.sh <tag> [--quick|--full]` automates the net-patch variant of this procedure (see
`docs/PCBKIT-REBASE.md`, "The script"); the manual steps below remain the reference.

1. `git fetch upstream --tags`; `git rebase <new-tag>` (or merge) the `pcbkit` branch; resolve
   conflicts at the ledger sites below.
2. Build the old and new upstream binaries (`cargo build --release` at each tag) and run
   `scripts/pcbkit-ab.sh <old-upstream-bin> <new-upstream-bin>` to see the expected drift in the
   corpus (differences here are upstream's, not ours).
3. Run `scripts/pcbkit-gate.sh`, then `scripts/pcbkit-bench.sh upstream` and `parity-route` (upstream tests and parity must stay green; re-seed
   `reference/freerouting` if upstream names a new parity commit and rebuild the jar as above).
4. Run `scripts/pcbkit-ab.sh <pcbkit-before-rebase-bin> <pcbkit-after-rebase-bin>`; with the
   hooks default-off, drift must equal step 2.
5. Re-run `router_conformance.py` (`pcbkit-bench.sh conformance` does this at threads 1 and 2) and bump the router build hash.

## `fastroute serve` usage

```
fastroute serve            # JSON lines on stdin, one response line per request on stdout, logs on stderr
```

Protocol 1.2.0 (spec: toolkit `docs/router-protocol/SPEC.md`). First line must be `hello` (protocol, client, threads,
settings); the reply lists the capabilities this build claims (`budget check congestion incremental locking move seed
snapshot starts widen`; `blockers` is added when `ops/blockers.rs` `CLAIMED` is true) and the router build hash. Then `load`
(DSN path or text), `route`, `lock`/`unlock`, `move`, `snapshot`/`restore`, `congestion`, `export` (`ses`), `shutdown`. `route.starts` (1..16, default 4 = the CLI) sets the multi-start count and the result echoes it.
Seed 0 with `nets: all` equals the stock CLI byte for byte; results are deterministic at a fixed thread count.

The toolkit finds the server through one environment variable holding the full command line (shlex-split):

```
PCBKIT_FASTROUTE_SERVE_CMD="/home/jubau/coolProjects/fastroute/target/pcbkit-plain/release/fastroute serve"
```

Build that binary with `CARGO_TARGET_DIR=<checkout>/target/pcbkit-plain cargo build --release -p fastroute`: it is the
shipped binary, without the `test-hooks` feature. `target/release/fastroute` is built by `cargo test` WITH test-hooks
(it contains the `FR_SERVE_TEST_PANIC` fault injection), so do not point the toolkit at it. The bench `plain-bin` subcommand
builds it this way and fails if `strings` finds `FR_SERVE_TEST_PANIC`; `conformance` runs the runner on it.

(any absolute path to a built `fastroute` binary followed by the word `serve`). Conformance:

```
python3 <toolkit>/scripts/router_conformance.py --server "$BIN serve" --stock-cli "$BIN" --threads N
```

passes with every claimed capability at N = 1, 2, 4, 8 (checked on `pcbkit-serve-integration`). Combined-capability
test: `crates/fastroute/tests/serve_combined.rs` (lock, move off locked nets, targeted route, snapshot, move, route,
restore, export; locked nets byte-identical throughout, only affected nets change).

## Serve `widen` and `check` (protocol 1.2, `crates/fr-serve/src/ops/{widen,check}.rs`, tests `serve_necks.rs`)

No core hook and no ledger row: both ops use the existing clearance engine (`fr_engine::drc`) and board API from `fr-serve`.

* `widen` (capability `widen`) does what the toolkit's `route_necks.widen` does after a route, without KiCad: every
  `Unfixed`/`ShoveFixed` trace narrower than its net class width (or `widths[net]`) is replaced by the same trace at the
  target width; a candidate is kept only if `clearance_violations` of the new item names no item the old one did not
  (strict: `clearance_tolerance_um` is 0 for the run, restored after). A trace that cannot take the target is split into one
  trace per segment, and a segment that cannot is bisected down to 1 um. Widened and split traces are new items (new ids).
  DSN `fix` wiring (`SystemFixed`) and locked wiring (`UserFixed`) are never touched. It is a state-changing op (history).
* `check` (capability `check`) is read-only: clearance (`all_clearance_violations` on a copy with no tolerance), track width
  below class / minimum, via annular ring and drill (drill from the padstack name `Via[a-b]_<pad>:<drill>_um`), unconnected.
  Not checked: zone fill, thermal reliefs, silk, courtyards, solder mask, hole clearance, custom rules; KiCad DRC stays the
  authority. It is not part of the determinism claim for `wall_ms` only.
* The router counts a clearance shortfall up to `router.clearance_tolerance_um` (1 um) as clear; `check` and `widen` use 0.
* `check` counts one clearance finding per place two copper items of different nets are too close (violations of an item
  pair are merged when their overlap centres lie within one clearance), same-net copper is never a finding (the router's own
  `attach off` rule keeps same-net vias apart; KiCad does not report it), and a pair of multilayer items is one finding.
  Measured against KiCad DRC (toolkit `route_local.sh --fast --router-widen`, rp2040env and stm32io, placer seeds 1-3,
  2026-10-10): on the boards as `widen` leaves them, clearance 0 against 0 on all 6, via annular ring (75/75/76), drill and
  unconnected counts equal. With every wire fattened by 20 / 50 um (real violations): `check` found 91 % / 94 % of KiCad's
  clearance findings (313 of 343, 482 of 513, matched by net pair) and reported 36 % / 39 % more (466 / 715 against 343 / 513),
  because its convex pad and via shapes are polygons that enclose KiCad's arcs and it counts per trace item pair location.
  It is a conservative screen, not a count that equals KiCad's.

## Locking (serve, capability `locking`)

No new core hook. `crates/fr-serve/src/ops/lock.rs`: a trace or via is locked iff it is `UserFixed`
and not part of the DSN's own `UserFixed` wiring (`LockRegistry::base`). Hook H2b
(`keep_fixed_on_split`) is switched on with the first lock or `load.lock_initial`, not at load, so a
board that never locks is byte-identical to the stock CLI. Locked nets are subtracted from the H6 mask
in `route.rs` (mask stays `None` when nothing is locked); the registry is re-derived from the board
after every route. `export` writes locked items like routed wiring (no `(type protect)`). Wire ids are
the board's `ItemId`s. An empty `lock {"nets": []}` is a query.

## PATCH LEDGER

Final totals for `git diff --stat v0.1.13..HEAD -- crates/fr-engine crates/fastroute/src` (core and entry point, excluding
the test files): 15 files, 301 insertions, 15 deletions across H0 (11), H1 (68), H1b (19), H2b (10), H3 (109), H5 (12), H6 (14), F2 (about 60);
the `core-files` row of the gate (which also counts `crates/fastroute/Cargo.toml` and `scripts/parity-route.sh`) lists 17 files, 16 before F2 (F2 adds `pipeline/optimizer.rs`);
the numbers below are per hook. Hook tests: `crates/fr-engine/tests/pcbkit_{blockers,board_hooks,seed_mask}.rs`.

Every change to upstream-owned files. Hooks are default off: with the flag unset the output is
byte-identical to upstream (`pcbkit-ab.sh` proves it).

| Hook | File:line | Lines changed | Default-off flag | Why |
|---|---|---|---|---|
| H0 serve dispatch | `crates/fastroute/src/main.rs:790-793`, `crates/fastroute/Cargo.toml:14` | +4 + 5 (main.rs; Cargo.toml: the `fr-serve` dependency and its `test-hooks` dev-dependency) | `serve` subcommand only (the `FR_SERVE_TEST_PANIC` hook is compiled only under `cargo test`) | route `fastroute serve` to the server crate |
| H0 dep | `crates/fr-engine/Cargo.toml:17-18` | +2 | dev-dependency only | `fr-io` dev-dep for the pcbkit hook tests |
| H1 move | `crates/fr-engine/src/board/basic_board.rs:1068-1121` (`place_component`), `crates/fr-engine/src/structure/component.rs:95-101` (`Component::set_pose`), `:286-291` (`Components::set_pose`) | +55 + 7 + 6 = +68 | via protocol `move` only | apply part moves to the loaded board |
| H1b rigid wiring move | `crates/fr-engine/src/board/basic_board.rs:1049-1066` (`turn_translate_wiring`) | +19 (1049-1067, incl. one blank) | via protocol `move` (`carry: fixed`) only | turn a fixed trace/via by k*90 degrees about the old part origin and translate it, in place (same id), so plane fan-out follows its part |
| H2b keep-fixed-on-split | `crates/fr-engine/src/board/basic_board.rs:74-75` (field `keep_fixed_on_split`), `:177` (init), `crates/fr-engine/src/board/shape_trace_entries.rs:92-96`, `crates/fr-engine/src/board/trace_ops.rs:548-558` | +2 +1 (basic_board), +3 -1 (shape_trace_entries), +4 -2 (trace_ops) = +10 -3 | via protocol locks only | locked wires survive trace splitting. CAVEAT: upstream `remove_item` refuses UserFixed items, so a cut-out on a UserFixed parent leaves the parent and adds UserFixed duplicate pieces; locking must not cut out locked traces. |
| H3 blockers | `crates/fr-engine/src/autoroute/control.rs:15,75,173` (`collect_blockers`, `AutorouteAttemptResult.blockers`), `crates/fr-engine/src/autoroute/engine.rs` (`blockers` field), `crates/fr-engine/src/autoroute/router.rs` (`autoroute_connection` wrapper), `crates/fr-engine/src/autoroute/maze.rs` (`note_blocker`, `note_wall_blockers` + 5 call sites), `crates/fr-engine/src/pipeline/autorouter.rs:158-197` (`BatchAutorouter::route_connection_alone`, a wrapper of `route_connection_alone_on`, which also returns the routed copy), test data `crates/fr-engine/tests/data/blocked.dsn` | +109 -6: control.rs +8 -3, engine.rs +3, maze.rs +39 -2 (call sites 343, 448, 1018, 1023, 1028; fns 1176-1207), router.rs +17 (59-75), autorouter.rs +42 -1 (the other +6 of its +48 are H6) | `ctrl.collect_blockers` (default false); only an extra push, no change to order, costs or RNG | report which items block a connection. Fixed items (pads, locked wires, keepouts) have no expansion room, so `note_wall_blockers` queries the tree for fixed items touching each expanded free room; rippable-item obstacles are recorded where `check_ripup` is negative. |
| H5 order seed | `crates/fr-engine/src/board/routing_board.rs:219-220` (`order_seed` field), `:268` (init), `crates/fr-engine/src/pipeline/mod.rs:187-190` (`run_pipeline`), `:276-280` (`multi_start`) | +12 -1 (routing_board 2+1, mod.rs 4+5-1) | seed unset = upstream order | deterministic net order per request seed |
| H6 net mask | `crates/fr-engine/src/board/routing_board.rs:221-222` (`route_nets` field), `:269` (init), `crates/fr-engine/src/pipeline/autorouter.rs:253-258`, `crates/fr-engine/src/pipeline/fanout.rs:92-96` | +14 (routing_board 2+1, autorouter 6, fanout 5) | mask unset = all nets | route only a subset of nets |
| F2 stale rooms + bounded retry | `crates/fr-engine/src/board/routing_board.rs` (`clone_for_worker`), `crates/fr-engine/src/pipeline/autorouter.rs:140,170,672` and `crates/fr-engine/src/pipeline/optimizer.rs:424,819` (clone sites switched to it), `crates/fr-engine/src/autoroute/engine.rs` (`MAX_ROOM_FAULTS`, `take_engine_fault`, `room_faults`, test hook), `crates/fr-engine/Cargo.toml` (`test-hooks` feature) | +15 routing_board, 1 line each at the 5 clone sites, +45 engine.rs, +3 Cargo.toml | rooms: none present = identical output; fault counter: only after 8 consecutive panics | a worker board must not carry expansion rooms of an engine it does not have (F2); a repeating completion panic can no longer loop |
| H7 deterministic | `crates/fr-engine/src/pipeline/mod.rs` (`PipelineContext.deterministic` field + default, `multi_start` variant ctx, multi-start skip guard, default optimizer budget guard), `crates/fr-engine/src/pipeline/optimizer.rs` (greedy-phase wall budget guard), `crates/fr-engine/src/pipeline/autorouter.rs` (slow-pass stagnation guard), `crates/fastroute/src/main.rs` (`deterministic: false` in the CLI's ctx literal) | +10 -4 in fr-engine core (mod.rs +8 -2 incl. 4 doc lines, optimizer.rs +1 -1, autorouter.rs +1 -1), +1 -1 main.rs | `ctx.deterministic` (default false; only `fr-serve` sets it) | serve results independent of CPU load (F1): no wall-clock decision changes the output |
| H8 min-width clamp | `crates/fr-engine/src/autoroute/control.rs` (`clamp_neckdown_half_width` now calls the free `clamp_half_width_to_min`, plus the `pcbkit_min_width_tests` unit-test module) | +30 -5 | none (behaviour unchanged; `router.min_trace_width_um` unset = no clamp) | the neck-down floor is unit-testable without routing a board (owner rule 2026-10-09) |
| T1 test-only | `scripts/parity-route.sh` (lines 15-21, 26, 28, 57, 59-60), `crates/fr-jcompat/tests/jdk_vectors.rs:293-320` | 17 and 16 | n/a | Linux support, see above |

## F2: move, then route from the current board at 2+ threads (fixed)

Finding F2 of `docs/PCBKIT-REBASE.md`: after `move`, `route {from: current}` at threads >= 2 never answered.

* Cause: a board clone drops the autoroute engine but keeps the expansion rooms the engine left in the search
  trees (`autoroute_maintenance.maintain_database`). The parallel autorouter passes (`snapshot`), the parallel
  optimizer (`evaluate_fresh`, greedy `trial`) and `route_connection_alone_on` route on such clones with a fresh
  engine that knows none of those rooms, so the neighbour search panicked (`search_tree.rs:773` `rooms[&key]`, or
  `engine.rs:130` "shape is null") in `complete_expansion_room`, which logs and returns, and the maze search then
  asked for the same room again, for ever. Threads 1 never clones the board, so it was unaffected.
* Fix 1: `RoutingBoard::clone_for_worker()` (a clone with the rooms of all search trees removed) at the five
  clone sites above. Boards without rooms are copied exactly as before, so output at threads 1 and for every board
  that never hit the bug is unchanged (`pcbkit-ab.sh --quick` ALL SAME).
* Fix 2 (bound): `AutorouteEngine::complete_expansion_room` counts consecutive panics; after 8 (`MAX_ROOM_FAULTS`)
  it records an engine fault, requests a stop and unwinds out of the search. `fr-serve` `route` clears the fault
  before the pipeline and, if it is set afterwards, drops the working copy and answers the SPEC error `internal`
  (session board unchanged, no contract change).
* Tests (`crates/fastroute/tests/serve_move.rs`, phase `tests-serve-b`): `move_then_route_from_current_returns_at_threads_2_and_4_and_is_deterministic`
  (energy-12-1, move R2 and C18, route nets:all from:current, two server processes give identical SES bytes at
  threads 2 and 4, each under a 240 s watchdog that kills a hung server so the test fails instead of stalling) and
  `a_worker_panic_is_an_internal_error_not_a_hang` (a forced completion panic, `FR_ENGINE_TEST_PANIC_ROOMS=<n>`,
  compiled only with the `test-hooks` feature, returns `internal`). Probe: `sweep.py --probe-carry-hang` prints
  `ok` at threads 1, 2, 4 and 8.

## Load-independent serve routing (F1)

`fastroute serve` sets `PipelineContext.deterministic` (hook H7). With it, none of the wall-clock decisions below
can change the result, so output depends only on seed and thread count, never on CPU load:

* greedy-phase budget (`optimizer.rs` `apply_greedy`: stop when `start.elapsed() >= budget`, budget derived from the
  pass's wall time clamped to 2..60 s): the phase now ends only after 10 rejected candidates in a row or when all
  candidates were tried. This was the live one: under CPU oversubscription it fires after 2 s.
* default optimizer budget (`pipeline/mod.rs`, `routing_start.elapsed().max(60 s)`): was already inert under
  `wall_clock_limits: false` (the deadline is only built in `if ctx.wall_clock_limits`); also guarded for clarity.
* slow-pass stagnation stop (`autorouter.rs`, pass longer than 20 s) and the multi-start skip (first run longer than
  600 s): both are wall-clock thresholds, both skipped in serve.

An explicit client `budget` still means wall time and is the only nondeterministic path (SPEC). The stock CLI is
unchanged (`deterministic: false`), so on a board where the stock run itself hits one of these stops under load, serve
(which never stops there) differs from it. Measured without load, seed 0, `sweep.py --equiv-only --threads 1,2`,
core corpus (9 boards, 36 rows): all rows SAME or SAME-ERR, 0 bad; `pcbkit-ab.sh --quick` before vs after: ALL SAME.
No stock-equivalence row had to be excluded. Test: `serve_determinism_load` (`pcbkit-bench.sh determinism`); on the shared
development host it was already green before the fix (the greedy stop needs a trial that takes over 2 s, which a
loaded host did not produce for hb200 or energy-12-1), so it is a regression guard, not a proven red-to-green test.

## Serve `move` semantics (capability `move`, `crates/fr-serve/src/ops/move_.rs`)

* Rip set: traces and vias of the router (`Unfixed`, and `ShoveFixed`, which is what the router marks the pad stubs it creates; a
  DSN states its own fixed wiring as `fix` = `SystemFixed`) reached from a moved part's pads through same-net copper overlap
  (`all_contacts`, never through another pad). Locked = `UserFixed` or a locked net.
* Carry (`carry: fixed`, default): `SystemFixed` traces/vias connected to the part's pads whose connected island touches only
  that part's pads and planes move rigidly (turn by the rotation delta about the old origin, then translate). Only a delta
  that is a multiple of 90 degrees carries; any other island stays put. Other `SystemFixed` wiring never moves.
* `route.from = "scratch"` also removes `ShoveFixed` router stubs (otherwise a scratch route depended on the previous route).
* Fidelity: move then `route scratch` equals a fresh load of the DSN with the edited place record then `route`
  (`crates/fastroute/tests/serve_move.rs`, tiny and two corpus boards, with `carry: none`: a reload leaves DSN-fixed wiring behind).
* Protocol coordinates are DSN resolution units: `board = round(units / resolution * scale)`; a pose from an SES place record
  round-trips exactly.

## Serve `route` from scratch is placement-pure (`crates/fr-serve/src/load.rs` `rebuild_for_scratch`)

* Contract (SPEC 7): `route` with `from: "scratch"` and all nets returns what a fresh `load` of the same placement plus one
  scratch route returns, whatever the session did before (earlier routes, other seeds, moves, locks).
  Tests: `crates/fastroute/tests/serve_scratch_fidelity.rs` (tiny, det/hb200, det/energy-12-1, threads 1 and 2).
* Why ripping in place is not enough: after the rip the items (ids, kinds, fixed states), the settings, the rules and the
  components equal a fresh load's (checked on hb200), yet the route differs (hb200: 619 wires against 595). The rest is private
  `BasicBoard`/`RoutingBoard` state that history leaves behind (search trees, `undo`/`revision`, the trace half width
  watermarks `min/max_trace_half_width`, `basic_board.rs:513-514`, which only ever widen). Setting the watermarks and the id
  generator back did not change the result, so the residue is in the trees or journals; it is not isolated further.
* So scratch rebuilds: the session keeps the `load` inputs (`Origin`: DSN bytes, initial SES, `lock_initial`) and the arguments of
  every successful `move` (`Board::moves`). A scratch route with all nets builds a new board from the inputs, replays the moves
  (`unlock: true`, as the live moves did; DSN-fixed fan-out is carried exactly as before), re-inserts the locked wiring
  (`ops/lock.rs` `carry_locked_wiring`: new items under the ids they had, so `lock {wires:[id]}` keeps naming them), and raises
  the id generator to the live maximum (ids are never reused, SPEC 4). Then the usual rip of `Unfixed`/`ShoveFixed` wiring runs.
  No core change and no ledger row.
* `route` with a net list and `from: "scratch"` stays in place: it keeps the other nets' wiring by definition, so it depends on
  the session.
* Locked wiring is the lock registry's `UserFixed` wiring, not DSN `fix` wires: the two route the other nets differently, so a
  board with `fix` wires is not a reference for a locked one.

## Serve `blockers` names causal objects (`crates/fr-serve/src/ops/blockers.rs`, falsified by `tests/serve_blockers.rs`)

The first version named what the alone-route touched. On a real board that is hundreds of items (energy-12-1
U3 opens: 421 to 440 objects), and the fallbacks (`nearest_wiring`, the wiring on the free path) named wires
that did not matter. Falsification on hb200 and energy-12-1 (seed 0, one thread; rip-up of the named routed
wires, then `route nets: [net]`; or the named wiring taken off a copy, then the alone-route) found:

| open | first answer | cause found | now |
|---|---|---|---|
| hb200 `MCU.VDD_1V1` C18-1 to a via | congestion, 3 wires named, ripping them did not route it (nor did the pipeline with nothing ripped) | the free path touches 3 wires, but with the routed wiring kept other nets' wires also block; the search names none of them | congestion; 5 to 8 routed wires named, found by `causal_set`; ripping them routes the net (control without rip: open) |
| hb200 `I2C_BUS_SDA` R10-1 to a via; energy-12-1 `MCU.XOUT` C20-1 to Y1-3 | congestion, nearest wires named | the alone-route returns "routed" with nothing added: fixed copper of the net already joins the ends for the maze, but not for the connectivity (energy-12-1: a fixed wire ends on the corner of another, a T-junction, `(wire (path F.Cu 150 37400 -38200 39270 -38200))` ends where `(path ... 39270 -39100 39270 -38200 ...)` has a corner) | `blocked`; the two items at the gap are named (`gap_items`). Not routable by any router; the DSN writer must split the wire at the junction |
| energy-12-1 U3-21, U3-24, U3-51 (`MCU.XOUT_R`, `MCU.SWCLK`, `MCU.QSPI_IO3`) | blocked, 20 objects, 3 to 10 of them wires, removing them did not unblock | the first 20 touched objects are not a cut; more rounds are needed | blocked; 5, 18 and 20 fixed wires and vias (DSN fanout copper of GND, VDD_1V1, VDD_3V3 and the neighbouring signals), removing them lets the alone-route succeed |

`causal_set`: remove the wires and vias among the hits (round by round, then, on the kept board, every other
net's routed item nearest the line when the search names none), until the attempt routes, then keep the shortest
prefix that is enough (binary search; removing more never hurts). Pins, keepouts and the boundary are named only
when taking off all wiring is not enough. Class rules: strip-mode alone-route fails: `blocked`; routes with items
added: `congestion`, attributed on the board with the routed wiring kept; routes with nothing added: `blocked`
(SPEC 5.6 calls this `congestion`; the SPEC does not cover it, and a contract patch should say so).

Comparison with the earlier manual attribution (lock test, pitfall 8: "own_via blames point at U3's own fanout
via"): the named vias alone do NOT unblock U3-21 (2 vias), U3-24 (6) or U3-51 (6); with the fixed wires named next to
them they do. Fanout vias are part of the answer, not all of it. Every named wire and via of these opens is fixed
copper from the DSN, so the router cannot rip it: the fix lies in the DSN (do not emit that fanout), not in the
placement of a movable part. Seeds 1 and 2 give the same opens (hb200 seed 1: the congestion open again, 5 wires, fixed
by ripping them; seed 2 routes it).

The test (`named_blockers_are_causal_on_corpus_boards`) uses `fr_serve::falsify::Probe` (feature `test-hooks`, never in
a shipped build): the protocol has no op that removes one wire, so the test removes items in-process. A ripped board
routes slightly differently from a fresh one (see "route from scratch is placement-pure"); the pipeline check is a
sufficient-condition check, with a control route in the same state.
