#!/usr/bin/env python3
"""Inline the compiled wasm into template.html -> one self-contained HTML file."""
import base64, sys, pathlib

here = pathlib.Path(__file__).resolve().parent
wasm = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
tpl = (here / "web" / "template.html").read_text()
b64 = base64.b64encode(wasm.read_bytes()).decode()
assert "/*WASM_B64*/" in tpl
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(tpl.replace("/*WASM_B64*/", b64))
print(f"wrote {out} ({out.stat().st_size // 1024} KiB, wasm {wasm.stat().st_size // 1024} KiB)")

# Artifact variant: the Artifact host supplies the doctype/head/body skeleton,
# so publish only <title>, <style> and the body content.
import re
html = out.read_text()
title = re.search(r"<title>.*?</title>", html, re.S).group(0)
style = re.search(r"<style>.*?</style>", html, re.S).group(0)
body = re.search(r"<body>(.*)</body>", html, re.S).group(1)
art = out.with_name(out.stem + ".artifact.html")
art.write_text(title + "\n" + style + "\n" + body)
print(f"wrote {art} ({art.stat().st_size // 1024} KiB)")
