# fastroute for KiCad

A KiCad PCB editor plugin that autoroutes the board with `fastroute` — no Java
needed. It uses KiCad's built-in Specctra exchange (export DSN, route, import
SES) and adds a few steps that make the result DRC-clean in KiCad.

## Install

1. Build the package: `integrations/kicad/package.sh` (add `--pgo` for a
   ~15 % faster binary). This writes `dist/fastroute-kicad-<version>.zip` with
   the `fastroute` binary for the current platform bundled.
2. In KiCad: **Plugin and Content Manager → Install from File…** and pick the zip.
3. The action appears under **Tools → External Plugins → fastroute autorouter**
   (and as a toolbar button).

The plugin finds `fastroute` in this order: `$FASTROUTE_BIN`, the bundled
`plugins/bin/<platform>/`, `PATH`.

## Options

| Option | Default | What it does |
|---|---|---|
| Mode | Fast | *Fast*: parallel optimizer. *Exact*: bit-identical to Freerouting's deterministic mode (single-threaded optimizer). *Quick*: autorouter only. |
| Max. passes | 0 | Autorouter pass limit (0 = until done/stagnant). |
| Remove existing tracks | off | Deletes unlocked tracks/vias first; locked ones are kept. |
| Route zone nets with tracks | on | KiCad exports copper zones as Specctra *planes* covering the zone outline, so the router assumes every pad of the zone's net is connected. The refilled zone leaves clearance around the new tracks and can cut pads off. With this option the planes are removed from the exchange file and those nets are routed too. |
| Keep tracks away from copper texts | on | KiCad's export omits copper texts; each glyph is added as a keepout. |
| Respect minimum track width | on | Uses *Board Setup → Constraints → Minimum track width*: no neck-down below it (`--router.min_trace_width_um`, a fastroute extension). |
| Refill zones | on | Refills all zones after importing the result. |

## Headless use

`plugins/route_cli.py` routes a `.kicad_pcb` file without the GUI. Run it with
KiCad's Python (it needs the `pcbnew` module):

```sh
PY=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3
$PY integrations/kicad/plugins/route_cli.py board.kicad_pcb -o routed.kicad_pcb --clear
```

`tests/e2e.sh` routes the KiCad demo boards this way and runs KiCad's DRC on
the result (optionally side by side with Freerouting via `FREEROUTING_CMD`).

## Results (KiCad 10.0.6, demo boards, tracks removed and re-routed)

Unconnected items / routing-related DRC violations (clearance, track width,
shorts, hole clearance, thermals, copper-to-edge) reported by KiCad's DRC:

| Board | Original (hand-routed) | Freerouting plugin flow | fastroute plugin |
|---|---|---|---|
| pic_programmer | 0 / 8 | 0 / 118 (3.9 s) | **0 / 0** (0.7 s) |
| interf_u | 0 / 3 | 8 / 346 (28.8 s) | **0 / 1** (3.3 s) |
| ecc83-pp | 0 / 0 | 0 / 6 (2.8 s) | **0 / 0** (0.3 s) |
| sonde xilinx | 0 / 38 | 1 / 122 (4.4 s) | **0 / 36** (0.4 s) |
| complex_hierarchy | 0 / 1 | 83 / 3 (3.4 s) | 13 / 3 (5.3 s) |

The remaining sonde violations are pads placed too close to the board edge in
the original design; complex_hierarchy is a hard board for this router
(Freerouting leaves 86 connections unrouted on the same export).

## Known limitations

- Uses the classic `pcbnew` Python action-plugin API (KiCad 7–10). The KiCad 10
  IPC plugin API has no Specctra export yet.
- Specctra carries no differential-pair or length-tuning rules; those nets are
  routed as ordinary nets.
