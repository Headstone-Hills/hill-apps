#!/usr/bin/env python3
"""Write golden.json for the browser tests: expected values from the Python reference (hillview/).

usage: python3 gen_golden.py OUT_DIR
"""
import json
import os
import sys

import numpy as np

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", ".."))
from hillview import effects
from hillview.color import rgb_to_oklab
from hillview.modulation import hash01, source_value

out = {}
out["hash"] = [[n, s, hash01(n, s)] for n in (-5, -1, 0, 1, 2, 12345, 2 ** 40) for s in (0, 1, 7, -3, 2 ** 20)]
cases = [{"type": "lfo", "shape": "sine", "hz": 0.7, "phase": 0.2}, {"type": "lfo", "shape": "saw", "hz": 1.3},
         {"type": "lfo", "shape": "tri", "hz": 2.0, "phase": 0.9}, {"type": "lfo", "shape": "square", "hz": 3.1, "duty": 0.3},
         {"type": "noise", "hz": 2.5, "seed": 4}, {"type": "steps", "hz": 5.0, "seed": 9}]
out["mod"] = [[c, t, source_value(c, t)] for c in cases for t in (-2.37, 0.0, 0.41, 7.89, 33.3)]
out["oklab"] = [[list(map(float, c)), list(map(float, rgb_to_oklab(np.array(c, np.float32))))]
                for c in ([1, 0, 0], [0.5, 0.5, 0.5], [0.1, 0.7, 0.3], [0.9, 0.9, 0.2], [0, 0, 0], [1, 1, 1])]
W, H = 64, 40
s, px = 12345, np.empty(W * H * 3, np.uint8)
for i in range(px.size):
    s = (s * 1664525 + 1013904223) & 0xFFFFFFFF
    px[i] = s >> 24
img = px.reshape(H, W, 3).copy()
img[: H // 3, : W // 2] = [200, 30, 40]
img[H // 2:, W // 2:] = [40, 40, 200]
x = img.astype(np.float32) * np.float32(1 / 255)
rules = [{"from": "#c81e28", "to": "#00ffff", "threshold": 0.2, "softness": 0.1, "amount": 1.0},
         {"from": "#2828c8", "to": "#ffe600", "threshold": 0.2, "softness": 0.15, "amount": 0.8}]
a = effects.remap_colors(effects.posterize(x, 4), rules, 0.05, 0.9, 0.8)
b = effects.blur(effects.posterize(x, 4), 2.0)
u8 = lambda v: (np.clip(v, 0, 1) * 255 + .5).astype(np.uint8).reshape(-1).tolist()
out["img"] = {"w": W, "h": H, "rgb": img.reshape(-1).tolist(), "rules": rules, "A": u8(a), "B": u8(b)}
json.dump(out, open(os.path.join(sys.argv[1], "golden.json"), "w"))
