"""Integration tests: import/export round trips and the app's HTTP API. Needs ffmpeg."""
import json
import os
import shutil
import subprocess
import threading
import time
import urllib.error
import urllib.request

import numpy as np
import pytest

pytestmark = pytest.mark.skipif(not shutil.which("ffmpeg"), reason="ffmpeg required")


def ff(*a):
    subprocess.run(["ffmpeg", "-y", "-v", "error", *a], check=True)


@pytest.fixture(scope="module")
def clip(tmp_path_factory):
    d = tmp_path_factory.mktemp("clips")
    p = str(d / "in.mp4")
    ff("-f", "lavfi", "-i", "testsrc2=size=160x96:rate=24:duration=1.5", "-f", "lavfi",
       "-i", "sine=f=110:d=1.5", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", p)
    return p


def test_probe_and_odd_size(tmp_path):
    from hillview import video_io
    p = str(tmp_path / "odd.mp4")
    ff("-f", "lavfi", "-i", "color=red:size=161x97:rate=10:duration=1", "-c:v", "libx264", "-pix_fmt", "yuv444p", p)
    i = video_io.probe(p)
    assert (i["width"], i["height"], i["frames"]) == (160, 96, 10) and not i["has_audio"]


def test_rotation_metadata_swaps_dims(tmp_path):
    from hillview import video_io
    a, b = str(tmp_path / "a.mp4"), str(tmp_path / "b.mp4")
    ff("-f", "lavfi", "-i", "testsrc2=size=160x96:rate=10:duration=1", "-c:v", "libx264", "-pix_fmt", "yuv420p", a)
    ff("-display_rotation", "90", "-i", a, "-c", "copy", b)
    i = video_io.probe(b)
    assert (i["width"], i["height"]) == (96, 160)
    s = video_io.Source(b, 4)
    assert s.get(0).shape == (160, 96, 3)
    s.close()


@pytest.mark.parametrize("fmt", ["h264", "h265", "vp9", "prores_422", "mjpeg", "ffv1", "mpeg4", "gif"])
def test_export_roundtrip_every_container(clip, tmp_path, fmt):
    from hillview import video_io
    from hillview.render import render_video
    av = video_io.available_formats()
    if fmt not in av:
        pytest.skip("encoder missing")
    out = str(tmp_path / f"o.{av[fmt]['ext']}")
    n = render_video(clip, out, {"quantize": {"enabled": True, "levels": 3}}, fmt)
    assert n == 36
    back = video_io.probe(out)  # re-import what we exported
    assert (back["width"], back["height"]) == (160, 96) and abs(back["frames"] - 36) <= 1


def test_dnxhr_needs_min_size_with_clear_error_and_works_when_big_enough(clip, tmp_path):
    from hillview import video_io
    from hillview.render import render_video
    with pytest.raises(ValueError, match="at least 256x120"):
        render_video(clip, str(tmp_path / "x.mov"), {}, "dnxhr_hq")
    big = str(tmp_path / "big.mp4")
    ff("-f", "lavfi", "-i", "testsrc2=size=320x180:rate=10:duration=0.5", "-c:v", "libx264", "-pix_fmt", "yuv420p", big)
    out = str(tmp_path / "big.mov")
    assert render_video(big, out, {}, "dnxhr_hq") == 5
    assert video_io.probe(out)["width"] == 320


def test_animated_webp_export_is_valid_riff(clip, tmp_path):
    # ffmpeg's decoder can't read animated WebP back, so check the container structure instead
    from hillview.render import render_video
    out = str(tmp_path / "a.webp")
    render_video(clip, out, {}, "webp")
    head = open(out, "rb").read(64)
    assert head[:4] == b"RIFF" and head[8:12] == b"WEBP" and b"ANIM" in head


def test_audio_is_muxed_and_separate_audio_replaces(clip, tmp_path):
    from hillview import video_io
    from hillview.render import render_video
    out = str(tmp_path / "a.mp4")
    render_video(clip, out, {}, "h264")
    assert video_io.probe(out)["has_audio"]
    wav = str(tmp_path / "m.wav")
    ff("-f", "lavfi", "-i", "sine=f=440:d=3", wav)
    out2 = str(tmp_path / "b.mov")
    render_video(clip, out2, {}, "prores_422", audio_path=wav)
    assert video_io.probe(out2)["has_audio"]


def test_image_sequence_in_and_out(clip, tmp_path):
    from hillview import video_io
    from hillview.render import render_video
    seq = str(tmp_path / "seq")
    n = render_video(clip, seq, {}, "png_seq")
    assert len(os.listdir(seq)) == n == 36
    out = str(tmp_path / "from_seq.mp4")
    assert render_video(os.path.join(seq, "frame_%05d.png"), out, {}, "h264", fps=24) == 36
    assert video_io.probe(out)["fps"] == 24


def test_variable_frame_rate_is_conformed(tmp_path):
    from hillview import video_io
    from hillview.render import render_video
    vfr = str(tmp_path / "vfr.mp4")
    ff("-f", "lavfi", "-i", "testsrc2=size=64x64:rate=30:duration=1", "-vf", "setpts='N/30/TB+(random(0)*0.01)/TB'",
       "-fps_mode", "vfr", "-c:v", "libx264", "-pix_fmt", "yuv420p", vfr)
    out = str(tmp_path / "cfr.mp4")
    assert abs(render_video(vfr, out, {}, "h264") - 30) <= 2


def test_single_image_import(tmp_path):
    from hillview import video_io
    p = str(tmp_path / "x.png")
    ff("-f", "lavfi", "-i", "testsrc2=size=64x48", "-frames:v", "1", p)
    assert video_io.probe(p)["frames"] == 1


def test_bad_file_is_a_clean_error(tmp_path):
    from hillview import video_io
    p = tmp_path / "junk.mp4"
    p.write_bytes(b"not a video")
    with pytest.raises(ValueError, match="cannot read"):
        video_io.probe(str(p))


# ------------------------------------------------------------------ HTTP API
@pytest.fixture(scope="module")
def server(tmp_path_factory):
    os.environ["HILLVIEW_HOME"] = str(tmp_path_factory.mktemp("home"))
    import importlib
    from hillview import app
    importlib.reload(app)
    from http.server import ThreadingHTTPServer
    srv = ThreadingHTTPServer(("127.0.0.1", 0), app.Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{srv.server_address[1]}", app
    srv.shutdown()


def call(base, app, path, body=None, token=True, host=None, raw=False):
    h = {"X-HV-Token": app.TOKEN} if token else {}
    if host:
        h["Host"] = host
    data = body if raw else (json.dumps(body).encode() if body is not None else None)
    req = urllib.request.Request(base + path, data=data, headers=h)
    try:
        with urllib.request.urlopen(req) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def test_api_rejects_missing_token_and_bad_host(server):
    base, app = server
    assert call(base, app, "/api/formats", token=False)[0] == 403
    assert call(base, app, "/api/formats", host="evil.example.com")[0] == 403
    assert call(base, app, "/api/formats")[0] == 200


def test_api_open_preview_render_download(server, clip):
    base, app = server
    c, b = call(base, app, "/api/open", {"path": clip})
    assert c == 200
    sess = json.loads(b)
    assert sess["info"]["frames"] == 36 and sess["audio"]
    cfg = {"smooth": {"sigma": 2}, "modulation": [{"source": {"type": "audio", "channel": "rms"},
                                                   "target": "blur.sigma", "depth": 4}]}
    c, png = call(base, app, "/api/preview", {"id": sess["id"], "frame": 5, "config": cfg})
    assert c == 200 and png[:8] == b"\x89PNG\r\n\x1a\n"
    c, orig = call(base, app, "/api/preview", {"id": sess["id"], "frame": 5, "original": True})
    assert c == 200 and orig != png
    c, b = call(base, app, "/api/render", {"id": sess["id"], "config": cfg, "format": "vp9", "name": "../../evil"})
    job = json.loads(b)["job"]
    for _ in range(200):
        st = json.loads(call(base, app, f"/api/job?id={job}")[1])
        if st["status"] != "running":
            break
        time.sleep(0.1)
    assert st["status"] == "done", st
    assert os.path.dirname(st["path"]) == app.OUTPUT  # path traversal in `name` neutralised
    c, data = call(base, app, f"/api/download?id={job}")
    assert c == 200 and len(data) > 1000


def test_api_errors_are_json(server):
    base, app = server
    c, b = call(base, app, "/api/open", {"path": "/nonexistent/x.mp4"})
    assert c == 400 and "not found" in json.loads(b)["error"]
    c, b = call(base, app, "/api/preview", {"id": "nope"})
    assert c == 404


def test_upload_then_open(server, clip):
    base, app = server
    c, b = call(base, app, "/api/upload?name=my%20clip.mp4", open(clip, "rb").read(), raw=True)
    path = json.loads(b)["path"]
    assert c == 200 and path.endswith("my clip.mp4") and path.startswith(app.UPLOADS)
    assert call(base, app, "/api/open", {"path": path})[0] == 200
