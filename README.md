# fastroute

A Rust port of the [Freerouting](https://github.com/freerouting/freerouting) PCB
autorouter. Same algorithms, same input/output (Specctra `.dsn` → `.ses`), same
command line — without Java, several times faster and with a fraction of the memory.

- **Exact mode** (`--parity`) produces SES output byte-identical to Freerouting's
  deterministic mode (source commit `aa909a3`, single-threaded optimizer, count-based
  time limits) on all 20 benchmark boards.
- **Default mode** runs the optimizer in parallel; results are deterministic for any
  thread count.
- **KiCad plugin** in [`integrations/kicad`](integrations/kicad/README.md).

## Build and run

```sh
cargo build --release -p fastroute          # or scripts/build-pgo.sh (~15 % faster)
target/release/fastroute -de board.dsn -do board.ses
```

Windows x64 build (cross-compiled with MinGW-w64): `scripts/build-windows.sh` →
`dist/windows/fastroute.exe`. The KiCad package can bundle several platforms:
`integrations/kicad/package.sh --bin <macos binary> macos-arm64 --bin dist/windows/fastroute.exe windows-x64`.

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

## License

GPL-3.0-or-later. fastroute is a derivative work of Freerouting (GPL-3.0); all credit for
the routing algorithms goes to the Freerouting authors.
