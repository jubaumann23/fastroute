# fastroute

[![Sponsor](https://img.shields.io/badge/Sponsor-%E2%9D%A4-db61a2?logo=githubsponsors&logoColor=white)](https://github.com/sponsors/parisxmas)
[![Open Collective](https://img.shields.io/badge/Open%20Collective-support-7FADF2?logo=opencollective&logoColor=white)](https://opencollective.com/fastroute)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

**PCB autorouter for KiCad and Specctra DSN** — a Rust port of the
[Freerouting](https://github.com/freerouting/freerouting) PCB autorouter.
(Not related to the FastRoute global router for IC design used in OpenROAD.)

Same algorithms, same input/output (Specctra `.dsn` → `.ses`), same
command line — without Java, several times faster and with a fraction of the memory.

- **Exact mode** (`--parity`) produces SES output byte-identical to Freerouting's
  deterministic mode (source commit `aa909a3`, single-threaded optimizer, count-based
  time limits) on all 20 benchmark boards.
- **Default mode** runs the optimizer in parallel; results are deterministic for any
  thread count.
- **KiCad plugin** in [`integrations/kicad`](integrations/kicad/README.md).

## What is different from Freerouting

The bug fixes and routing improvements are on by default; `--no-enhancements` switches them
off (the DSN parser fixes stay), `--parity` gives byte-identical Freerouting results. The new
features are used when asked for (an option, or a rule in the KiCad board). Details,
measurements and the reasoning for each change: [docs/IMPROVEMENTS.md](docs/IMPROVEMENTS.md).

### Bugs fixed

| Freerouting behaviour | fastroute |
|---|---|
| The optimizer never runs after the autorouter stopped itself: the stop request stays set, so every optimizer candidate fails to route | The router's own stop is withdrawn before the optimizer |
| When the stagnation rule stops the autorouter, the final tail removal is skipped and unused fanout escapes (via + stub) stay on the board | Tails are removed in every case |
| The optimizer's score is clamped at 0; on boards whose score is below 0 (e.g. DAC2020_bm01) no candidate can ever be accepted | Unclamped scores are compared (bm01: −182 → 107) |
| A connection that ends in a via into its net's plane layer is rejected ("layers disabled"), so plane nets only reach the plane through fanout | Such connections are accepted |
| `ignore_net_classes` is honoured by the autorouter but not by the fanout, which still adds stubs to the ignored nets | No fanout on ignored classes |
| A failing connection rips its whole net on every failure from the second on; unroutable connections make large nets (GND, 3V3) oscillate | Only once per net |
| Class-pair clearances from the DSN only reach the clearance classes named after the net classes, not the pin/SMD classes of the same nets | Applied to all item classes of both net classes |
| DSN lexer: non-ASCII letters dropped from names, desynchronisation on some quoted strings, a pin token swallowing a character of the next pin | Parsed correctly |

### Routing and optimizer improvements

- **Optimizer on partly routed boards** (Freerouting skips it as soon as one connection is
  unrouted); **greedy passes** that apply every improving candidate instead of one per pass;
  re-routing only the ripped item's nets; skipping connections that were unrouted from the
  start; a time budget equal to the routing time.
- **Parallel autorouting passes** (`router.autorouter.max_threads`) and a **parallel
  optimizer**, deterministic for a given thread count.
- **Multi-start routing** (`--multi-start`): if connections stay unrouted, the autorouter is
  rerun with shuffled orders in parallel and the best result is kept.
- **Undo of bad passes**, stagnation stop for slow passes, fanout that stops when it only
  re-fans, minimum track width for neck-down (`router.min_trace_width_um`).
- **Checkpoints and time limits**: the session file always holds the best board so far;
  `--max-time`, Ctrl+C / SIGTERM stop the run and keep the result.

### New features

- **Length matching** (`--tune`): groups of nets (buses, pair skew) are matched with
  meanders after routing; every meander is clearance-checked, via heights count as in KiCad.
- **Controlled impedance**: trace widths per layer from the board stackup (microstrip,
  stripline, differential), no neck-down for those classes (`--no-neckdown-classes`).
- **Differential pairs** (`--pairs`): pairs are routed first with the second net following
  the first at the pair gap, held while the rest of the board is routed.

### KiCad plugin

The plugin ([`integrations/kicad`](integrations/kicad/README.md)) exports the board, fills
the gaps of KiCad's Specctra export and imports the result:
- `.kicad_dru` rules are carried over: class clearances, per-layer track widths, length and
  skew rules, differential pair gaps; minimum track width and copper-to-edge clearance too.
- Zones, copper texts and rule areas are exported correctly (KiCad exports every zone as a
  plane and every rule area as a keepout); zones are refilled after routing.
- `impedance_cli.py` computes controlled-impedance widths and writes the DRC rules.
- Checked with KiCad's own DRC on the KiCad demo boards (see the results table in
  [docs/IMPROVEMENTS.md](docs/IMPROVEMENTS.md)).

## Build and run

```sh
cargo build --release -p fastroute          # or scripts/build-pgo.sh (~15 % faster)
target/release/fastroute -de board.dsn -do board.ses
```

The session file (`-do`) is rewritten with the best board so far whenever routing or
optimizing improves, so a run that is stopped or killed still leaves its best result.
Ctrl+C, SIGTERM and Ctrl+Break stop the run and write the best result (a second signal exits
at once). `fastroute --help` lists the options and the common `--router.*` settings.

Windows x64 build (cross-compiled with MinGW-w64): `scripts/build-windows.sh` →
`dist/windows/fastroute.exe`. Linux x64 and arm64 builds (cargo-zigbuild, glibc 2.17):
`scripts/build-linux.sh` → `dist/linux-*/fastroute`. The KiCad package can bundle several
platforms: `integrations/kicad/package.sh --bin <macos binary> macos-arm64 --bin
dist/windows/fastroute.exe windows-x64 --bin dist/linux-x64/fastroute linux-x64 ...`.

Freerouting's options are accepted: `-mp <passes>`, `--router.<path>=<value>` (e.g.
`--router.optimizer.enabled=false`, `--router.scoring.via_costs=80`); GUI/API options such
as `--gui.enabled` are ignored. fastroute adds:

| Option | Meaning |
|---|---|
| `--parity` | Exact Freerouting results: java-compatible optimizer, count-based time limits |
| `--optimizer-mode=java-compat\|parallel` | Optimizer strategy (default: parallel if more than one thread) |
| `--time-limit-mode=wall\|count\|disabled`, `--time-limit-factor=N` | How Freerouting's internal time limits are evaluated |
| `--router.min_trace_width_um=<w>` | Never neck traces down below this width |
| `--multi-start=N` | Rerun the autorouter with N−1 shuffled orders in parallel if connections remain unrouted (default 4) |
| `--no-enhancements` | Freerouting's behaviour without fastroute's routing/optimizer improvements (see [docs/IMPROVEMENTS.md](docs/IMPROVEMENTS.md)) |
| `--router.autorouter.max_threads=N` | Threads of the parallel autorouting pass (default: all cores; 1 = sequential pass) |
| `--tune=FILE` | Length matching after routing: groups of nets matched with meanders (see `fastroute --help`) |
| `--layer-heights=MM,..` | Height of each copper layer (stackup), so that vias count in length matching |
| `--pairs=FILE` | Differential pairs: routed first, the second net following the first at the pair gap |
| `--no-neckdown-classes=A,B` | Keep the full trace width of these classes at pins (controlled impedance) |
| `--max-time=SECONDS` | Stop after this time and write the best result so far |
| `-V`, `--version` | Print the version |
| `-v` | Verbose progress |

## Performance

Full pipeline (autorouter + optimizer), exact mode, 20 benchmark boards
([docs/PERFORMANCE.md](docs/PERFORMANCE.md)):

| | Freerouting (Java) | fastroute | fastroute + PGO |
|---|---|---|---|
| Sum of 20 boards | 2102 s | 510 s | 420 s |
| DAC2020_bm01 | 960 s | 380 s | 310 s |
| CM5_MINIMA_3 | 410 s | 23 s | 20 s |
| Peak memory (typical) | 1–2 GB | 20–300 MB | |

The parallel optimizer mode is another 1.5–3× faster on most boards.

## Layout

| Crate | Contents |
|---|---|
| `fr-dsn` | Specctra S-expression reader and typed DSN model |
| `fr-geom` | Planar geometry (port of `geometry.planar`) |
| `fr-jcompat` | Bit-exact emulation of the Java library behaviour the router depends on |
| `fr-settings` | Router settings, defaults, CLI/DSN settings sources |
| `fr-engine` | Board model, search trees, autorouter, pull-tight/shove/optimizer, DRC, scoring, pipeline |
| `fr-io` | DSN → board loader, post-load processing, SES writer/reader |
| `fastroute` | Command-line tool |

How the port was done and how parity is tested: [docs/PORTING.md](docs/PORTING.md).
The Java reference build and scripts: `scripts/java-parity.sh`, `scripts/parity-route.sh`.

## Support

fastroute is developed in spare time. If it saves you routing hours or a Java setup, you can
support its development through [GitHub Sponsors](https://github.com/sponsors/parisxmas) or
[Open Collective](https://opencollective.com/fastroute). Bug reports with a board that shows
the problem (a `.dsn` file is enough) are just as welcome.

## License

GPL-3.0-or-later ([LICENSE](LICENSE)). fastroute is a derivative work of Freerouting
(GPL-3.0); all credit for the routing algorithms goes to the Freerouting authors. See
[NOTICE](NOTICE) for the attribution, the Freerouting version it is based on, and the
third-party crates.
