"""fastroute KiCad plugin: registers the PCB editor action."""

import logging
from pathlib import Path

_log = logging.getLogger("fastroute")
if not _log.handlers:
    try:
        _handler = logging.FileHandler(Path(__file__).resolve().parent / "fastroute_plugin.log")
        _handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(message)s"))
        _log.addHandler(_handler)
        _log.setLevel(logging.INFO)
    except OSError:
        pass

try:
    from .action import FastrouteAction

    FastrouteAction().register()
    _log.info("fastroute plugin registered")
except Exception as exc:  # never break KiCad's plugin loading
    _log.exception("fastroute plugin failed to load: %s", exc)
