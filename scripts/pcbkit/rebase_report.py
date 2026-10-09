#!/usr/bin/env python3
"""Helper for scripts/pcbkit-rebase.sh: map conflicts to PATCH LEDGER hook sites.

Stdlib only.
  rebase_report.py --ledger docs/PCBKIT.md --files a.rs b.rs   conflict report for files in cwd
  rebase_report.py --ledger docs/PCBKIT.md --apply-log log     per-file apply status from git apply stderr
  rebase_report.py --self-check
Exit 0 always for reports (the caller decides); --self-check exits 1 on failure.
"""
import argparse
import re
import sys
import tempfile
from pathlib import Path

PATH_RE = re.compile(r"^[A-Za-z0-9_./-]+\.(rs|toml|sh|py|md)$")


def ledger_map(text):
    """path -> list of (hook id, file:line spec) from the PATCH LEDGER table."""
    out = {}
    in_ledger = False
    for line in text.splitlines():
        if line.startswith("## PATCH LEDGER"):
            in_ledger = True
            continue
        if in_ledger and line.startswith("## "):
            break
        if not (in_ledger and line.startswith("|")):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) < 2 or cells[0] in ("Hook", "") or set(cells[0]) <= set("-"):
            continue
        hook = cells[0].split()[0]
        for tok in re.findall(r"`([^`]+)`", cells[1]):
            for piece in re.split(r"[,\s]+", tok):
                path = piece.split(":")[0]
                if PATH_RE.match(path):
                    spec = piece if ":" in piece else path
                    out.setdefault(path, []).append((hook, spec))
    return out


def conflict_hunks(text):
    """(start_line, end_line) of every conflict hunk."""
    hunks, start = [], None
    for n, line in enumerate(text.splitlines(), 1):
        if line.startswith("<<<<<<<"):
            start = n
        elif line.startswith(">>>>>>>") and start is not None:
            hunks.append((start, n))
            start = None
    return hunks


def report_conflicts(ledger_text, files):
    lm = ledger_map(ledger_text)
    rows = []
    for f in files:
        p = Path(f)
        hunks = conflict_hunks(p.read_text(errors="replace")) if p.is_file() else []
        hooks = lm.get(f)
        if hooks:
            site = "; ".join(sorted({f"{h} ({s})" for h, s in hooks}))
        else:
            site = "NOT A LEDGER SITE (fork-owned file or untracked hook)"
        spans = ",".join(f"{a}-{b}" for a, b in hunks) or "no markers (add/delete or binary conflict)"
        rows.append((f, len(hunks), spans, site))
    return rows


def apply_status(log_text):
    """file -> clean | conflicts | failed, from `git apply --3way` stderr."""
    status = {}
    for line in log_text.splitlines():
        m = re.match(r"Applied patch to '(.+)' (cleanly|with conflicts)\.", line)
        if m:
            status[m.group(1)] = "clean" if m.group(2) == "cleanly" else "conflicts"
            continue
        m = re.match(r"error: (?:patch failed: )?(.+?)(?::\d+)?(?:: .*)?$", line)
        if m and PATH_RE.match(m.group(1)):
            status.setdefault(m.group(1), "failed")
    return status


def self_check():
    ledger = (
        "## PATCH LEDGER\n\n| Hook | File:line | Lines | Flag | Why |\n|---|---|---|---|---|\n"
        "| H1 move | `crates/a/src/x.rs:10-20` (`f`), `crates/a/src/y.rs:5` | +9 | f | w |\n"
        "| H2b keep | `crates/a/src/x.rs:74-75`, `:177` | +2 | f | w |\n\n## Next\n"
        "| H9 x | `crates/zz/ignored.rs` | | | |\n"
    )
    lm = ledger_map(ledger)
    assert set(lm) == {"crates/a/src/x.rs", "crates/a/src/y.rs"}, lm
    assert [h for h, _ in lm["crates/a/src/x.rs"]] == ["H1", "H2b"], lm
    body = "a\n<<<<<<< ours\nb\n=======\nc\n>>>>>>> theirs\nd\n<<<<<<< o\n=======\n>>>>>>> t\n"
    assert conflict_hunks(body) == [(2, 6), (8, 10)]
    log = ("Applied patch to 'a.rs' cleanly.\nApplied patch to 'b.rs' with conflicts.\n"
           "error: c.rs: does not match index\nerror: patch failed: d.rs:12\n")
    assert apply_status(log) == {"a.rs": "clean", "b.rs": "conflicts", "c.rs": "failed", "d.rs": "failed"}, apply_status(log)
    with tempfile.TemporaryDirectory() as d:
        import os
        old = os.getcwd()
        os.chdir(d)
        try:
            Path("crates/a/src").mkdir(parents=True)
            Path("crates/a/src/x.rs").write_text(body)
            rows = report_conflicts(ledger, ["crates/a/src/x.rs", "crates/q.rs"])
        finally:
            os.chdir(old)
    assert rows[0][1] == 2 and "H1" in rows[0][3] and "H2b" in rows[0][3], rows
    assert "NOT A LEDGER SITE" in rows[1][3], rows
    print("rebase_report self-check ok")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ledger")
    ap.add_argument("--files", nargs="*")
    ap.add_argument("--apply-log")
    ap.add_argument("--self-check", action="store_true")
    a = ap.parse_args()
    try:
        if a.self_check:
            self_check()
        elif a.apply_log:
            for f, s in sorted(apply_status(Path(a.apply_log).read_text(errors="replace")).items()):
                print(f"{s}\t{f}")
        elif a.ledger and a.files is not None:
            for f, n, spans, site in report_conflicts(Path(a.ledger).read_text(), a.files):
                print(f"{f}\t{n}\t{spans}\t{site}")
        else:
            ap.error("need --self-check, --apply-log, or --ledger with --files")
    except AssertionError as e:
        print(f"self-check FAILED: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
