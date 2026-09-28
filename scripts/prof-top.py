#!/usr/bin/env python3
"""Summarise a `samply record --save-only --unstable-presymbolicate` profile.

Usage: prof-top.py profile.json.gz [N] [filter-substring]
Prints the top-N functions by self and inclusive sample counts (all threads).
"""
import bisect, gzip, json, re, sys
from collections import Counter

path = sys.argv[1]
top = int(sys.argv[2]) if len(sys.argv) > 2 else 40
flt = sys.argv[3] if len(sys.argv) > 3 else None
prof = json.load(gzip.open(path))
syms = json.load(open(re.sub(r"\.gz$", "", path) + ".syms.json"))
strings = syms["string_table"]
libsyms = {}
for d in syms["data"]:
    tab = sorted(d["symbol_table"], key=lambda e: e["rva"])
    libsyms[d["debug_name"]] = ([e["rva"] for e in tab], tab)

def clean(name):
    name = re.sub(r"::h[0-9a-f]{16}$", "", name)
    name = re.sub(r"<[^<>]*>", "<>", name)
    return name

def symbolize(lib, addr):
    if lib not in libsyms:
        return f"{lib}+{addr:#x}"
    rvas, tab = libsyms[lib]
    i = bisect.bisect_right(rvas, addr) - 1
    if i >= 0 and addr < tab[i]["rva"] + tab[i]["size"]:
        return clean(strings[tab[i]["symbol"]])
    return f"{lib}+{addr:#x}"

self_c, incl_c, total = Counter(), Counter(), 0
for t in prof["threads"]:
    ft, st, rt, fn = t["frameTable"], t["stackTable"], t["resourceTable"], t["funcTable"]
    names = {}
    def frame_name(f):
        if f not in names:
            func = ft["func"][f]
            res = fn["resource"][func]
            lib = prof["libs"][rt["lib"][res]]["debugName"] if res is not None and res >= 0 else "?"
            names[f] = symbolize(lib, ft["address"][f])
        return names[f]
    for s in t["samples"]["stack"]:
        if s is None:
            continue
        total += 1
        seen = set()
        leaf = True
        while s is not None:
            n = frame_name(st["frame"][s])
            if leaf:
                self_c[n] += 1
                leaf = False
            if n not in seen:
                incl_c[n] += 1
                seen.add(n)
            s = st["prefix"][s]

def show(title, c):
    print(f"== {title} (of {total} samples)")
    k = 0
    for n, v in c.most_common():
        if flt and flt not in n:
            continue
        print(f"{100.0 * v / total:6.2f}%  {n[:150]}")
        k += 1
        if k >= top:
            break

show("self", self_c)
show("inclusive", incl_c)
