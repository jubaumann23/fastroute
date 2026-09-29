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
| Route zone nets with tracks | on | KiCad exports copper zones as Specctra *planes* covering the zone outline, so the router assumes every pad of the zone's net is connected; the refilled zone may cut pads off. Pours are removed and their nets routed with tracks. A power-type layer with a plane covering most of the board stays a plane layer (pads connect through vias); other power-type layers are routed like signal layers. |
| Keep tracks away from copper texts | on | KiCad's export omits copper texts; each glyph is added as a keepout. |
| Respect minimum track width | on | Uses *Board Setup → Constraints → Minimum track width*: neck-down stops there, and connections that do not fit are retried with traces of that width. The board's copper-to-edge clearance is passed on as well. |
| Refill zones | on | Refills all zones after importing the result. |

## Headless use

`plugins/route_cli.py` routes a `.kicad_pcb` file without the GUI. Run it with
KiCad's Python (it needs the `pcbnew` module):

```sh
PY=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3
$PY integrations/kicad/plugins/route_cli.py board.kicad_pcb -o routed.kicad_pcb --clear
# with a time limit (keeps the best result) and extra fastroute settings after "--"
$PY integrations/kicad/plugins/route_cli.py board.kicad_pcb -o routed.kicad_pcb --max-time 3000 \
    -- --router.autorouter.ignore_net_classes=GUC
```

From Python, `core.Router(board, extra_args=..., max_time=seconds)` does the same.
fastroute rewrites its session file with the best board so far after every
improvement; if the process ends abnormally (killed, crashed) the router still
imports that file and sets `result.partial`.

Length matching: if the board's `.kicad_dru` has `skew` or `length (min …)` rules for net
classes or net names, the plugin passes them to fastroute, which adds meanders after routing
(see docs/IMPROVEMENTS.md). Example:

```
(rule "SDRAM data"
  (condition "A.NetName == '/SD_D*' || A.NetName == '/SD_NBL*'")
  (constraint skew (max 0.5mm)))
```

Controlled impedance: `impedance_cli.py` computes trace widths for target
impedances from the board's stackup and, with `--write`, adds per-layer
`track_width` rules to the `.kicad_dru`. The plugin then routes those classes
with the per-layer widths and without neck-down:

```
$PY integrations/kicad/plugins/impedance_cli.py board.kicad_pcb --class RF=50 --class USB=90d --write
```

A target ending in `d` is differential (`--gap` sets the pair gap, default
equal to the width). Solder mask is not included; check the widths against
your fabricator's stackup.

Zones: by default a zone on a *power*-type layer covering at least half of the
board stays a plane (pads connect with vias, no tracks on that layer); other
zones are removed for routing and their nets are routed with tracks, then the
zones are refilled. `--zones-as-planes` (`route_zone_nets=False`) keeps every
zone as a plane instead, like the plain Freerouting flow.

`tests/e2e.sh` routes the KiCad demo boards this way and runs KiCad's DRC on
the result (optionally side by side with Freerouting via `FREEROUTING_CMD`).

## Results

See [docs/IMPROVEMENTS.md](../../docs/IMPROVEMENTS.md) for the full table.
KiCad 10 DRC after re-routing from scratch (unconnected items / routing violations):

| Board | Original design | Plain Freerouting flow | fastroute plugin |
|---|---|---|---|
| lora_node (4 layers, 445 SMD pads) | 0 / 0 | — | 1 / 0 (53 s incl. optimizer) |
| multichannel_mixer | 0 / 0 | 160 unrouted (benchmark DSN) | 0 / 0 (3.3 s) |
| complex_hierarchy | 0 / 1 | 83 / 3 | 0 / 0 (0.7 s) |
| interf_u | 0 / 3 | 8 / 346 | 0 / 2 (5.6 s) |
| pic_programmer | 0 / 8 | 0 / 118 | 0 / 0 (0.7 s) |
| ecc83-pp | 0 / 0 | 0 / 6 | 0 / 0 (0.3 s) |
| sonde xilinx | 0 / 38 | 1 / 122 | 0 / 36 (0.4 s) |

## Known limitations

- Uses the classic `pcbnew` Python action-plugin API (KiCad 7–10). The KiCad 10
  IPC plugin API has no Specctra export yet.
- Differential pairs are routed as two ordinary nets: they get the computed
  width, but no coupled routing and no enforced gap.
