#!/usr/bin/env python3
"""Measure that locked nets survive re-routes and that a targeted re-route after a move changes only unlocked nets.

Speaks the toolkit router protocol 1.0.0 (docs/router-protocol/SPEC.md of the toolkit) to ``fastroute serve``.
Python 3.10, stdlib only. For every board: route seed 0 from scratch, lock every complete net, then

  (A) 10 re-routes (seeds 1..10, nets=all, from=current): locked nets whose per-net SES wiring changed, and lock
      leaks (locked wiring items missing / duplicated / extra in the SES, or a changed locked-item count);
  (B) per unrouted connection (every trial starts from a snapshot restore): ask ``blockers``, move the movable
      blocking part 0.25 mm away (unlock:false, retry with unlock:true on locked_conflict), route
      nets=[ripped + unrouted + unlocked nets] from=current, and count nets whose wiring changed that were
      locked (must be 0) or neither ripped nor listed (must be 0), opens fixed, new opens, router ms.

Exit 0 only when every invariant count is zero over all measured boards; 1 otherwise; 2 on usage errors.
Results go to ``<out>/<board>.json``; the markdown report is rendered from every JSON found in ``<out>``.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import select
import subprocess
import sys
import time
from collections import Counter
from pathlib import Path

PROTOCOL = "1.0.0"
CLIENT = "pcbkit-measure-lock/1"
THREADS = 8  # the toolkit's fixed fastroute thread count (adapters/fastroute.py THREADS)
SEEDS_A = range(1, 11)
NUDGE_MM = 0.25
SMALL_PART_PINS = 3  # "small part": the lock test's movable-part rule (3 pads or fewer)
CALL_TIMEOUT_S = 120.0
ROUTE_TIMEOUT_S = 900.0
ROOT = Path(__file__).resolve().parents[2]

# Router settings = the toolkit adapters/fastroute.py settings() for route_local.sh's defaults
# (max_passes 100, neckdown on, via_costs unset); edge clearance = max(fab profile copper_to_edge_mm 0.2,
# board min_copper_edge_clearance), min width = max(profile min_trace_width_mm 0.1, board min_track_width)
# (pcbkit-route.py edge_clearance_mm / min_trace_width_mm; board rules read from each fixture's .kicad_pro).
RP2040_RULES = {"edge_mm": 0.2, "width_mm": 0.1}
RULES = {
    "rp2040env": RP2040_RULES,
    "sensor2l": {"edge_mm": 0.4, "width_mm": 0.1524},
    "usbsense": {"edge_mm": 0.2, "width_mm": 0.1},
    "stm32io": {"edge_mm": 0.2, "width_mm": 0.1},
}
RP_PLACEMENTS = ["energy-1", "energy-2", "energy-3", "energy-4", "energy-5", "energy-6", "energy-8", "energy-10",
                 "energy-12", "heuristic-baseline"]
# Toolkit DSNs of the other fixture boards, relative to --toolkit (first existing one is used per board).
TOOLKIT_DSNS = {
    "sensor2l": ["tests/fixtures/boards/sensor2l/layout/board.dsn"],
    "usbsense": ["tests/fixtures/dsn/usbsense_stripped.dsn"],
    "stm32io": ["tests/fixtures/boards/stm32io/layout/board.dsn", "tests/fixtures/dsn/stm32io_stripped.dsn"],
}
LOCK_TEST = {"unconnected": 14, "fixed": 4, "new": 3, "router_s": 3.7}  # scratchpad lock-test.md, tlock arm


def settings_for(rules: dict) -> dict:
    """The hello settings, same keys and rounding as the toolkit adapter (fastroute.settings + _typed)."""
    return {
        "router.automatic_neckdown": True,
        "router.autorouter.max_passes": 100,
        "router.copper_to_edge_clearance_um": round(rules["edge_mm"] * 1000),
        "router.min_trace_width_um": round(rules["width_mm"] * 1000),
    }


# ----------------------------------------------------------------------------------------------- s-expressions
_TOKEN = re.compile(r'\(|\)|"[^"]*"|[^\s()]+')


def sexpr(text: str) -> list:
    stack: list[list] = [[]]
    for tok in _TOKEN.findall(text):
        if tok == "(":
            stack.append([])
        elif tok == ")":
            done = stack.pop()
            stack[-1].append(done)
        else:
            stack[-1].append(tok[1:-1] if tok[0] == '"' else tok)
    return stack[0][0] if stack[0] else []


def ser(node) -> str:
    if isinstance(node, list):
        return "(" + " ".join(ser(c) for c in node) + ")"
    return str(node)


def walk(node, head: str):
    if isinstance(node, list):
        if node and node[0] == head:
            yield node
        for c in node:
            yield from walk(c, head)


def image_pin_counts(dsn_text: str) -> dict[str, int]:
    """Pad count per DSN image name."""
    return {im[1]: sum(1 for c in im[2:] if isinstance(c, list) and c and c[0] == "pin")
            for im in walk(sexpr(dsn_text), "image") if len(im) > 1}


def ses_nets(ses_text: str) -> dict[str, Counter]:
    """Per net: multiset of canonical wiring items (wire / via) of the SES network_out."""
    out: dict[str, Counter] = {}
    for net_out in walk(sexpr(ses_text), "network_out"):
        for net in (c for c in net_out[1:] if isinstance(c, list) and c and c[0] == "net"):
            out.setdefault(net[1], Counter()).update(ser(it) for it in net[2:])
    return out


def ses_places(ses_text: str) -> dict[str, dict]:
    """ref -> place record {image, x, y, side, rot} from the SES placement section."""
    out = {}
    for comp in walk(sexpr(ses_text), "component"):
        for pl in (c for c in comp[2:] if isinstance(c, list) and c and c[0] == "place"):
            out[pl[1]] = {"image": comp[1], "x": int(pl[2]), "y": int(pl[3]), "side": pl[4], "rot": int(pl[5])}
    return out


def self_test() -> None:
    ses = ('(session b (placement (resolution um 10) (component "IMG A" (place R1 10 -20 front 90)))'
           ' (routes (resolution um 10) (network_out (net N1 (wire (path F.Cu 100 0 0 5 5)) (via V 1 1))'
           ' (net "N 2" (wire (path F.Cu 100 0 0 5 5)) (wire (path F.Cu 100 0 0 5 5))))))')
    nets = ses_nets(ses)
    assert sum(nets["N1"].values()) == 2 and nets["N 2"]["(wire (path F.Cu 100 0 0 5 5))"] == 2, nets
    assert ses_places(ses)["R1"] == {"image": "IMG A", "x": 10, "y": -20, "side": "front", "rot": 90}
    dsn = '(pcb x (library (image I1 (pin a 1 0 0) (pin a 2 1 0)) (image I2 (pin a 1 0 0) (outline))))'
    assert image_pin_counts(dsn) == {"I1": 2, "I2": 1}
    assert unit_nudge({"x": 0, "y": 0}, (5, 1), 2500) == (-2500, 0)
    assert unit_nudge({"x": 0, "y": 0}, (0, 9), 2500) == (0, -2500)
    print("measure_lock self-test ok")


# ----------------------------------------------------------------------------------------------- protocol client
class ServerError(RuntimeError):
    def __init__(self, op: str, err: dict):
        super().__init__(f"{op}: {err.get('code')}: {err.get('message')}")
        self.op, self.code, self.details = op, err.get("code"), err.get("details") or {}


class Server:
    """One ``fastroute serve`` child; stderr goes to a log file, never mixed with the protocol stream."""

    def __init__(self, cmd: list[str], log: Path):
        self._log = open(log, "wb")
        self.proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self._log,
                                     env={"PATH": "/usr/bin:/bin"})
        self.next_id = 1
        self.build = ""

    def call(self, op: str, args: dict | None = None, timeout: float = CALL_TIMEOUT_S) -> dict:
        rid, self.next_id = self.next_id, self.next_id + 1
        line = json.dumps({"id": rid, "op": op, "args": args or {}}, separators=(",", ":")) + "\n"
        self.proc.stdin.write(line.encode())
        self.proc.stdin.flush()
        ready, _, _ = select.select([self.proc.stdout], [], [], timeout)
        if not ready:
            self.proc.kill()
            raise RuntimeError(f"{op}: no response in {timeout:.0f} s")
        raw = self.proc.stdout.readline()
        if not raw:
            raise RuntimeError(f"{op}: server closed the stream (rc {self.proc.poll()})")
        resp = json.loads(raw)
        if resp.get("id") != rid:
            raise RuntimeError(f"{op}: response id {resp.get('id')} != {rid}")
        if self.build and resp.get("build") != self.build:
            raise RuntimeError(f"{op}: build changed mid-session")
        self.build = resp.get("build", self.build)
        if not resp.get("ok"):
            raise ServerError(op, resp.get("error", {}))
        return resp["result"]

    def close(self) -> None:
        try:
            self.call("shutdown", timeout=10)
        except (RuntimeError, OSError, ServerError):
            self.proc.kill()
        self.proc.wait(timeout=10)
        self._log.close()


# ----------------------------------------------------------------------------------------------- measurement
def conn_key(c: dict) -> tuple:
    return (c["net"], *sorted((c["from"], c["to"])))


def unit_nudge(place: dict, at: tuple[int, int], size: int) -> tuple[int, int]:
    """(dx, dy) of length ``size`` along the dominant axis of (part - at): away from the blocked point."""
    vx, vy = place["x"] - at[0], place["y"] - at[1]
    if abs(vx) >= abs(vy):
        return ((size if vx >= 0 else -size), 0)
    return (0, (size if vy > 0 else -size))


def diff_nets(before: dict[str, Counter], after: dict[str, Counter]) -> dict[str, dict]:
    """Nets whose item multiset differs: net -> {missing, extra, duplicated} counts (item-level)."""
    out = {}
    for net in sorted(set(before) | set(after)):
        b, a = before.get(net, Counter()), after.get(net, Counter())
        if b == a:
            continue
        missing = sum((b - a).values())
        extra = sum((a - b).values())
        dup = sum(1 for k, n in a.items() if n > 1 and n > b.get(k, 0))
        out[net] = {"missing": missing, "extra": extra, "duplicated": dup}
    return out


def pick_part(srv: Server, conn: dict, places: dict, pins: dict[str, int]) -> tuple[str | None, str, str, tuple]:
    """(ref, source, class, at) for the nudge: the nearest small blocking part, else the small part at an end."""
    res = srv.call("blockers", {"connection": {"net": conn["net"], "from": conn["from"], "to": conn["to"]},
                                "max": 20})
    ends = {conn["from"].rsplit("-", 1)[0], conn["to"].rsplit("-", 1)[0]}

    def small(ref: str) -> bool:
        return ref in places and pins.get(places[ref]["image"], 99) <= SMALL_PART_PINS

    for b in res["blockers"]:
        ref = b.get("ref")
        if b["kind"] == "pin" and ref and ref not in ends and small(ref):
            return ref, "blocker", res["class"], tuple(b["at"])
    mid = ((conn["from_xy"][0] + conn["to_xy"][0]) // 2, (conn["from_xy"][1] + conn["to_xy"][1]) // 2)
    for end in (conn["from"], conn["to"]):
        ref = end.rsplit("-", 1)[0]
        if small(ref):
            return ref, "fallback-end", res["class"], mid
    return None, "none", res["class"], mid


def run_board(name: str, dsn: Path, rules: dict, binary: Path, outdir: Path, argv: list[str]) -> dict:
    log = outdir / (name.replace("/", "_") + ".stderr.log")
    srv = Server([str(binary), "serve"], log)
    t_board = time.time()
    rec: dict = {"board": name, "dsn": str(dsn), "argv": argv, "settings": settings_for(rules)}
    try:
        hello = srv.call("hello", {"protocol": PROTOCOL, "client": CLIENT, "threads": THREADS,
                                   "settings": rec["settings"]})
        for need in ("locking", "snapshot", "move", "blockers", "incremental", "seed"):
            if need not in hello["capabilities"]:
                raise RuntimeError(f"server lacks capability {need}")
        if hello["threads"] != THREADS or hello["settings"]["unknown"]:
            raise RuntimeError(f"hello mismatch: {hello['threads']} threads, unknown {hello['settings']['unknown']}")
        rec.update(build=hello["build"], threads=hello["threads"], router=hello["router"],
                   applied=hello["settings"]["applied"], protocol=hello["protocol"])
        load = srv.call("load", {"dsn": {"path": str(dsn)}})
        unit = load["resolution"]["value"]
        nudge_units = round(NUDGE_MM * 1000 * unit)
        pins = image_pin_counts(dsn.read_text(errors="replace"))
        r0 = srv.call("route", {"seed": 0, "nets": "all", "from": "scratch"}, ROUTE_TIMEOUT_S)
        complete = [n["name"] for n in r0["nets"] if n["status"] == "routed" and n["unrouted"] == 0]
        lock0 = srv.call("lock", {"nets": complete})
        ses0_text = srv.call("export", {"format": "ses"})["text"]
        wiring0, places = ses_nets(ses0_text), ses_places(ses0_text)
        ses0_sha = hashlib.sha256(ses0_text.encode()).hexdigest()
        snap = srv.call("snapshot")["snapshot"]
        opens0 = {conn_key(c): c for c in r0["unrouted_connections"]}
        rec["seed0"] = {"complete": r0["complete"], "connections": r0["connections"], "unrouted": r0["unrouted"],
                        "nets": len(r0["nets"]), "locked_nets": len(complete), "locked_items": lock0["wires"],
                        "vias": r0["vias"], "wires": r0["wires"], "wall_ms": r0["wall_ms"],
                        "unrouted_connections": [list(k) for k in opens0], "ses_sha256": ses0_sha}
        locked = set(complete)

        # (A) re-routes of everything from the current locked board
        a_rows, a_locked_changed, a_leaks = [], 0, 0
        for seed in SEEDS_A:
            r = srv.call("route", {"seed": seed, "nets": "all", "from": "current"}, ROUTE_TIMEOUT_S)
            ses = srv.call("export", {"format": "ses"})["text"]
            d = diff_nets(wiring0, ses_nets(ses))
            changed_locked = sorted(n for n in d if n in locked)
            leaks = sum(d[n]["missing"] + d[n]["duplicated"] + max(0, d[n]["extra"] - d[n]["duplicated"])
                        for n in changed_locked)
            relock = srv.call("lock", {"nets": complete})
            count_drift = relock["wires"] != lock0["wires"] or relock["nets"] != lock0["nets"]
            status_bad = [n["name"] for n in r["nets"] if n["name"] in locked and n["status"] != "locked"]
            leaks += int(count_drift) + len(status_bad)
            a_locked_changed += len(changed_locked)
            a_leaks += leaks
            a_rows.append({"seed": seed, "unrouted": r["unrouted"], "wall_ms": r["wall_ms"],
                           "locked_changed": changed_locked, "leaks": leaks, "lock_count_drift": count_drift,
                           "status_not_locked": status_bad,
                           "unlocked_nets_changed": sum(1 for n in d if n not in locked)})
        rec["A"] = {"rows": a_rows, "locked_nets_changed": a_locked_changed, "lock_leaks": a_leaks}

        # (B) targeted re-route after a nudge, each trial from the snapshot
        b_rows = []
        for key, conn in opens0.items():
            rest = srv.call("restore", {"snapshot": snap})
            row: dict = {"connection": list(key), "restored_unrouted": rest["unrouted"]}
            ref, source, cls, at = pick_part(srv, conn, places, pins)
            row.update(part=ref, source=source, blockers_class=cls)
            if ref is None:
                row["skipped"] = "no small part at the blocker or either end"
                b_rows.append(row)
                continue
            pl = places[ref]
            dx, dy = unit_nudge(pl, at, nudge_units)
            mv = {"ref": ref, "x": pl["x"] + dx, "y": pl["y"] + dy, "rot": pl["rot"], "side": pl["side"]}
            row["move"] = {"dx": dx, "dy": dy}
            unlocked_nets: list[str] = []
            try:
                m = srv.call("move", {"moves": [mv], "unlock": False})
            except ServerError as exc:
                if exc.code != "locked_conflict":
                    row["skipped"] = f"move failed: {exc}"
                    b_rows.append(row)
                    continue
                unlocked_nets = list(exc.details.get("nets", []))
                row["locked_conflict"] = {"nets": unlocked_nets, "wires": exc.details.get("wires", [])}
                m = srv.call("move", {"moves": [mv], "unlock": True})
            ripped = list(m["ripped"]["nets"])
            listed = sorted(set(ripped) | {k[0] for k in opens0} | set(unlocked_nets))
            still_locked = locked - set(unlocked_nets)
            r = srv.call("route", {"seed": 0, "nets": listed, "from": "current"}, ROUTE_TIMEOUT_S)
            ses = srv.call("export", {"format": "ses"})["text"]
            d = diff_nets(wiring0, ses_nets(ses))
            locked_changed = sorted(n for n in d if n in still_locked)
            out_scope = sorted(n for n in d if n not in still_locked and n not in listed)
            opens1 = {conn_key(c) for c in r["unrouted_connections"]}
            row.update(ripped_nets=ripped, listed=len(listed), unlocked_by_conflict=unlocked_nets,
                       locked_changed=locked_changed, out_of_scope_changed=out_scope,
                       nets_changed=len(d), opens_before=len(opens0), opens_after=len(opens1),
                       target_fixed=key not in opens1, fixed=len(set(opens0) - opens1),
                       new=len(opens1 - set(opens0)), router_ms=r["wall_ms"], complete=r["complete"])
            b_rows.append(row)
        measured = [r for r in b_rows if "skipped" not in r]
        rec["B"] = {"rows": b_rows, "trials": len(measured), "skipped": len(b_rows) - len(measured),
                    "locked_nets_changed": sum(len(r["locked_changed"]) for r in measured),
                    "out_of_scope_nets_changed": sum(len(r["out_of_scope_changed"]) for r in measured)}
        rest = srv.call("restore", {"snapshot": snap})
        again = srv.call("export", {"format": "ses"})["text"]
        rec["restore_matches_seed0_ses"] = hashlib.sha256(again.encode()).hexdigest() == ses0_sha
        rec["wall_s"] = round(time.time() - t_board, 1)
    finally:
        srv.close()
    return rec


# ----------------------------------------------------------------------------------------------- report
def violations(rec: dict) -> int:
    return (rec["A"]["locked_nets_changed"] + rec["A"]["lock_leaks"] + rec["B"]["locked_nets_changed"]
            + rec["B"]["out_of_scope_nets_changed"] + (0 if rec.get("restore_matches_seed0_ses") else 1))


def render(recs: list[dict], not_measured: list[str], settings_note: str) -> str:
    L: list[str] = []
    w = L.append
    b0 = recs[0] if recs else {}
    w("# Lock and targeted re-route measurements\n")
    w("Generated by `scripts/pcbkit/measure_lock.py`. Router protocol 1.0.0 against `fastroute serve`.\n")
    w(f"- Router build: `{b0.get('build', '-')}` ({b0.get('router', {}).get('name', '-')} "
      f"{b0.get('router', {}).get('version', '-')})")
    w(f"- Threads: {b0.get('threads', '-')} (the toolkit's fixed fastroute thread count), seed 0 for the base route, "
      "count-mode limits (deterministic)")
    w(f"- Settings source: {settings_note}")
    w("- Settings applied: `" + json.dumps(b0.get("applied", {}), sort_keys=True) + "`\n")
    w("## Commands\n")
    w("Each run goes through the toolkit's route slot, with the toolkit's route cache variable exported (the server "
      "does not use the cache), after `cargo build --release -p fastroute` in this worktree:\n")
    w("```")
    w("export PCBKIT_ROUTE_CACHE=~/.cache/pcbkit/route-cache")
    seen: list[str] = []
    for r in recs:
        c = " ".join(r["argv"])
        if c not in seen:
            seen.append(c)
            w("$TOOLKIT/scripts/improve_route_slot.sh " + c.replace(str(ROOT) + "/", ""))
    w("```\n")
    w("## Invariants (all must be zero)\n")
    w("| board | A: locked nets changed | A: lock leaks | B: locked nets changed | B: out-of-scope nets changed | "
      "restore = seed-0 SES |")
    w("|---|---|---|---|---|---|")
    tot = [0, 0, 0, 0]
    for r in recs:
        v = [r["A"]["locked_nets_changed"], r["A"]["lock_leaks"], r["B"]["locked_nets_changed"],
             r["B"]["out_of_scope_nets_changed"]]
        tot = [a + b for a, b in zip(tot, v)]
        w(f"| {r['board']} | {v[0]} | {v[1]} | {v[2]} | {v[3]} | {'yes' if r.get('restore_matches_seed0_ses') else 'NO'} |")
    w(f"| **total** | **{tot[0]}** | **{tot[1]}** | **{tot[2]}** | **{tot[3]}** | |")
    for n in not_measured:
        w(f"| {n} | not measured | | | | |")
    w("")
    w("## Base route (seed 0, from scratch) and the lock\n")
    w("| board | nets | locked nets | locked items | connections | opens | router s | board wall s |")
    w("|---|---|---|---|---|---|---|---|")
    for r in recs:
        s = r["seed0"]
        w(f"| {r['board']} | {s['nets']} | {s['locked_nets']} | {s['locked_items']} | {s['connections']} | "
          f"{s['unrouted']} | {s['wall_ms'] / 1000:.1f} | {r.get('wall_s', '-')} |")
    w("\n## (A) 10 re-routes, seeds 1..10, nets=all, from=current, every complete net locked\n")
    w("Locked nets changed: nets in the lock set whose SES item multiset differs from the post-lock export. "
      "Lock leaks: wiring items of a locked net missing, duplicated or added in the SES, plus a changed locked-item "
      "count or net list when the same nets are locked again (an unlocked or duplicated piece would raise it), "
      "plus a locked net the route reports with a status other than `locked`.\n")
    w("| board | re-routes | locked nets changed | lock leaks | unrouted after (min-max) | other nets changed (sum) | "
      "router s (mean) |")
    w("|---|---|---|---|---|---|---|")
    for r in recs:
        rows = r["A"]["rows"]
        un = [x["unrouted"] for x in rows]
        ms = sum(x["wall_ms"] for x in rows) / len(rows) / 1000
        w(f"| {r['board']} | {len(rows)} | {r['A']['locked_nets_changed']} | {r['A']['lock_leaks']} | "
          f"{min(un)}-{max(un)} | {sum(x['unlocked_nets_changed'] for x in rows)} | {ms:.2f} |")
    w("\n## (B) targeted nudge, then re-route of the affected nets only\n")
    w("One trial per unrouted connection of the base route, each from a snapshot restore. The part is the nearest "
      "small (3 pads or fewer) `pin` blocker that is not an endpoint of the open; else the small part at an end of "
      "the open. It moves 0.25 mm along the dominant axis away from the blocker point (`unlock:false`, retry with "
      "`unlock:true` on `locked_conflict`). Route: `nets` = ripped + all base opens' nets + nets unlocked by a "
      "conflict, `from: current`, seed 0.\n")
    w("| board | trials | skipped | conflicts (unlock retry) | locked nets changed | out-of-scope changed | "
      "target fixed | fixed (all) | new opens | router s (mean) |")
    w("|---|---|---|---|---|---|---|---|---|---|")
    g = {"trials": 0, "fixed": 0, "new": 0, "ms": 0, "tfix": 0, "conf": 0}
    for r in recs:
        rows = [x for x in r["B"]["rows"] if "skipped" not in x]
        conf = sum(1 for x in rows if x.get("locked_conflict"))
        fixed, new = sum(x["fixed"] for x in rows), sum(x["new"] for x in rows)
        tfix = sum(1 for x in rows if x["target_fixed"])
        ms = sum(x["router_ms"] for x in rows)
        g["trials"] += len(rows); g["fixed"] += fixed; g["new"] += new; g["ms"] += ms; g["tfix"] += tfix
        g["conf"] += conf
        mean = f"{ms / len(rows) / 1000:.2f}" if rows else "-"
        w(f"| {r['board']} | {len(rows)} | {r['B']['skipped']} | {conf} | {r['B']['locked_nets_changed']} | "
          f"{r['B']['out_of_scope_nets_changed']} | {tfix} | {fixed} | {new} | {mean} |")
    mean_all = f"{g['ms'] / g['trials'] / 1000:.2f}" if g["trials"] else "-"
    w(f"| **total** | {g['trials']} | | {g['conf']} | | | {g['tfix']} | {g['fixed']} | {g['new']} | {mean_all} |")
    w("\nTrial detail:\n")
    w("| board | open | part (source, class) | nudge | unlocked by conflict | ripped nets | opens before -> after | "
      "fixed / new | router s |")
    w("|---|---|---|---|---|---|---|---|---|")
    for r in recs:
        for x in r["B"]["rows"]:
            c = x["connection"]
            if "skipped" in x:
                w(f"| {r['board']} | {c[0]} {c[1]}-{c[2]} | skipped: {x['skipped']} | | | | | | |")
                continue
            w(f"| {r['board']} | {c[0]} {c[1]}-{c[2]} | {x['part']} ({x['source']}, {x['blockers_class']}) | "
              f"{x['move']['dx']},{x['move']['dy']} | {', '.join(x['unlocked_by_conflict']) or '-'} | "
              f"{len(x['ripped_nets'])} | {x['opens_before']} -> {x['opens_after']} | {x['fixed']} / {x['new']} | "
              f"{x['router_ms'] / 1000:.2f} |")
    rp = [r for r in recs if r["board"].startswith("rp2040env")]
    w("\n## Comparison with the lock test (`tlock`, 9 rp2040env placements)\n")
    rp_trials = [x for r in rp for x in r["B"]["rows"] if "skipped" not in x]
    rp_after = sum(r["seed0"]["unrouted"] for r in rp)
    w("| | lock test `tlock` (full pipeline, DRC truth) | this run (router-side, protocol server) |")
    w("|---|---|---|")
    w(f"| unconnected after | {LOCK_TEST['unconnected']} (DRC) | n/a: per-trial, see the tables above |")
    w(f"| opens fixed / new | {LOCK_TEST['fixed']} / {LOCK_TEST['new']} | "
      f"{sum(x['fixed'] for x in rp_trials)} / {sum(x['new'] for x in rp_trials)} over {len(rp_trials)} trials "
      f"on {len(rp)} placements ({rp_after} base opens) |")
    mean_rp = (sum(x["router_ms"] for x in rp_trials) / len(rp_trials) / 1000) if rp_trials else 0
    w(f"| router s per targeted route (mean) | {LOCK_TEST['router_s']} (whole nudged board, CLI reload) | "
      f"{mean_rp:.2f} (live session, affected nets only) |")
    w(f"\nBase opens differ from the lock test's base column (16 DRC unconnected over its 9 placements) because the "
      "inputs differ: here the DSN is routed by the server at 8 threads from scratch with the settings above, so "
      "some placements complete (energy-3 and energy-8 have no open here and no trial) while the DSN route in the "
      "lock test went through the full pipeline.")
    w("\nThe lock test moved one blamed part per placement (up to two) and re-ran the pipeline with the rest of the "
      "board as `(type fix)` DSN wiring; here every open gets its own trial from a snapshot, so the counts above "
      "are per trial, not per placement, and are router counts. The router's unrouted count includes plane "
      "connections that KiCad completes through zones, so DRC truth about opens and clearances needs the toolkit "
      "pipeline (`scripts/route_local.sh`, SES import, zone refill, KiCad DRC); this measurement proves only the "
      "router-side invariants.\n")
    w("## Method notes\n")
    w("- Per-net wiring is compared as a multiset of canonical SES items (`wire` / `via` s-expressions); the SES is "
      "byte-deterministic, so any difference is a real change of that net's copper. Fixed wiring is never emitted.")
    w("- Item ids are not listed by the protocol (only `blockers` shows ids), so a lock leak is observed through the "
      "SES items, the locked-item count and the per-net `locked` status, not by id.")
    w("- Trials restore a snapshot before each move; after all trials the session restores once more and the SES must "
      "equal the post-lock export (column `restore = seed-0 SES`).")
    w("- Wall times are `route.wall_ms` as reported by the server; `budget_ms` is never used.")
    w("- Wall times are measured on a shared host (other agents route and run gates at the same time), so they "
      "are upper bounds; SES content is load-independent (count-mode limits).")
    w("- Every trial in (B) hit `locked_conflict` on the first attempt (the nudged part sits in the dense area "
      "covered by locked plane/power fan-out), so each result is of the `unlock:true` retry; the unlocked nets are "
      "listed per trial and are re-routed with the ripped nets.")
    if not_measured:
        w("- Not measured: " + "; ".join(not_measured) + ".")
    return "\n".join(L) + "\n"


# ----------------------------------------------------------------------------------------------- main
def shared_reference() -> Path:
    local = ROOT / "reference" / "pcbkit-corpus"
    if local.exists():
        return local
    try:
        common = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--path-format=absolute", "--git-common-dir"],
                                capture_output=True, text=True, check=True).stdout.strip()
        return Path(common).parent / "reference" / "pcbkit-corpus"
    except (OSError, subprocess.CalledProcessError):
        return local


def board_table(corpus: Path, toolkit: Path | None) -> tuple[dict[str, tuple[Path, dict]], list[str]]:
    found: dict[str, tuple[Path, dict]] = {}
    missing: list[str] = []
    for p in RP_PLACEMENTS:
        dsn = corpus / "runs" / p / "base" / "layout" / ".route" / "board.dsn"
        (found.__setitem__(f"rp2040env/{p}", (dsn, RULES["rp2040env"])) if dsn.is_file()
         else missing.append(f"rp2040env/{p} (no DSN at {dsn})"))
    for board, rels in TOOLKIT_DSNS.items():
        hit = next((toolkit / r for r in rels if toolkit and (toolkit / r).is_file()), None)
        if hit:
            found[board] = (hit, RULES[board])
        else:
            missing.append(f"{board} (no DSN found{'' if toolkit else ': --toolkit not given'})")
    return found, missing


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--binary", default=str(ROOT / "target" / "release" / "fastroute"))
    ap.add_argument("--corpus", default=None, help="reference/pcbkit-corpus (default: the shared reference/)")
    ap.add_argument("--toolkit", default=None, help="toolkit repo root, for the sensor2l / usbsense / stm32io DSNs")
    ap.add_argument("--out", default=str(ROOT / "target" / "pcbkit-measure-lock"))
    ap.add_argument("--doc", default=str(ROOT / "docs" / "PCBKIT-MEASUREMENTS.md"))
    ap.add_argument("--boards", default="", help="comma list of boards to run now (default all available)")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--render-only", action="store_true", help="run nothing; rebuild the report from <out>/*.json")
    args = ap.parse_args(argv)
    if args.self_test:
        self_test()
        return 0
    binary = Path(args.binary)
    if not binary.is_file():
        print(f"measure_lock: no router binary at {binary}", file=sys.stderr)
        return 2
    corpus = Path(args.corpus) if args.corpus else shared_reference()
    toolkit = Path(args.toolkit) if args.toolkit else None
    table, missing = board_table(corpus, toolkit)
    wanted = [b for b in args.boards.split(",") if b] or list(table)
    unknown = [b for b in wanted if b not in table and not any(m.startswith(b) for m in missing)]
    if unknown:
        print(f"measure_lock: unknown boards {unknown}; available {list(table)}", file=sys.stderr)
        return 2
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    cmd = ["python3", "scripts/pcbkit/measure_lock.py", *argv]
    for name in ([] if args.render_only else wanted):
        if name not in table:
            continue
        dsn, rules = table[name]
        print(f"== {name}  ({dsn})", flush=True)
        rec = run_board(name, dsn, rules, binary, out, cmd)
        (out / (name.replace("/", "_") + ".json")).write_text(json.dumps(rec, indent=1, sort_keys=True))
        print(f"   A: locked changed {rec['A']['locked_nets_changed']}, leaks {rec['A']['lock_leaks']}; "
              f"B: {rec['B']['trials']} trials, locked changed {rec['B']['locked_nets_changed']}, "
              f"out-of-scope {rec['B']['out_of_scope_nets_changed']}; {rec['wall_s']} s", flush=True)
    recs = [json.loads(p.read_text()) for p in sorted(out.glob("*.json"))]
    recs.sort(key=lambda r: r["board"])
    if not recs:
        print("measure_lock: no results", file=sys.stderr)
        return 2
    measured = {r["board"] for r in recs}
    not_measured = [m for m in missing if m.split(" ")[0] not in measured]
    note = ("the toolkit's `adapters/fastroute.py` `settings()` on branch `rust-integration` "
            "(infra task fastroute-cli-router, merged there at f336cca): `router.automatic_neckdown` true, "
            "`router.autorouter.max_passes` 100 (`pcbkit-route.py --max-passes` default), no `via_costs`, "
            "`router.copper_to_edge_clearance_um` = max(`profiles/jlcpcb.json` copper_to_edge_mm, board "
            "`min_copper_edge_clearance`), `router.min_trace_width_um` = max(profile min_trace_width_mm, board "
            "`min_track_width`); board rules read from each fixture's `.kicad_pro` (rp2040env 0.2 / 0.1, sensor2l "
            "0.4 / 0.1524, usbsense 0.2 / 0.1)")
    doc = Path(args.doc)
    doc.parent.mkdir(parents=True, exist_ok=True)
    doc.write_text(render(recs, not_measured, note))
    bad = sum(violations(r) for r in recs)
    print(f"wrote {doc}; boards measured {len(recs)}; invariant violations {bad}")
    if bad:
        print("measure_lock: FAIL: an invariant count is non-zero", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
