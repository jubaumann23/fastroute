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
  --no-fail-fast` via `$CARGO_SLOT` (default: the shared cargo_slot.sh), fails on any skipped parity
  test; Java-vs-Rust SES identity on bm02 and pic_programmer (`scripts/parity-route.sh`); when the
  binary has `serve`, `router_conformance.py` at `--threads 1` and `2` (path override:
  `PCBKIT_CONFORMANCE`; `--stock-cli` passed if the runner supports it). Prints a PASS/FAIL table.
  Last result on the unmodified branch: 213 tests executed, 0 skipped, parity routes identical,
  conformance SKIP (no `serve`), exit 0 (about 40 s warm).
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
  NaN with the sign bit set, while the vectors come from Java's `Double.doubleToLongBits`, which
  collapses every NaN to `0x7ff8000000000000`. The test now canonicalizes NaN the same way
  (`double_to_long_bits`); all other comparisons are unchanged.
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

## PATCH LEDGER

Every change to upstream-owned files. Hooks are default off: with the flag unset the output is
byte-identical to upstream (`pcbkit-ab.sh` proves it).

| Hook | File:line | Lines changed | Default-off flag | Why |
|---|---|---|---|---|
| H0 serve dispatch | `crates/fastroute/src/main.rs` (line set when merged) | TBD | `serve` subcommand only | route `fastroute serve` to the server crate |
| H1 move | TBD | TBD | via protocol `move` only | apply part moves to the loaded board |
| H2b keep-fixed-on-split | TBD | TBD | via protocol locks only | locked wires survive trace splitting |
| H3 blockers | TBD | TBD | via protocol `blockers` only | report which items block a connection |
| H5 order seed | TBD | TBD | seed unset = upstream order | deterministic net order per request seed |
| H6 net mask | TBD | TBD | mask unset = all nets | route only a subset of nets |
| T1 test-only | `scripts/parity-route.sh`, `crates/fr-jcompat/tests/jdk_vectors.rs` | about 17 and 12 | n/a | Linux support, see above |
