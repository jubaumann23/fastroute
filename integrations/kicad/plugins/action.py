"""PCB editor action: Tools > External Plugins > fastroute autorouter."""

import json
import logging
import threading
from pathlib import Path

import pcbnew
import wx

from . import core

SETTINGS_FILE = Path(__file__).resolve().parent / "settings.json"

MODES = [
    ("fast", "Fast (parallel optimizer)"),
    ("exact", "Exact (identical to Freerouting's deterministic mode)"),
    ("quick", "Quick (autorouter only, no optimizer)"),
]

DEFAULTS = {
    "mode": "fast",
    "max_passes": 0,
    "clear_tracks": False,
    "keep_existing": False,
    "route_zone_nets": True,
    "text_keepouts": True,
    "respect_min_width": True,
    "refill_zones": True,
    "keep_stitching": True,
    "live_view": False,
}


def load_settings():
    try:
        data = json.loads(SETTINGS_FILE.read_text(encoding="utf-8"))
        return {**DEFAULTS, **{k: data[k] for k in DEFAULTS if k in data}}
    except Exception:
        return dict(DEFAULTS)


def save_settings(settings):
    try:
        SETTINGS_FILE.write_text(json.dumps(settings, indent=2), encoding="utf-8")
    except OSError:
        pass


class SettingsDialog(wx.Dialog):
    def __init__(self, parent, settings):
        super().__init__(parent, title="fastroute autorouter")
        s = settings
        box = wx.BoxSizer(wx.VERTICAL)
        grid = wx.FlexGridSizer(0, 2, 8, 8)

        grid.Add(wx.StaticText(self, label="Mode:"), 0, wx.ALIGN_CENTER_VERTICAL)
        self.mode = wx.Choice(self, choices=[label for _, label in MODES])
        self.mode.SetSelection([k for k, _ in MODES].index(s["mode"]))
        grid.Add(self.mode, 1, wx.EXPAND)

        grid.Add(wx.StaticText(self, label="Max. passes (0 = no limit):"), 0, wx.ALIGN_CENTER_VERTICAL)
        self.passes = wx.SpinCtrl(self, min=0, max=10000, initial=s["max_passes"])
        grid.Add(self.passes, 1, wx.EXPAND)
        box.Add(grid, 0, wx.ALL | wx.EXPAND, 12)

        def check(label, key, tip):
            cb = wx.CheckBox(self, label=label)
            cb.SetValue(bool(s[key]))
            cb.SetToolTip(tip)
            box.Add(cb, 0, wx.LEFT | wx.RIGHT | wx.BOTTOM, 12)
            return cb

        self.clear = check(
            "Remove existing unlocked tracks and vias first", "clear_tracks",
            "Locked tracks and vias are always kept and routed around.",
        )
        self.keep = check(
            "Keep all existing tracks and vias unchanged (route only open connections)",
            "keep_existing",
            "Otherwise unlocked tracks may be moved, shortened or rerouted. "
            "Ignored when existing tracks are removed.",
        )
        self.zone_nets = check(
            "Route nets of copper zones with tracks", "route_zone_nets",
            "Recommended. Otherwise pads are assumed to be connected by the zone, "
            "which the refilled zone may not guarantee.",
        )
        self.text_keepouts = check(
            "Keep tracks away from texts on copper layers", "text_keepouts",
            "KiCad's Specctra export omits copper texts.",
        )
        self.min_width = check(
            "Respect the board's minimum track width (neck-down stops there)",
            "respect_min_width",
            "Board Setup > Constraints > Minimum track width.",
        )
        self.stitching = check(
            "Keep zone stitching vias", "keep_stitching",
            "Vias that no track touches (GND stitching, thermal vias) are kept where they "
            "are, also with the tracks removed. Otherwise the router treats them as loose "
            "ends and removes them.",
        )
        self.refill = check("Refill zones after routing", "refill_zones", "")
        self.live = check(
            "Show the routing live in the web browser", "live_view",
            "Opens a local page (http://127.0.0.1:7878) with the board, the progress of every "
            "pass and the log while fastroute routes. Routing results are not affected.",
        )

        buttons = self.CreateStdDialogButtonSizer(wx.OK | wx.CANCEL)
        self.FindWindowById(wx.ID_OK).SetLabel("Route")
        box.Add(buttons, 0, wx.ALL | wx.ALIGN_RIGHT, 12)
        self.SetSizerAndFit(box)

    def values(self):
        return {
            "mode": MODES[self.mode.GetSelection()][0],
            "max_passes": self.passes.GetValue(),
            "clear_tracks": self.clear.GetValue(),
            "keep_existing": self.keep.GetValue(),
            "route_zone_nets": self.zone_nets.GetValue(),
            "text_keepouts": self.text_keepouts.GetValue(),
            "respect_min_width": self.min_width.GetValue(),
            "refill_zones": self.refill.GetValue(),
            "keep_stitching": self.stitching.GetValue(),
            "live_view": self.live.GetValue(),
        }


class ProgressDialog(wx.Dialog):
    """Shows fastroute's progress lines while the router runs in a thread."""

    def __init__(self, parent, router):
        super().__init__(parent, title="fastroute: routing...", size=(620, 320),
                         style=wx.DEFAULT_DIALOG_STYLE | wx.RESIZE_BORDER)
        self.router = router
        self.result = None
        box = wx.BoxSizer(wx.VERTICAL)
        self.status = wx.StaticText(self, label="Exporting board...")
        box.Add(self.status, 0, wx.ALL | wx.EXPAND, 10)
        self.log = wx.TextCtrl(self, style=wx.TE_MULTILINE | wx.TE_READONLY | wx.HSCROLL)
        box.Add(self.log, 1, wx.LEFT | wx.RIGHT | wx.EXPAND, 10)
        buttons = wx.BoxSizer(wx.HORIZONTAL)
        self.live_url = None
        self.live = wx.Button(self, label="Open live view")
        self.live.Bind(wx.EVT_BUTTON, lambda _e: self.live_url and wx.LaunchDefaultBrowser(self.live_url))
        self.live.Hide()
        buttons.Add(self.live, 0, wx.RIGHT, 8)
        self.cancel = wx.Button(self, wx.ID_CANCEL, "Stop")
        self.cancel.Bind(wx.EVT_BUTTON, self.on_cancel)
        buttons.Add(self.cancel, 0)
        box.Add(buttons, 0, wx.ALL | wx.ALIGN_RIGHT, 10)
        self.SetSizer(box)
        self.Bind(wx.EVT_CLOSE, self.on_cancel)

    def start(self):
        """Exports on the GUI thread, routes in a worker, imports on the GUI thread."""
        failed = self.router.prepare()
        if failed is not None:
            self.result = failed
            return wx.ID_OK
        threading.Thread(target=self._work, daemon=True).start()
        self.ShowModal()
        if self.result is not None:
            self.result = self.router.finish(self.result)
        return wx.ID_OK

    def _work(self):
        def on_line(level, text):
            wx.CallAfter(self._append, level, text)

        result = self.router.route(on_line)
        wx.CallAfter(self._done, result)

    def _append(self, level, text):
        self.log.AppendText(f"{level:5} {text}\n")
        url = core.live_url(text)
        if url and not self.live_url:
            # fastroute opens the page itself; the button opens it again
            self.live_url = url
            self.live.Show()
            self.Layout()
        if "pass #" in text or "stage" in text:
            self.status.SetLabel(text[:110])

    def _done(self, result):
        self.result = result
        self.EndModal(wx.ID_OK)

    def on_cancel(self, _event):
        self.status.SetLabel("Stopping...")
        self.router.cancel()


class FastrouteAction(pcbnew.ActionPlugin):
    def defaults(self):
        self.name = "fastroute autorouter"
        self.category = "Routing"
        self.description = "Autoroute the board with fastroute (a fast Rust port of Freerouting)"
        self.show_toolbar_button = True
        icon = Path(__file__).resolve().parent / "icon_24x24.png"
        if icon.is_file():
            self.icon_file_name = str(icon)

    def Run(self):
        """Shows the options, then routes after Run() has returned.

        KiCad snapshots the board items around Run() to build an undo entry;
        the session import deletes and recreates all tracks, which would
        leave that snapshot with dangling pointers (and crash the editor).
        So all board changes happen in a CallAfter, outside that bracket.
        """
        log = logging.getLogger("fastroute")
        log.info("run requested")
        try:
            parent = wx.GetActiveWindow()
            if core.find_binary() is None:
                wx.MessageBox(
                    "The fastroute executable was not found.\n\nPut it in the plugin's bin/ "
                    "directory, on PATH, or set FASTROUTE_BIN.",
                    "fastroute", wx.OK | wx.ICON_ERROR, parent,
                )
                return
            dialog = SettingsDialog(parent, load_settings())
            ok = dialog.ShowModal() == wx.ID_OK
            settings = dialog.values()
            dialog.Destroy()
            if not ok:
                return
            save_settings(settings)
            wx.CallAfter(self._route, settings)
        except Exception as exc:
            log.exception("fastroute plugin failed: %s", exc)
            wx.MessageBox(f"fastroute plugin error:\n{exc}", "fastroute", wx.OK | wx.ICON_ERROR)

    def _route(self, settings):
        log = logging.getLogger("fastroute")
        try:
            self._route_inner(settings, log)
        except Exception as exc:
            log.exception("fastroute plugin failed: %s", exc)
            wx.MessageBox(f"fastroute plugin error:\n{exc}", "fastroute", wx.OK | wx.ICON_ERROR)

    def _route_inner(self, settings, log):
        board = pcbnew.GetBoard()
        parent = wx.GetActiveWindow()
        min_width = core.min_track_width_nm(board) if settings["respect_min_width"] else 0
        router = core.Router(
            board,
            extra_args=core.mode_args(
                settings["mode"], settings["max_passes"], min_track_width_nm=min_width,
                edge_clearance_nm=core.copper_edge_clearance_nm(board),
            ),
            refill=settings["refill_zones"],
            route_zone_nets=settings["route_zone_nets"],
            text_keepouts=settings["text_keepouts"],
            clear_tracks=settings["clear_tracks"],
            keep_existing=settings["keep_existing"],
            live=settings["live_view"],
            keep_stitching=settings["keep_stitching"],
            in_editor=True,
        )
        progress = ProgressDialog(parent, router)
        progress.start()
        result = progress.result
        progress.Destroy()
        pcbnew.Refresh()

        if result is not None:
            log.info("result ok=%s message=%s unrouted=%s violations=%s",
                     result.ok, result.message.splitlines()[0] if result.message else "",
                     result.unrouted, result.violations)
        if result is None or result.cancelled:
            return
        if not result.ok:
            wx.MessageBox(result.message, "fastroute", wx.OK | wx.ICON_ERROR, parent)
            return
        summary = "Routing finished. Use File > Save to keep the result."
        if result.unrouted is not None:
            ignored = result.unrouted_ignored or 0
            summary += f"\n\nUnrouted connections: {result.unrouted - ignored}"
            if result.kicad_unconnected is not None:
                summary += f" (open connections in KiCad after the import: {result.kicad_unconnected})"
            if ignored:
                summary += f"\nIn ignored net classes (not routed): {ignored}"
            summary += f"\nClearance violations: {result.violations}"
            if result.violations_unfixable:
                summary += f" ({result.violations_unfixable} of them between fixed items of the design, not fixable by routing)"
        if result.warnings:
            shown = result.warnings[:12]
            summary += "\n\nWarnings:\n" + "\n".join("- " + w for w in shown)
            if len(result.warnings) > len(shown):
                summary += f"\n... and {len(result.warnings) - len(shown)} more (see fastroute_plugin.log)"
            for w in result.warnings:
                log.warning("%s", w)
        wx.MessageBox(summary, "fastroute", wx.OK | wx.ICON_INFORMATION, parent)
