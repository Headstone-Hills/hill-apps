#!/usr/bin/env python3
"""Sanity-check presets.json. Run: python3 validate_presets.py [path/to/presets.json]"""
import json, sys, collections
path = sys.argv[1] if len(sys.argv) > 1 else "presets.json"
d = json.load(open(path))
ROWS = {"AC","CY","CH","OH","HT","MT","SD","RS","LT","CPS","CB","TAM","BD"}
ids, bad = set(), []
for p in d["patterns"]:
    if p["id"] in ids: bad.append(("dup id", p["id"]))
    ids.add(p["id"])
    if p["steps"] not in (12, 16): bad.append(("steps", p["id"]))
    if not p["tracks"]: bad.append(("empty", p["id"]))
    for k, s in p["tracks"].items():
        if k not in ROWS: bad.append(("row", p["id"], k))
        if len(s) != p["steps"] or set(s) - set("x.F"): bad.append(("track", p["id"], k, s))
    if p["style"] not in d["styles"]: bad.append(("style", p["id"]))
c = collections.Counter(p["style"] for p in d["patterns"])
print(len(d["patterns"]), "patterns,", len(d["styles"]), "styles,",
      sum(1 for p in d["patterns"] if p["steps"] == 12), "with 12 steps,",
      sum(s.count("F") for p in d["patterns"] for s in p["tracks"].values()), "flams")
print("PROBLEMS:" if bad else "OK", *bad, sep="\n")
sys.exit(1 if bad else 0)
