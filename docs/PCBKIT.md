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

  Pinned stock binary: `cargo build --release` of tag v0.1.13 (6e14035) in its own worktree
  `fastroute-wt/base` (detached, clean), `target/release/fastroute`, `fastroute 0.1.13`:

STOCK_CLI_SHA256: 0e1bfbeed55916db2dffd87474e9d4e04ab7e4c824faf09469bb33b335cc929e

  Rebuilding that worktree changes the hash only if the toolchain changes; then re-pin here in the
  same commit.
  `ledger` step: every path in `git diff --name-only ${PCBKIT_BASE:-v0.1.13}..HEAD` must be exempt
  (`crates/fr-serve/**`, `crates/*/tests/pcbkit_*.rs`, `crates/fastroute/tests/serve_*.rs`,
  `scripts/pcbkit-*.sh`, `docs/PCBKIT.md`, `Cargo.lock`) or appear in the PATCH LEDGER table below.
  Last result: see the commit that last touched this line; conformance runs at t1 and t2 (SKIP only
  if the binary has no `serve`), exit 0.
* `scripts/pcbkit-ab.sh [--parity] [--quick] <base> <new> [threads...]`: routes the corpus
  (`crates/fr-io/testdata/dsn` + `reference/pcbkit-corpus`, sha256-deduplicated) with both binaries
  under `--no-time-limits`, thread flags set to N (default 1 and 4), `cmp`s the SES, prints
  SAME/DIFF and wall times, exit 0 only if everything is SAME. A DSN both binaries reject
  identically (`synth_pcb_keepout.dsn`) is `SAME-ERR`. The full corpus takes about an hour; `--quick`
  skips `runs/` (about 1 min per thread count).

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

## Locking (serve, capability `locking`)

No new core hook. `crates/fr-serve/src/ops/lock.rs`: a trace or via is locked iff it is `UserFixed`
and not part of the DSN's own `UserFixed` wiring (`LockRegistry::base`). Hook H2b
(`keep_fixed_on_split`) is switched on with the first lock or `load.lock_initial`, not at load, so a
board that never locks is byte-identical to the stock CLI. Locked nets are subtracted from the H6 mask
in `route.rs` (mask stays `None` when nothing is locked); the registry is re-derived from the board
after every route. `export` writes locked items like routed wiring (no `(type protect)`). Wire ids are
the board's `ItemId`s. An empty `lock {"nets": []}` is a query.

## PATCH LEDGER

Every change to upstream-owned files. Hooks are default off: with the flag unset the output is
byte-identical to upstream (`pcbkit-ab.sh` proves it).

| Hook | File:line | Lines changed | Default-off flag | Why |
|---|---|---|---|---|
| H0 serve dispatch | `crates/fastroute/src/main.rs:790-793`, `crates/fastroute/Cargo.toml:14` | 4 + 1 | `serve` subcommand only | route `fastroute serve` to the server crate |
| H0 dep | `crates/fr-engine/Cargo.toml:17-18` | 2 | dev-dependency only | `fr-io` dev-dep for the pcbkit hook tests |
| H1 move | `crates/fr-engine/src/board/basic_board.rs:1068-1121` (`place_component`), `crates/fr-engine/src/structure/component.rs:95-101` (`Component::set_pose`), `:286-291` (`Components::set_pose`) | 55 + 7 + 6 | via protocol `move` only | apply part moves to the loaded board |
| H1b rigid wiring move | `crates/fr-engine/src/board/basic_board.rs:1049-1066` (`turn_translate_wiring`) | 18 | via protocol `move` (`carry: fixed`) only | turn a fixed trace/via by k*90 degrees about the old part origin and translate it, in place (same id), so plane fan-out follows its part |
| H2b keep-fixed-on-split | `crates/fr-engine/src/board/basic_board.rs:74-75` (field `keep_fixed_on_split`), `:177` (init), `crates/fr-engine/src/board/shape_trace_entries.rs:92-96`, `crates/fr-engine/src/board/trace_ops.rs:548-558` | 2 + 1, 3, 5 | via protocol locks only | locked wires survive trace splitting. CAVEAT: upstream `remove_item` refuses UserFixed items, so a cut-out on a UserFixed parent leaves the parent and adds UserFixed duplicate pieces; locking must not cut out locked traces. |
| H3 blockers | `crates/fr-engine/src/autoroute/control.rs` (`collect_blockers`, `AutorouteAttemptResult.blockers`), `crates/fr-engine/src/autoroute/engine.rs` (`blockers` field), `crates/fr-engine/src/autoroute/router.rs` (`autoroute_connection` wrapper), `crates/fr-engine/src/autoroute/maze.rs` (`note_blocker`, `note_wall_blockers` + 5 call sites), `crates/fr-engine/src/pipeline/autorouter.rs` (`BatchAutorouter::route_connection_alone`), test data `crates/fr-engine/tests/data/blocked.dsn` | ~75 | `ctrl.collect_blockers` (default false); only an extra push, no change to order, costs or RNG | report which items block a connection. Fixed items (pads, locked wires, keepouts) have no expansion room, so `note_wall_blockers` queries the tree for fixed items touching each expanded free room; rippable-item obstacles are recorded where `check_ripup` is negative. |
| H5 order seed | `crates/fr-engine/src/board/routing_board.rs:219-220` (`order_seed` field), `:268` (init), `crates/fr-engine/src/pipeline/mod.rs:187-190` (`run_pipeline`), `:276-280` (`multi_start`) | 2 + 1 + 4 + 5 (1 replaced) | seed unset = upstream order | deterministic net order per request seed |
| H6 net mask | `crates/fr-engine/src/board/routing_board.rs:221-222` (`route_nets` field), `:269` (init), `crates/fr-engine/src/pipeline/autorouter.rs:253-258`, `crates/fr-engine/src/pipeline/fanout.rs:92-96` | 2 + 1 + 6 + 5 | mask unset = all nets | route only a subset of nets |
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
