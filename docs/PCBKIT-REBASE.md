# pcbkit: upstream rebase procedure, rebase drill, thread/determinism sweep

Companion to `docs/PCBKIT.md` (patch ledger, gate). Everything here was measured on the
`pcbkit` tip `135cc5d` (v0.1.13 plus 45 commits), 2026-10-08/09, on a shared 32-core box
(load average 8 to 17 from other workers throughout, so wall times are upper bounds).

## 1. Rebase drill (local only, nothing pushed, drill branches are not merged)

### Facts

* `git fetch upstream --tags` (read-only): newest upstream tag is **v0.1.13**, and
  `upstream/main` (`6e14035`) **is** v0.1.13. There is no newer upstream tag or commit, so the
  required drill is a rebase onto `upstream/main`, i.e. onto the same base. Say so wherever the
  drill is cited: it proves the procedure and the gate, not a real upstream delta.
* Because the drill target is the current base, a second, harder drill was added: replay the patch
  set onto the two **older** tags v0.1.12 (3 upstream commits back, 11 files) and v0.1.11
  (8 commits back, 22 files, +412/-51). Older bases are the only available way to make upstream
  files differ under the hook sites.

### Result per method (worktree `/home/jubau/coolProjects/fastroute-wt/rebase-drill`)

| method | target | outcome | time |
|---|---|---|---|
| `git rebase upstream/main` (plain, flattens merges) | v0.1.13 | **conflict** at `5dcb67f` (the 7th replayed commit, which touches only our own files: `fr-serve/src/load.rs`, `serve_baseline.rs`, gate and docs, plus a test fix; a history-replay artefact of the merge-based branch, not a hook site) | 0.5 s to the stop |
| `git rebase --rebase-merges upstream/main` | v0.1.13 | clean, 92 todo items, tip identical to `pcbkit` (`135cc5d`) | 0.4 s |
| `git rebase --rebase-merges --onto v0.1.12 v0.1.13` | v0.1.12 | **conflict** replaying merge `5feb2f9` in `crates/fr-serve/src/load.rs` and `crates/fastroute/tests/serve_baseline.rs` (ours, because those files moved across the 45 commits; no core hook file conflicts) | 0.5 s to the stop |
| same, `--onto v0.1.11` | v0.1.11 | same two files | 0.4 s to the stop |
| **net patch**: `git diff v0.1.13 pcbkit \| git apply --3way` on a branch from the tag | v0.1.12, v0.1.11 | **applies cleanly, 0 conflicts at any hook** (H0 to H6, ledger sites below) | 0.07 s |

Conflicts per hook (ledger rows of `docs/PCBKIT.md`): **none** in any of the four net-patch
applications; the only conflicts are history-replay conflicts in `fr-serve` and its tests, caused
by replaying 45 commits and merges one by one. So the recommended procedure is the net patch.

### Recommended procedure (script-ready, about 1 minute of git plus the gate)

```
git fetch upstream --tags                       # read-only
NEW=<newest tag, or upstream/main if no newer tag>
git worktree add ../fastroute-wt/rebase-$NEW -b pcbkit-rebase-$NEW $NEW
cd ../fastroute-wt/rebase-$NEW
git diff v0.1.13 pcbkit | git apply --3way --index   # net patch; conflict markers appear only at hook sites
git commit -m "feat(pcbkit): patch set on $NEW"
ln -s /home/jubau/coolProjects/fastroute/reference reference
# fast gate, then the on-demand bench (see docs/PCBKIT.md), then A/B old vs rebased:
scripts/pcbkit-gate.sh
scripts/pcbkit-bench.sh upstream; scripts/pcbkit-bench.sh parity-route; scripts/pcbkit-bench.sh conformance
scripts/pcbkit-ab.sh --quick <old fastroute> target/release/fastroute 1 4
```

Set `PCBKIT_BASE=$NEW` for the ledger step when the base tag changes, and re-pin
`STOCK_CLI_SHA256` (stock binary of the new tag). History of the old integration branch stays on
`pcbkit`; the new branch carries one squashed patch commit (rebase of the 45-commit history is
not needed and is what conflicts).

### The script: `scripts/pcbkit-rebase.sh <target-tag>`

The procedure above is scripted (fork tooling, ledger-exempt via `scripts/pcbkit-*.sh`; helper
`scripts/pcbkit/rebase_report.py`, `scripts/pcbkit-rebase.sh --self-check`). It creates
`fastroute-wt/rebase-script-<tag>` (branch `pcbkit-rebase-script-<tag>`) at the tag, builds the stock binary of
that tag, applies `git diff v0.1.13 pcbkit` with `git apply --3way --index`, and reports:

* upstream drift: files that differ between v0.1.13 and the target AND carry a ledger hook (the only
  places a conflict can occur);
* apply status per file (clean / conflicts / failed), and for each conflicted file its hunk line spans and
  the PATCH LEDGER hook rows of that file (parsed from `docs/PCBKIT.md` on `pcbkit`);
* on a clean apply: a patch commit, a `STOCK_CLI_SHA256` re-pin commit, a plain build (no test-hooks),
  `router_conformance.py` at threads 1 and 2 with `--stock-cli` = the new tag's stock binary, and the gate
  and bench of the mode (`--quick`: none; default: the fast `pcbkit-gate.sh`; `--full`: the gate plus the bench
  subcommands `ignored`, `determinism`, `upstream`, `parity-route`, with `PCBKIT_BASE=<tag>`), then a PASS/FAIL table. Exit 0 only if every executed step passed.

It only reads `pcbkit`, writes only its own worktree and branch, and never pushes (`--fetch` is the only
network access and is opt-in). `--resume` reruns the build/conformance/gate part in an existing worktree
(use it to split `--full` into several foreground calls); `--force` recreates a script-made worktree.
Needs `PCBKIT_CONFORMANCE`; `CARGO_SLOT` defaults to `cargo`.

Dry runs on `pcbkit` 0e27db7 (F1/F2/H7 merged), 2026-10-09:

| target | result | time |
|---|---|---|
| v0.1.12 `--quick` | 61 files, **0 conflicts** (the upstream drift touches hook files `main.rs`, `autorouter.rs`, `fanout.rs`, `pipeline/mod.rs`, but never the same hunks); build OK; conformance t1 and t2 PASS; exit 0 | 1 m 49 s |
| v0.1.12 default (resume) | build, conformance t1/t2 PASS, `tests-core` PASS; exit 0 | 3 m 39 s |
| v0.1.11 default | **1 conflict**: `crates/fr-engine/src/pipeline/autorouter.rs` lines 1115-1119, hook site **H7** (deterministic guard on the slow-pass stagnation stop; v0.1.13 added `&& current_pass >= min_passes` to that same line, which v0.1.11 lacks); exit 1, worktree left for a human merge | 44 s |

So v0.1.12 matches the drill above (clean). v0.1.11 no longer applies cleanly, which differs from the drill
earlier in this section: the drill ran before F1/H7 existed, and H7 edits a line that upstream changed
between v0.1.11 and v0.1.13. The rest of the drill text stays valid for the patch set it measured.
For a tag newer than v0.1.13, conflicts can only appear in the drift list the script prints.

### Gate result on the drill branch (`pcbkit-rebase-drill`, tip == `pcbkit` 135cc5d)

`pcbkit-gate.sh` phases (historical: the phase machinery was removed 2026-10-09 when the gate became unit tests only, the
slow parts now being `scripts/pcbkit-bench.sh`), each in the foreground, on the loaded box:

| phase | result | wall |
|---|---|---|
| tests-core | PASS, 239 tests | 5 m 02 s |
| tests-serve-a | PASS, 38 | 3 m 17 s |
| tests-serve-b | PASS, 6 | 4 m 26 s |
| tests-serve-c | PASS, 7 | 6 m 06 s |
| tests-serve-d | PASS, 5 | 0 m 47 s |
| tests-serve-e | PASS, 8 | 4 m 16 s |
| final | PASS: coverage 303 tests, ledger, parity-route (Java vs Rust SES identical), stock-pin, contract-pin 3fc599c, conformance t1/t2, plain-bin + plain conformance t1 | 4 m 01 s |

Total about 28 minutes. `pcbkit-ab.sh --quick` old (`135cc5d` build) vs rebased build, threads 1 and 4:
**AB: ALL SAME** (all rows SAME or SAME-ERR; 2 m 04 s).

### Stress drill: patch set on v0.1.12 (`pcbkit-rebase-drill-v0.1.12`, one squashed commit)

Builds (1 m 24 s), `router_conformance.py` t1 and t2 exit 0, `tests-core` PASS (239 tests, 3 m 24 s),
`pcbkit-ab.sh --quick` against the v0.1.13 based fork at t1 and t4: ALL SAME. The three upstream
commits between the tags do not touch a hook site; a real upstream change to
`pipeline/autorouter.rs`, `autoroute/maze.rs` or `basic_board.rs` (the three largest hook sites,
+49, +41, +77 lines) is where a human merge would be needed.

## 2. Thread and determinism sweep (`scripts/pcbkit/sweep.py`)

`sweep.py` is stdlib only and speaks the protocol directly. Per board, per thread count
(1, 2, 4, 8), per seed (0, 1, 7), it starts **two separate server processes** (hello, load by path,
`route` from scratch `nets: all`, export) and compares the result (minus `wall_ms`) and the SES
bytes. Seed 0 is also compared with the pinned stock v0.1.13 CLI (`--no-time-limits`, both thread
flags set to N). Pools are sized so concurrent routes x threads stay within 24 threads.

```
scripts/pcbkit/sweep.py --server target/release/fastroute --stock $PCBKIT_STOCK_CLI --corpus core|full [--shard i/n] [--equiv-only] --json out.json
scripts/pcbkit/sweep.py --summarize out1.json out2.json ...      # exit 1 on any bad row
```

### Coverage and outcome

| run | boards | rows | what | bad |
|---|---|---|---|---|
| determinism, core corpus (`testdata/dsn` + `det/*` + `protocol-fixtures`) | 9 | 144 | 2 processes per row, t1/2/4/8 x seeds 0,1,7 (+ seed 0 from current), seed 0 vs stock | **0** |
| determinism, 2 shards of the full corpus (`runs/`) | 4 more | 96 | same | **0** |
| stock equivalence, **whole corpus** (`full`, 66 distinct DSNs) | 66 | 528 | seed 0 `from: scratch` and `from: current`, t1/2/4/8, vs stock CLI | **0** |

* Determinism: identical results and SES between two processes at every fixed thread count on all
  240 pair rows. Seeds 1 and 7 differ from seed 0 on 63 of 112 (board, threads, seed) rows (they
  select other variants, as intended), and are deterministic per seed.
* Stock equivalence: all 528 rows equal the stock CLI SES, with the two qualifications below.
  `synth_pcb_keepout.dsn` is rejected identically by both (`SAME-ERR`). One row (energy-6, t4,
  seed 0 current) differed once under load with the wall-clock optimizer rule firing (finding F1)
  and was SAME when re-run with one job.
* The determinism pairs were run for 13 of the 66 boards (the heavy ones included: hb200,
  energy-12 variants); the other 53 boards were checked for stock equivalence only. At about 15 s a
  route, pairs over all 66 boards at four thread counts and three seeds take several hours on the
  shared box. The remaining shards can be run with `--shard i/24` (the exact command is above).

### Findings

* **F1 (upstream, not fixed): determinism breaks under CPU oversubscription.** With enhancements on
  (the default, also in the stock CLI), `optimizer.rs` `apply_greedy` stops on a wall-clock budget
  (`start.elapsed() >= budget`, logged "time budget used") and `pipeline/mod.rs:238` gives the
  optimizer a wall-clock `default_budget`; `--no-time-limits` does not cover them. SPEC section 7
  says no wall-clock stop rule may change a result. Reproduced on energy-12 `pfull` (t8, seed 1): 2
  of 14 concurrent runs differed; the **stock CLI** shows the same (2 of 14), `--parity` is
  deterministic (14 of 14). Sweep policy: a differing pair whose log contains "time budget used"
  or "Optimizer stage timed out" is `LOAD-DIFF`, reported, not failed; any other difference fails.
  Consequence for the placer: do not run more router threads than cores in parallel, or the route
  cache key (which assumes determinism) is violated. A fork fix (disable these two budgets in
  `serve` and when `--no-time-limits` is set) is about 10 lines in two core files; not done here
  (scope, and the core patch budget).
* **F2 (fork bug, FIXED in pcbkit-fix-move-route-livelock-mt, see docs/PCBKIT.md "F2"; text below is the original finding): `route` after `move` can livelock at threads >= 2.**
  Repro: `sweep.py --server <bin> --stock x --threads 4 --probe-carry-hang
  reference/pcbkit-corpus/det/energy-12-1/board.dsn R2 C18` prints `HANG`; at `--threads 1` it
  prints `ok`. Sequence: scratch route, `move` R2 (+1000 units), `move` C18, `route all from
  current`. The server never answers: a worker thread panics in
  `crates/fr-engine/src/board/search_tree.rs:773` (`rooms[&key]`, "no entry found for key") inside
  `AutorouteEngine.complete_expansion_room`, which is caught and retried (millions of log lines).
  Also hangs with `carry: none` (heuristic-baseline-1: R7, route, C7, route), and one move plus one
  route is fine. A scratch route after the same moves is fine. So a stale expansion room survives
  a move in the multi-thread path (board clone). Impact: a placer must not chain `move` plus
  `route(current)` at threads >= 2 until fixed; `route from: scratch`, or `nets: [ripped]`
  (the incremental path, which was used for the timings and worked), are unaffected in these
  tests. Owner: fork `move` hook (`place_component`/`remove_items` rooms invalidation).
* **F3 (spec reading, not a bug): scratch is not stock when the DSN carries unfixed wiring.** 20 of
  the rows (synth_order_placed, synth_rules, a few corpus boards) differ between `from: scratch`
  and the stock CLI because the CLI resumes the DSN's unfixed wires and scratch drops them
  (SPEC 5.5). `from: current` equals stock on all of them, so the sweep reports these as
  `SAME-RESUME`. The conformance fixtures have no such wiring, which is why SPEC 5.5 holds there.

## 3. Server vs CLI speed (t4, one board per row, wall seconds)

Cold-start comparison, one full route (spawn + hello + load + route + export in one session vs one
CLI run): energy-12-1 3.63 vs 3.61, heuristic-baseline-1 12.17 vs 12.65, hb200 12.33 vs 12.44, i.e.
the server costs nothing extra for a single route (within noise).

Moves, `carry: none`, 0.1 mm step on parts whose move rips a net (found on a snapshot, untimed).
"CLI" = a full CLI reload per move; "serve" = one session, `move` then `route` of the ripped nets
only (`incremental`), session time includes the initial full route:

| board | moves | CLI reloads | serve session (incl. initial route) | per-move route in serve | speed-up on the moves |
|---|---|---|---|---|---|
| energy-12-1 | 4 | 24.19 s | 7.51 s (3.63 initial) | 0.85, 0.79, 1.25, 0.82 s | 6.2x |
| heuristic-baseline-1 | 3 | 52.72 s | 27.04 s (12.17 initial) | 5.49, 2.24, 1.43 s | 3.5x |
| hb200 | 3 | 47.51 s | 24.13 s (12.33 initial) | 5.24, 2.47, 1.40 s | 3.3x |

Routing the **whole** board from current after each move (`nets: all`) is no faster than reloading
(energy-12-1: 21.89 s vs 24.19 s) and, on the two larger boards, hit F2 after the first move.
Why 4 or 3 moves, not 5: only that many front parts rip a net on these boards at 0.1 mm; larger
steps push parts into neighbours and unroute the board.

## 4. Files

`scripts/pcbkit/sweep.py` (new, `--self-check` for its helpers), this document. Unavoidable edits
outside that scope: `scripts/pcbkit-gate.sh` and the matching sentence in `docs/PCBKIT.md` now
treat `scripts/pcbkit/*` and `docs/PCBKIT-*.md` as ledger-exempt (they are fork tooling, not core
hooks); the core-files INFO row ignores them too, so the core patch budget number is unchanged.
