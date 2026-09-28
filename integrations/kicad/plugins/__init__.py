"""fastroute KiCad plugin: registers the PCB editor action."""

try:
    from .action import FastrouteAction

    FastrouteAction().register()
except Exception as exc:  # never break KiCad's plugin loading
    import logging

    logging.getLogger("fastroute").exception("fastroute plugin failed to load: %s", exc)
