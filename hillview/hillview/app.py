"""HillView standalone app: a local web UI served by the Python stdlib.

Security: this process reads and writes files on request, so it binds to 127.0.0.1 only,
requires a random per-launch token on every /api call (blocks other websites from driving it
via the browser), and rejects foreign Host headers (blocks DNS rebinding).
"""
import json
import os
import re
import secrets
import shutil
import threading
import uuid
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

from . import audio as audio_mod, video_io
from .engine import Engine
from .pngio import encode_png
from .render import render_video, Cancelled

HOME = os.environ.get("HILLVIEW_HOME", os.path.join(os.path.expanduser("~"), "HillView"))
UPLOADS, OUTPUT = os.path.join(HOME, "uploads"), os.path.join(HOME, "output")
UI_DIR = os.path.join(os.path.dirname(__file__), "ui")
PREVIEW_W = 640

TOKEN = secrets.token_urlsafe(24)
SESSIONS, JOBS, LOCK = {}, {}, threading.Lock()


def safe_name(name):
    name = re.sub(r"[^\w.\- ]", "_", os.path.basename(name or "file")).strip(". ") or "file"
    return name[:120]


class Session:
    def __init__(self, path, fps=None):
        self.id, self.path = uuid.uuid4().hex[:12], path
        self.info = video_io.probe(path, fps)
        self.fps_override = fps
        self.audio_path, self._mod, self._mod_for = None, None, None
        s = min(1.0, PREVIEW_W / self.info["width"])
        self.pw, self.ph = max(2, int(self.info["width"] * s) // 2 * 2), max(2, int(self.info["height"] * s) // 2 * 2)
        self.px_scale = self.pw / self.info["width"]
        self.src = video_io.RandomSource(path, self.info, (self.pw, self.ph))
        self.lock = threading.Lock()

    def audio_source(self):
        return self.audio_path or (self.path if self.info["has_audio"] else None)

    def audio_mod(self):
        """Analyze lazily; cached per audio file."""
        a = self.audio_source()
        if not a:
            return None
        if self._mod_for != a:
            self._mod = audio_mod.analyze(audio_mod.decode(a), self.info["fps"], n_frames=self.info["frames"])
            self._mod_for = a
        return self._mod


class Job:
    def __init__(self, out_path, fmt):
        self.id, self.out, self.fmt = uuid.uuid4().hex[:12], out_path, fmt
        self.status, self.done, self.total, self.error, self.cancel = "running", 0, 1, None, False


def run_job(job, sess, cfg, fmt, use_audio, speed=None):
    try:
        am = sess.audio_mod() if use_audio else None
        render_video(sess.path, job.out, cfg, fmt, audio_path=sess.audio_path, audio_mod=am,
                     fps=sess.fps_override, cancel=lambda: job.cancel, speed=speed,
                     progress=lambda i, t: (setattr(job, "done", i), setattr(job, "total", max(t, 1))))
        job.status = "done"
    except Cancelled:
        job.status = "cancelled"
    except Exception as e:  # surfaced to the UI
        job.status, job.error = "error", str(e)


class Handler(BaseHTTPRequestHandler):
    server_version = "HillView"

    def log_message(self, *a):
        pass

    # --- helpers
    def _send(self, code, body=b"", ctype="application/json", headers=None):
        if isinstance(body, (dict, list)):
            body = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def _err(self, code, msg):
        self._send(code, {"error": msg})

    def _host_ok(self):
        host = (self.headers.get("Host") or "").rsplit(":", 1)[0]
        return host in ("127.0.0.1", "localhost")

    def _json(self):
        n = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(n) or b"{}")

    def _session(self, sid):
        s = SESSIONS.get(sid)
        if not s:
            raise KeyError("no such session; open a file first")
        return s

    # --- routing
    def do_GET(self):
        self._route("GET")

    def do_POST(self):
        self._route("POST")

    def _route(self, method):
        if not self._host_ok():
            return self._err(403, "bad host")
        u = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(u.query).items()}
        if method == "GET" and u.path == "/":
            html = open(os.path.join(UI_DIR, "index.html"), encoding="utf-8").read().replace("__TOKEN__", TOKEN)
            return self._send(200, html.encode(), "text/html; charset=utf-8")
        if not u.path.startswith("/api/"):
            return self._err(404, "not found")
        if not (secrets.compare_digest(self.headers.get("X-HV-Token", ""), TOKEN)
                or secrets.compare_digest(q.get("token", ""), TOKEN)):
            return self._err(403, "bad token")
        try:
            fn = getattr(self, "api_" + u.path[5:].replace("/", "_"), None)
            fn(q) if fn else self._err(404, "unknown endpoint")
        except KeyError as e:
            self._err(404, str(e).strip("'\""))
        except (ValueError, json.JSONDecodeError) as e:
            self._err(400, str(e))
        except Exception as e:
            self._err(500, f"{type(e).__name__}: {e}")

    # --- endpoints
    def api_formats(self, q):
        self._send(200, video_io.available_formats())

    def api_upload(self, q):
        d = os.path.join(UPLOADS, uuid.uuid4().hex[:8])
        os.makedirs(d, exist_ok=True)
        path = os.path.join(d, safe_name(q.get("name")))
        left = int(self.headers.get("Content-Length") or 0)
        with open(path, "wb") as f:
            while left > 0:
                chunk = self.rfile.read(min(1 << 20, left))
                if not chunk:
                    break
                f.write(chunk)
                left -= len(chunk)
        self._send(200, {"path": path})

    def api_open(self, q):
        b = self._json()
        path = os.path.expanduser(b["path"])
        if not (os.path.exists(path) or video_io.is_sequence(path)):
            raise ValueError(f"file not found: {path}")
        s = Session(path, b.get("fps"))
        if b.get("audio"):
            audio_path = os.path.expanduser(b["audio"])
            if not os.path.exists(audio_path):
                raise ValueError(f"audio file not found: {audio_path}")
            s.audio_path = audio_path
        SESSIONS[s.id] = s
        self._send(200, {"id": s.id, "info": s.info, "audio": s.audio_source() is not None,
                         "name": os.path.basename(path)})

    def api_set_audio(self, q):
        b = self._json()
        s = self._session(b["id"])
        p = os.path.expanduser(b.get("path") or "")
        if p and not os.path.exists(p):
            raise ValueError(f"audio file not found: {p}")
        s.audio_path = p or None
        self._send(200, {"audio": s.audio_source() is not None})

    def api_preview(self, q):
        b = self._json()
        s = self._session(b["id"])
        n = max(0, min(int(b.get("frame", 0)), s.info["frames"] - 1))
        with s.lock:
            if b.get("original"):
                img = s.src.get(n)
            else:
                cfg = b.get("config") or {}
                needs_audio = any(m.get("source", {}).get("type") == "audio" for m in cfg.get("modulation", []))
                eng = Engine(cfg, s.info["fps"], s.audio_mod() if needs_audio else None, s.px_scale)
                img = eng.render(n, s.src)
        self._send(200, encode_png(img), "image/png")

    def api_render(self, q):
        b = self._json()
        s = self._session(b["id"])
        fmt = b.get("format", "h264")
        fmts = video_io.available_formats()
        if fmt not in fmts:
            raise ValueError(f"format {fmt!r} not available on this machine")
        video_io.check_size(fmt, s.info["width"], s.info["height"])
        base = safe_name(b.get("name") or os.path.splitext(os.path.basename(s.path))[0] + "_hillview")
        base = os.path.splitext(base)[0]
        os.makedirs(OUTPUT, exist_ok=True)
        ext, seq = fmts[fmt]["ext"], fmts[fmt]["kind"] == "seq"
        taken = lambda n: os.path.exists(os.path.join(OUTPUT, n if seq else f"{n}.{ext}"))
        name, i = base, 1
        while taken(name):  # never overwrite an earlier export
            name, i = f"{base}_{i}", i + 1
        out = os.path.join(OUTPUT, name if seq else f"{name}.{ext}")
        job = Job(out, fmt)
        JOBS[job.id] = job
        threading.Thread(target=run_job, args=(job, s, b.get("config") or {}, fmt, b.get("audio_reactive", True), b.get("speed") or None),
                         daemon=True).start()
        self._send(200, {"job": job.id})

    def api_job(self, q):
        j = JOBS.get(q.get("id"))
        if not j:
            raise KeyError("no such job")
        self._send(200, {"status": j.status, "done": j.done, "total": j.total, "error": j.error,
                         "path": j.out, "kind": video_io.FORMATS[j.fmt][5]})

    def api_cancel(self, q):
        j = JOBS.get(q.get("id"))
        if j:
            j.cancel = True
        self._send(200, {})

    def api_download(self, q):
        j = JOBS.get(q.get("id"))
        if not j or j.status != "done":
            raise KeyError("nothing to download")
        path, ctype = j.out, "application/octet-stream"
        if video_io.FORMATS[j.fmt][5] == "seq":  # zip the sequence folder
            path = shutil.make_archive(j.out, "zip", j.out)
            ctype = "application/zip"
        size = os.path.getsize(path)
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(size))
        self.send_header("Content-Disposition", f'attachment; filename="{os.path.basename(path)}"')
        self.end_headers()
        with open(path, "rb") as f:
            shutil.copyfileobj(f, self.wfile)


def serve(port=8765, open_browser=True, host="127.0.0.1"):
    for d in (UPLOADS, OUTPUT):
        os.makedirs(d, exist_ok=True)
    srv = ThreadingHTTPServer((host, port), Handler)
    url = f"http://127.0.0.1:{srv.server_address[1]}/"
    print(f"HillView running at {url}   (files are saved under {OUTPUT})\nCtrl+C to quit.")
    if open_browser:
        webbrowser.open(url)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    return srv
