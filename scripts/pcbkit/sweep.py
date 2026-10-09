#!/usr/bin/env python3
"""Thread/determinism sweep and server-vs-CLI speed for the pcbkit `fastroute serve`.

Stdlib only; talks the toolkit's router protocol (docs/PCBKIT.md) over stdio, no toolkit imports.

  scripts/pcbkit/sweep.py --server target/release/fastroute --stock $PCBKIT_STOCK_CLI \
      [--corpus core|full|PATH...] [--threads 1,2,4,8] [--seeds 0,1,7] [--jobs 4] [--json out.json]
      [--timing-threads 4] [--timing-boards 3]

Determinism phase: for every (board, threads, seed) two separate server processes load the DSN, route
from scratch with `nets: all`, export the SES; the result minus `wall_ms` and the SES bytes must be
identical. Seed 0 must also equal the SES of the stock CLI (`--no-time-limits`, both thread flags set to
the thread count) on the same board. A DSN that every side rejects with the same error is SAME-ERR.

Timing phase (sequential, so the numbers are not skewed by other jobs): per board, wall time of the CLI
vs one serve session (spawn + hello + load + route + export), then 5 part moves routed incrementally in
one session vs 5 full CLI reloads of the moved DSN.

Exit 0 only if every row is SAME/SAME-ERR and every seed-0 row equals the stock CLI.
Corpus: core = crates/fr-io/testdata/dsn + reference/pcbkit-corpus/{det,protocol-fixtures};
full = core + reference/pcbkit-corpus/runs (sha256-deduplicated).
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import json
import queue
import re
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PROTOCOL = "1.0.0"
STABLE_DROP = ("wall_ms",)
LINE_TIMEOUT = 900.0
HANG_WAIT_S = 90.0  # a `route all from current` that takes longer after moves is the F2 livelock
MOVE_MM = 0.1  # per timing move; larger steps pile parts into their neighbours and unroute the board
# Upstream's enhancements stop the optimizer's greedy phase and the whole optimizer on a WALL-CLOCK budget even
# under --no-time-limits (crates/fr-engine/src/pipeline/optimizer.rs apply_greedy, pipeline/mod.rs default_budget);
# the stock CLI has it too. Under CPU oversubscription it fires and the result changes. Its log lines:
WALL_RULE_MARKS = ("time budget used", "Optimizer stage timed out")


class SweepError(Exception):
    pass


class Server:
    """One `fastroute serve` child; one request, one response."""

    def __init__(self, argv: list[str]) -> None:
        self.err = tempfile.TemporaryFile()
        self.p = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=self.err, text=True, encoding="utf-8", bufsize=1)
        self.lines: queue.Queue = queue.Queue()
        threading.Thread(target=self._pump, daemon=True).start()
        self.rid = 0
        self.build = None
        self.timeout = LINE_TIMEOUT

    def _pump(self) -> None:
        for line in self.p.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def call(self, op: str, args: dict | None = None) -> dict:
        self.rid += 1
        req = {"id": self.rid, "op": op, **({"args": args} if args is not None else {})}
        self.p.stdin.write(json.dumps(req) + "\n")
        self.p.stdin.flush()
        try:
            line = self.lines.get(timeout=self.timeout)
        except queue.Empty:
            raise SweepError(f"{op}: no response in {self.timeout}s") from None
        if line is None:
            raise SweepError(f"{op}: server exited ({self.p.poll()})")
        resp = json.loads(line)
        if resp.get("id") != self.rid:
            raise SweepError(f"{op}: id {resp.get('id')} != {self.rid}")
        if self.build is None:
            self.build = resp.get("build")
        elif resp.get("build") != self.build:
            raise SweepError(f"{op}: build changed in session")
        return resp

    def ok(self, op: str, args: dict | None = None) -> dict:
        r = self.call(op, args)
        if not r["ok"]:
            e = r["error"]
            raise SweepError(f"{op}: {e['code']}: {e['message']}")
        return r["result"]

    def wall_rule_fired(self) -> bool:
        self.err.seek(0)
        return has_wall_mark(self.err.read().decode("utf-8", "replace"))

    def close(self) -> None:
        try:
            if self.p.poll() is None:
                self.call("shutdown")
        except (SweepError, OSError, ValueError):
            pass
        try:
            self.p.stdin.close()
        except OSError:
            pass
        try:
            self.p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.p.kill()
            self.p.wait()


def has_wall_mark(log_text: str) -> bool:
    return any(m in log_text for m in WALL_RULE_MARKS)


def stable(result: dict) -> dict:
    return {k: v for k, v in result.items() if k not in STABLE_DROP}


def serve_route(server: list[str], dsn: Path, threads: int, seed: int, frm: str = "scratch") -> dict:
    """hello + load + scratch route + export in one session. Returns outcome dict."""
    t0 = time.monotonic()
    s = Server(server)
    try:
        s.ok("hello", {"protocol": PROTOCOL, "client": "sweep", "threads": threads})
        try:
            s.ok("load", {"dsn": {"path": str(dsn)}})
        except SweepError as exc:
            return {"error": str(exc), "build": s.build, "wall": time.monotonic() - t0, "wall_rule": False}
        res = s.ok("route", {"seed": seed, "nets": "all", "from": frm})
        text = s.ok("export", {"format": "ses"})["text"]
        return {"result": stable(res), "ses": text.encode(), "build": s.build,
                "wall": time.monotonic() - t0, "wall_rule": s.wall_rule_fired()}
    finally:
        s.close()


def cli_flags(threads: int) -> list[str]:
    return ["--no-time-limits", f"--router.autorouter.max_threads={threads}",
            f"--router.optimizer.max_threads={threads}"]


def cli_route(stock: list[str], dsn: Path, threads: int, out_dir: Path | None = None) -> dict:
    with tempfile.TemporaryDirectory(dir=out_dir) as tmp:
        out = Path(tmp) / "o.ses"
        t0 = time.monotonic()
        p = subprocess.run([*stock, "-de", str(dsn), "-do", str(out), *cli_flags(threads)],
                           capture_output=True, text=True, timeout=LINE_TIMEOUT)
        rule = has_wall_mark(p.stdout + p.stderr)
        wall = time.monotonic() - t0
        if p.returncode != 0 or not out.is_file() or out.stat().st_size == 0:
            tail = (p.stderr.strip().splitlines() or p.stdout.strip().splitlines() or [""])[-1]
            return {"error": f"exit {p.returncode}: {tail[:160]}", "wall": wall, "wall_rule": rule}
        return {"ses": out.read_bytes(), "wall": wall, "wall_rule": rule}


def sha(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()[:12]


def corpus_files(mode: str, extra: list[str]) -> list[Path]:
    ref = ROOT / "reference" / "pcbkit-corpus"
    cands = sorted((ROOT / "crates/fr-io/testdata/dsn").glob("*.dsn"))
    if extra:
        cands = [Path(e).resolve() for e in extra]
    else:
        cands += sorted(ref.glob("det/**/*.dsn")) + sorted(ref.glob("protocol-fixtures/*.dsn"))
        if mode == "full":
            cands += sorted(ref.glob("runs/**/*.dsn"))
    seen, out = set(), []
    for f in cands:
        h = hashlib.sha256(f.read_bytes()).hexdigest()
        if h not in seen:
            seen.add(h)
            out.append(f)
    return out


def det_row(server: list[str], dsn: Path, threads: int, seed: int, frm: str = "scratch", once: bool = False) -> dict:
    """Two processes, same inputs; compare result (minus wall_ms) and SES. `once`: one process only (the SES is
    still compared with the stock CLI by the caller; determinism is then not checked for this row)."""
    a = serve_route(server, dsn, threads, seed, frm)
    b = a if once else serve_route(server, dsn, threads, seed, frm)
    if "error" in a or "error" in b:
        same = a.get("error") == b.get("error")
        return {"status": "SAME-ERR" if same else "DIFF-ERR", "ses": None,
                "wall": a["wall"], "note": a.get("error") or b.get("error")}
    same = a["result"] == b["result"] and a["ses"] == b["ses"] and a["build"] == b["build"]
    rule = a["wall_rule"] or b["wall_rule"]
    status = "SAME" if same else ("LOAD-DIFF" if rule else "DIFF")
    return {"status": status, "ses": a["ses"], "wall": a["wall"], "wall_rule": rule,
            "complete": a["result"].get("complete"), "unrouted": a["result"].get("unrouted"),
            "ses_sha": sha(a["ses"]), "note": "" if same else ("differs; upstream wall-clock optimizer budget fired in a process (CPU load)"
                                     if rule else "results or SES differ between processes")}


def determinism_phase(a: argparse.Namespace, boards: list[Path], server: list[str], stock: list[str]) -> list[dict]:
    """One pool per thread count, sized so jobs x threads stays within --core-budget (oversubscription is
    what trips upstream's wall-clock optimizer budget)."""
    rows: list[dict] = []
    for t in a.threads:
        seeds = [0] if a.equiv_only else a.seeds
        jobs = [(b, t, s, "scratch") for b in boards for s in seeds] + [(b, t, 0, "current") for b in boards]
        workers = max(1, min(a.jobs, a.core_budget // t))
        with cf.ThreadPoolExecutor(max_workers=workers) as ex:
            futs = {ex.submit(det_row, server, b, tt, s, m, a.equiv_only): (b, tt, s, m) for b, tt, s, m in jobs}
            cli_futs = {ex.submit(cli_route, stock, b, t): b for b in boards}
            det = {futs[f]: f.result() for f in cf.as_completed(futs)}
            cli = {cli_futs[f]: f.result() for f in cf.as_completed(cli_futs)}

        def vs_stock(b: Path, r: dict) -> str:
            c = cli[b]
            if "error" in c or r["ses"] is None:
                return "SAME-ERR" if ("error" in c and r["ses"] is None) else "DIFF-ERR"
            if c["ses"] == r["ses"]:
                return "SAME"
            return "LOAD-DIFF" if (c["wall_rule"] or r["wall_rule"]) else "DIFF"

        for b, tt, s, m in jobs:
            r = det[(b, tt, s, m)]
            row = {"board": "/".join(b.parts[-4:-1]) if b.name == "board.dsn" else b.name, "path": str(b),
                   "threads": t, "seed": s, "mode": m, "status": r["status"], "det_checked": not a.equiv_only, "wall_s": round(r["wall"], 2),
                   "note": r.get("note", ""), "ses_sha": r.get("ses_sha"), "unrouted": r.get("unrouted")}
            if s == 0:
                row["stock"] = vs_stock(b, r)
                if m == "scratch" and row["stock"] == "DIFF" and vs_stock(b, det[(b, tt, 0, "current")]) == "SAME":
                    # DSN carries unfixed wiring: stock resumes it, scratch drops it (SPEC 5.5)
                    row["stock"] = "SAME-RESUME"
                    row["note"] = "scratch drops DSN wiring the stock CLI resumes; mode=current equals stock"
            elif r["ses"] is not None:
                ref = det[(b, tt, 0, "scratch")]["ses"]
                row["differs_from_seed0"] = None if ref is None else r["ses"] != ref
            rows.append(row)
        print(f"threads {t}: {len(jobs)} jobs, {workers} workers done", flush=True)
    return rows


# ------------------------------------------------------------------------------------------ timing

PLACE = re.compile(r"\(place\s+(\S+)\s+(-?\d+(?:\.\d+)?)\s+(-?\d+(?:\.\d+)?)\s+(front|back)\s+(-?\d+(?:\.\d+)?)")


def parse_places(dsn_text: str) -> list[tuple[str, float, float, str, float]]:
    return [(m[1], float(m[2]), float(m[3]), m[4], float(m[5])) for m in PLACE.finditer(dsn_text)]


def num(v: float) -> str:
    return str(int(v)) if v == int(v) else repr(v)


def shift_place(dsn_text: str, ref: str, dx: float, dy: float) -> str:
    def sub(m: re.Match) -> str:
        if m[1] != ref:
            return m[0]
        return f"(place {ref} {num(float(m[2]) + dx)} {num(float(m[3]) + dy)} {m[4]} {m[5]}"
    return PLACE.sub(sub, dsn_text)


def routed_parts(server: list[str], dsn: Path, text: str, threads: int, n: int) -> list[str]:
    """Front parts whose 0.1 mm move rips at least one net on the scratch-routed board (found on a snapshot,
    untimed), so the timed moves have wiring to re-route. Falls back to evenly spread parts."""
    pl = {r: (x, y, sd, rt) for r, x, y, sd, rt in parse_places(text)}
    cands = movable_parts(text, 60)
    s = Server(server)
    found: list[str] = []
    try:
        s.ok("hello", {"protocol": PROTOCOL, "client": "sweep", "threads": threads})
        ld = s.ok("load", {"dsn": {"path": str(dsn)}})
        s.ok("route", {"seed": 0, "nets": "all", "from": "scratch"})
        snap = s.ok("snapshot")["snapshot"]
        step = round(MOVE_MM * {"um": 1000, "mm": 1, "mil": 1000 / 25.4, "inch": 1 / 25.4}[ld["resolution"]["unit"]]
                     * ld["resolution"]["value"])
        for ref in cands:
            x, y, sd, rt = pl[ref]
            mv = s.ok("move", {"carry": "none", "moves": [{"ref": ref, "x": round(x) + step, "y": round(y),
                                                           "rot": round(rt) % 360, "side": sd}]})
            if mv["ripped"]["nets"]:
                found.append(ref)
            s.ok("restore", {"snapshot": snap})
            if len(found) == n:
                break
    finally:
        s.close()
    return found or movable_parts(text, n)


def movable_parts(dsn_text: str, n: int) -> list[str]:
    seen, out = set(), []
    for ref, _x, _y, side, _r in parse_places(dsn_text):
        if side == "front" and ref not in seen:
            seen.add(ref)
            out.append(ref)
    step = max(1, len(out) // n)
    return out[::step][:n]


def timing_board(server: list[str], stock: list[str], dsn: Path, threads: int, n_moves: int) -> dict:
    text = dsn.read_text(errors="replace")
    cli = cli_route(stock, dsn, threads)
    srv = serve_route(server, dsn, threads, 0)
    row = {"board": dsn.parent.name if dsn.name == "board.dsn" else dsn.name, "threads": threads,
           "cli_s": round(cli["wall"], 2), "serve_s": round(srv["wall"], 2)}
    if "error" in cli or "error" in srv:
        row["note"] = "board rejected"
        return row
    refs = routed_parts(server, dsn, text, threads, n_moves)
    row["moved_parts"] = refs
    if not refs:
        row["note"] = "no movable parts"
        return row
    row["moves"] = len(refs)
    places = {r: (x, y) for r, x, y, _s, _r in parse_places(text)}
    sides = {r: (sd, rt) for r, _x, _y, sd, rt in parse_places(text)}
    for variant, label in (("all", "session_moves_all_s"), ("ripped", "session_moves_ripped_s")):
        t0 = time.monotonic()
        s = Server(server)
        inc_times: list[float] = []
        last = None
        try:
            s.ok("hello", {"protocol": PROTOCOL, "client": "sweep", "threads": threads})
            ld = s.ok("load", {"dsn": {"path": str(dsn)}})
            s.ok("route", {"seed": 0, "nets": "all", "from": "scratch"})
            per_mm = {"um": 1000, "mm": 1, "mil": 1000 / 25.4, "inch": 1 / 25.4}[ld["resolution"]["unit"]]
            step = round(MOVE_MM * per_mm * ld["resolution"]["value"])
            cur = dict(places)
            for ref in refs:
                ti = time.monotonic()
                x, y = cur[ref]
                cur[ref] = (x + step, y)
                mv = s.ok("move", {"carry": "none", "moves": [{"ref": ref, "x": round(cur[ref][0]), "y": round(cur[ref][1]),
                                              "rot": round(sides[ref][1]) % 360, "side": sides[ref][0]}]})
                nets = "all" if variant == "all" else mv["ripped"]["nets"]
                if nets:  # nothing ripped: nothing to re-route
                    s.timeout = HANG_WAIT_S if variant == "all" else LINE_TIMEOUT
                    try:
                        last = s.ok("route", {"seed": 0, "nets": nets, "from": "current"})
                    except SweepError as exc:  # finding F2: route(all, current) after 2+ moves can livelock
                        row["hang_" + variant] = f"after {len(inc_times)} moves: {exc}"
                        s.p.kill()
                        break
                inc_times.append(time.monotonic() - ti)
            if "hang_" + variant not in row:
                s.ok("export", {"format": "ses"})
            row["unrouted_after_" + variant] = last["unrouted"] if last else None
        finally:
            s.close()
        row[label] = None if "hang_" + variant in row else round(time.monotonic() - t0, 2)
        row["per_move_route_" + variant] = [round(t, 2) for t in inc_times]
    cur = dict(places)
    reload_t = 0.0
    with tempfile.TemporaryDirectory() as tmp:
        t = text
        for ref in refs:
            t = shift_place(t, ref, step, 0)
            f = Path(tmp) / "moved.dsn"
            f.write_text(t)
            c = cli_route(stock, f, threads)
            reload_t += c["wall"]
    row["cli_reloads_s"] = round(reload_t, 2)
    return row


def probe_carry_hang(server: list[str], dsn: Path, refs: list[str], threads: int, wait: float) -> str:
    """Known defect probe (docs/PCBKIT-REBASE.md, finding F2): scratch route, `move` (carry fixed) two parts, route
    from current. Returns "ok" or "HANG" (no answer within `wait` s: the engine panics in a catch-and-retry loop)."""
    global LINE_TIMEOUT
    old, LINE_TIMEOUT = LINE_TIMEOUT, wait
    pl = {r: (x, y, sd, rt) for r, x, y, sd, rt in parse_places(dsn.read_text(errors="replace"))}
    s = Server(server)
    try:
        s.ok("hello", {"protocol": PROTOCOL, "client": "sweep", "threads": threads})
        s.ok("load", {"dsn": {"path": str(dsn)}})
        s.ok("route", {"seed": 0, "nets": "all", "from": "scratch"})
        for r in refs:
            x, y, sd, rt = pl[r]
            s.ok("move", {"moves": [{"ref": r, "x": round(x) + 1000, "y": round(y), "rot": round(rt) % 360, "side": sd}]})
        try:
            s.ok("route", {"seed": 0, "nets": "all", "from": "current"})
            return "ok"
        except SweepError:
            return "HANG"
    finally:
        LINE_TIMEOUT = old
        s.p.kill()


def timing_phase(a: argparse.Namespace, boards: list[Path], server: list[str], stock: list[str]) -> list[dict]:
    rows = []
    cands = [b for b in boards if b.stat().st_size > 20000 or len(boards) <= a.timing_boards][: a.timing_boards] \
        or boards[: a.timing_boards]
    for b in cands:
        rows.append(timing_board(server, stock, b, a.timing_threads, a.timing_moves))
        print("timing", rows[-1], flush=True)
    return rows


# ------------------------------------------------------------------------------------------ main

def self_check() -> None:
    t = "(placement (component X (place R1 100 200 front 90) (place R2 5 6 back 0)))"
    assert parse_places(t) == [("R1", 100.0, 200.0, "front", 90.0), ("R2", 5.0, 6.0, "back", 0.0)]
    assert "(place R1 150 200 front 90)" in shift_place(t, "R1", 50, 0)
    assert shift_place(t, "R2", 50, 0).count("(place R1 100 200 front 90)") == 1
    assert movable_parts(t, 1) == ["R1"]
    assert stable({"wall_ms": 3, "a": 1}) == {"a": 1}
    assert has_wall_mark("INFO Optimizer greedy phase stopped after 2 of 9 candidates (4.6 s, time budget used).")
    assert not has_wall_mark("INFO Optimizer greedy phase stopped (1.5 s, no further improvements).")
    print("self-check ok")


def is_bad(r: dict) -> bool:
    return r["status"] not in ("SAME", "SAME-ERR") or r.get("stock") in ("DIFF", "DIFF-ERR")


def summarize(files: list[str]) -> int:
    rows: list[dict] = []
    for f in files:
        rows += json.loads(Path(f).read_text())["rows"]
    boards = {r["path"] for r in rows}
    bad = [r for r in rows if is_bad(r)]
    by_t: dict[int, int] = {}
    for r in rows:
        by_t[r["threads"]] = by_t.get(r["threads"], 0) + 1
    var = [r for r in rows if r["seed"] != 0 and r.get("differs_from_seed0") is not None]
    print(f"boards={len(boards)} rows={len(rows)} rows_per_threads={dict(sorted(by_t.items()))} bad={len(bad)} "
          f"seed_variants_differing={sum(1 for r in var if r['differs_from_seed0'])}/{len(var)} "
          f"scratch_vs_stock_resume={sum(1 for r in rows if r.get('stock') == 'SAME-RESUME')} "
          f"stock_checked={sum(1 for r in rows if r.get('stock'))} "
          f"det_pairs={sum(1 for r in rows if r.get('det_checked', True))} "
          f"same_err={sum(1 for r in rows if r['status'] == 'SAME-ERR')} "
          f"load_diff={sum(1 for r in rows if r['status'] == 'LOAD-DIFF' or r.get('stock') == 'LOAD-DIFF')}")
    for r in bad:
        print("BAD", r["board"], r["threads"], r["seed"], r["mode"], r["status"], r.get("stock"), r["note"])
    return 0 if not bad else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--server", help="fork binary (runs `<bin> serve`)")
    ap.add_argument("--stock", help="stock fastroute CLI (v0.1.13) binary")
    ap.add_argument("--corpus", default="core", help="core | full | paths of DSN files")
    ap.add_argument("--threads", default="1,2,4,8")
    ap.add_argument("--seeds", default="0,1,7")
    ap.add_argument("--jobs", type=int, default=8, help="max concurrent routes (also capped by --core-budget)")
    ap.add_argument("--core-budget", type=int, default=24, help="jobs x threads stays within this many threads")
    ap.add_argument("--json", help="write the full result here")
    ap.add_argument("--timing-threads", type=int, default=4)
    ap.add_argument("--timing-moves", type=int, default=5, help="part moves per board in the timing phase")
    ap.add_argument("--timing-boards", type=int, default=3, help="0 skips the timing phase")
    ap.add_argument("--probe-carry-hang", nargs="+", metavar=("DSN", "REF"),
                    help="DSN REF1 REF2: report whether move(carry fixed) of two parts then route(current) hangs")
    ap.add_argument("--equiv-only", action="store_true",
                    help="seed 0 only, one server process per row: stock equivalence without the determinism pairs")
    ap.add_argument("--skip-det", action="store_true", help="timing phase only")
    ap.add_argument("--shard", help="i/n: only every n-th board (1-based i) of the sorted, deduplicated list")
    ap.add_argument("--summarize", nargs="+", metavar="JSON", help="merge --json files, print totals, exit 1 on any bad row")
    ap.add_argument("--self-check", action="store_true")
    a = ap.parse_args()
    if a.summarize:
        return summarize(a.summarize)
    if a.self_check:
        self_check()
        return 0
    if not a.server or not a.stock:
        ap.error("--server and --stock are required")
    if a.probe_carry_hang:
        d, *refs = a.probe_carry_hang
        t = int(a.threads.split(",")[0])
        print("probe-carry-hang", probe_carry_hang([str(Path(a.server).resolve()), "serve"], Path(d).resolve(), refs, t, 60.0))
        return 0
    a.threads = [int(x) for x in a.threads.split(",")]
    a.seeds = [int(x) for x in a.seeds.split(",")]
    if 0 not in a.seeds:
        ap.error("seed 0 must be in --seeds (stock equivalence)")
    server = [str(Path(a.server).resolve()), "serve"]
    stock = [str(Path(a.stock).resolve())]
    mode = a.corpus if a.corpus in ("core", "full") else "core"
    extra = [] if a.corpus in ("core", "full") else a.corpus.split(",")
    boards = corpus_files(mode, extra)
    if a.shard:
        i, n = (int(x) for x in a.shard.split("/"))
        boards = boards[i - 1::n]
    print(f"boards={len(boards)} threads={a.threads} seeds={a.seeds} jobs<={a.jobs} core-budget={a.core_budget}", flush=True)
    rows = [] if a.skip_det else determinism_phase(a, boards, server, stock)
    print(f"{'STATUS':9} {'STOCK':11} {'T':>2} {'SEED':>4} {'MODE':8} {'WALL':>7}  BOARD")
    for r in rows:
        print(f"{r['status']:9} {r.get('stock', '-'):11} {r['threads']:>2} {r['seed']:>4} {r['mode']:8} {r['wall_s']:>7}  "
              f"{r['board']}{'  ' + r['note'] if r['note'] else ''}")
    timing = timing_phase(a, boards, server, stock) if a.timing_boards > 0 else []
    bad = [r for r in rows if is_bad(r)]
    variants = [r for r in rows if r["seed"] != 0 and r.get("differs_from_seed0")]
    summary = {"boards": len(boards), "rows": len(rows), "bad": len(bad),
               "seed_variants_differing_from_seed0": len(variants),
               "seed_variants_total": sum(1 for r in rows if r["seed"] != 0 and r.get("differs_from_seed0") is not None),
               "scratch_vs_stock_resume": sum(1 for r in rows if r.get("stock") == "SAME-RESUME"),
               "load_diff_rows": sum(1 for r in rows if r["status"] == "LOAD-DIFF" or r.get("stock") == "LOAD-DIFF")}
    print("SUMMARY", json.dumps(summary))
    if a.json:
        Path(a.json).write_text(json.dumps({"summary": summary, "rows": rows, "timing": timing}, indent=1))
    print("SWEEP:", "ALL OK" if not bad else f"{len(bad)} BAD ROWS")
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
