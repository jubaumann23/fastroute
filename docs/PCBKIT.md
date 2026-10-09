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

* `scripts/pcbkit-gate.sh` (run from any fork worktree): `cargo test --release --workspace
  --no-fail-fast` via `$CARGO_SLOT`, fails on any skipped parity test; Java-vs-Rust SES identity on
  bm02 and pic_programmer (`scripts/parity-route.sh`); when the binary has `serve`,
  `router_conformance.py` (`$PCBKIT_CONFORMANCE`) at `--threads 1` and `2`, with `--stock-cli` set to
  the pinned stock binary `$PCBKIT_STOCK_CLI`. `CARGO_SLOT`, `PCBKIT_CONFORMANCE` and
  `PCBKIT_STOCK_CLI` are required (exit 2 with the missing names otherwise; `CARGO_SLOT=cargo` for
  plain cargo). The `stock-pin` step checks the stock binary's sha256 against the line below, so the
  stock-equivalence check always compares against upstream v0.1.13, never against the fork itself.
  Prints a PASS/FAIL table.

  **Phases** (each one foreground call, well under the 600 s limit; the no-flag run does all of it
  in one call and takes about 9 to 10 minutes under load):

  | `--phase` | runs | measured wall time |
  |---|---|---|
  | `tests-core` | `cargo test` for every crate except `fastroute`, plus fastroute `--bins` and its non-`serve_*` test targets (`pipeline_parity`) | 12 to 45 s |
  | `tests-serve-a` | fastroute `serve_*` targets except move, snapshot, blockers and scratch_fidelity (baseline, combined, congestion, lock, route, settings) | 112 to 120 s |
  | `tests-serve-b` | fastroute `serve_move` | 150 to 290 s |
  | `tests-serve-c` | fastroute `serve_snapshot` | 204 to 261 s |
  | `tests-serve-d` | fastroute `serve_blockers` (blocked.dsn, errors, determinism, hb200 and energy-12-1 opens, the causality falsification) | 140 to 150 s |
  | `tests-serve-e` | fastroute `serve_scratch_fidelity` alone (it is the slowest target) | 200 to 305 s |
| `final` | records check, coverage, ledger, parity-skips, parity-route, stock-pin, conformance t1 and t2, plain-bin, conformance-plain-t1 | 160 s (includes the `--list` coverage pass and the plain-binary build) |

  Times were measured on the shared box at load average 8 to 17 with a warm build (a cold
  release build of the workspace adds about 150 s to whichever phase runs first, so build once with
  `cargo test --release --workspace --no-run` before the phases). Every phase must stay under
  480 s; if a `serve_*` target grows past that, move it to its own phase in `phase_cargo_args`.

  Each test phase writes `target/pcbkit-gate/<phase>.<HEAD sha>.{log,rc}`; the `.rc` holds `rc`,
  `executed`, `dirty` and `binsha` (sha256 of `target/release/fastroute`). The skip-line rule
  applies to every phase log. `final` refuses with a FAIL row when a record for the current HEAD is
  missing or failed, the worktree is dirty (now or when the record was taken), the binary sha
  differs from the recorded one, or the phases do not cover the workspace: the test names from
  `cargo test --release --workspace -- --list` must all appear in the executed names of the
  phase logs. Run the test phases on the commit you intend to report, then `final`; any commit
  after them makes the records stale.

  Pinned stock binary: `cargo build --release` of tag v0.1.13 (6e14035) in its own worktree
  `fastroute-wt/base` (detached, clean), `target/release/fastroute`, `fastroute 0.1.13`:

STOCK_CLI_SHA256: 0e1bfbeed55916db2dffd87474e9d4e04ab7e4c824faf09469bb33b335cc929e

  Pinned toolkit contract (the `rust-protocol` commit whose `scripts/router_conformance.py` the
  `final` phase runs; `contract-pin` prints it and FAILs on any other commit or a modified runner):

CONTRACT_COMMIT: 3fc599c34fb1bea792e6e3a9dc4f43ba9ba73918

  Re-pin it here, in the same commit, when the toolkit publishes a contract patch the fork passes.
  `final` also prints an INFO row `core-files` listing the non-exempt files changed against the base.

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
  the gate judges on `ses=IDENTICAL`.

## Rebasing on a new upstream tag

1. `git fetch upstream --tags`; `git rebase <new-tag>` (or merge) the `pcbkit` branch; resolve
   conflicts at the ledger sites below.
2. Build the old and new upstream binaries (`cargo build --release` at each tag) and run
   `scripts/pcbkit-ab.sh <old-upstream-bin> <new-upstream-bin>` to see the expected drift in the
   corpus (differences here are upstream's, not ours).
3. Run `scripts/pcbkit-gate.sh` (upstream tests and parity must stay green; re-seed
   `reference/freerouting` if upstream names a new parity commit and rebuild the jar as above).
4. Run `scripts/pcbkit-ab.sh <pcbkit-before-rebase-bin> <pcbkit-after-rebase-bin>`; with the
   hooks default-off, drift must equal step 2.
5. Re-run `router_conformance.py` (gate does this at threads 1 and 2) and bump the router build hash.

## `fastroute serve` usage

```
fastroute serve            # JSON lines on stdin, one response line per request on stdout, logs on stderr
```

Protocol 1.0.0 (spec: toolkit `docs/router-protocol/SPEC.md`). First line must be `hello` (protocol, client, threads,
settings); the reply lists the capabilities this build claims (`budget congestion incremental locking move seed
snapshot`; `blockers` is added when `ops/blockers.rs` `CLAIMED` is true) and the router build hash. Then `load`
(DSN path or text), `route`, `lock`/`unlock`, `move`, `snapshot`/`restore`, `congestion`, `export` (`ses`), `shutdown`.
Seed 0 with `nets: all` equals the stock CLI byte for byte; results are deterministic at a fixed thread count.

The toolkit finds the server through one environment variable holding the full command line (shlex-split):

```
PCBKIT_FASTROUTE_SERVE_CMD="/home/jubau/coolProjects/fastroute/target/pcbkit-plain/release/fastroute serve"
```

Build that binary with `CARGO_TARGET_DIR=<checkout>/target/pcbkit-plain cargo build --release -p fastroute`: it is the
shipped binary, without the `test-hooks` feature. `target/release/fastroute` is built by `cargo test` WITH test-hooks
(it contains the `FR_SERVE_TEST_PANIC` fault injection), so do not point the toolkit at it. The gate's `plain-bin` row
builds it this way and fails if `strings` finds `FR_SERVE_TEST_PANIC`; `conformance-plain-t1` runs the runner on it.

(any absolute path to a built `fastroute` binary followed by the word `serve`). Conformance:

```
python3 <toolkit>/scripts/router_conformance.py --server "$BIN serve" --stock-cli "$BIN" --threads N
```

passes with every claimed capability at N = 1, 2, 4, 8 (checked on `pcbkit-serve-integration`). Combined-capability
test: `crates/fastroute/tests/serve_combined.rs` (lock, move off locked nets, targeted route, snapshot, move, route,
restore, export; locked nets byte-identical throughout, only affected nets change).

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
the test files): 15 files, 243 insertions, 10 deletions across H0 (11), H1 (68), H1b (19), H2b (10), H3 (109), H5 (12), H6 (14);
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
| T1 test-only | `scripts/parity-route.sh` (lines 15-21, 26, 28, 57, 59-60), `crates/fr-jcompat/tests/jdk_vectors.rs:293-320` | 17 and 16 | n/a | Linux support, see above |

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
