"""Audio output backends. Each pulls fixed-size chunks from a Mixer on its own thread.

  AplayOutput        pipes int16 PCM to `aplay` (alsa-utils, present on the SP). Backpressure from
                     the pipe paces the loop; the pipe is shrunk so it adds little latency.
  PygameQueueOutput  keeps two chunks queued on one pygame.mixer channel (SDL/ALSA, the path the
                     Hill apps already use).
  FileOutput         offline render to a WAV (tests, and later the mixdown export).

All of them expose .frame (frames rendered = the transport position), .stats() and
.latency_ms() (estimated, to be calibrated against the real device).
"""

from __future__ import annotations

import fcntl
import os
import subprocess
import threading
import time
import wave

from mixer import RATE


class _Stats(object):
    def __init__(self):
        self.chunks = 0
        self.underruns = 0
        self.late = 0                   # loop iterations slower than one chunk period
        self.render_ms_sum = 0.0
        self.render_ms_max = 0.0
        self.write_ms_max = 0.0
        self.errors = []

    def as_dict(self, period_ms):
        n = max(1, self.chunks)
        return {"chunks": self.chunks, "underruns": self.underruns, "late": self.late,
                "render_ms_avg": round(self.render_ms_sum / n, 3), "render_ms_max": round(self.render_ms_max, 3),
                "write_ms_max": round(self.write_ms_max, 2), "period_ms": round(period_ms, 2),
                "errors": self.errors[-3:]}


class _Base(object):
    def __init__(self, mixer, rate=RATE, period_frames=1024):
        self.mixer, self.rate, self.period = mixer, rate, period_frames
        self.frame = 0
        self.s = _Stats()
        self._stop = threading.Event()
        self._thread = None

    def stats(self):
        d = self.s.as_dict(1000.0 * self.period / self.rate)
        d["frame"] = self.frame
        return d

    def latency_ms(self):
        return 0.0

    def _render(self):
        t0 = time.perf_counter()
        data = self.mixer.render(self.frame, self.period)
        dt = (time.perf_counter() - t0) * 1000.0
        self.s.render_ms_sum += dt
        self.s.render_ms_max = max(self.s.render_ms_max, dt)
        return data

    def start(self):
        self._stop.clear()
        self._thread = threading.Thread(target=self._guard, name="audio-out", daemon=True)
        self._thread.start()

    def _guard(self):
        try:
            self._loop()
        except Exception as e:                      # never let the audio thread die silently
            self.s.errors.append(repr(e))

    def stop(self):
        self._stop.set()
        if self._thread:
            self._thread.join(2.0)
        self._cleanup()

    def _cleanup(self):
        pass


class FileOutput(_Base):
    """Renders `frames` frames as fast as possible into a 16-bit stereo WAV."""
    def __init__(self, mixer, path, frames, **kw):
        _Base.__init__(self, mixer, **kw)
        self.path, self.total = path, frames

    def _loop(self):
        with wave.open(self.path, "wb") as w:
            w.setnchannels(2)
            w.setsampwidth(2)
            w.setframerate(self.rate)
            while self.frame < self.total and not self._stop.is_set():
                n = min(self.period, self.total - self.frame)
                w.writeframes(self.mixer.render(self.frame, n))
                self.frame += n
                self.s.chunks += 1


class AplayOutput(_Base):
    def __init__(self, mixer, rate=RATE, period_frames=1024, buffer_ms=100, device=None,
                 cmd=None, pipe_bytes=8192):
        _Base.__init__(self, mixer, rate, period_frames)
        self.buffer_ms, self.device, self.pipe_bytes = buffer_ms, device, pipe_bytes
        self.cmd = cmd                              # override for tests (e.g. ["cat"])
        self.proc = None
        self._err_thread = None

    def latency_ms(self):
        return self.buffer_ms + 1000.0 * self.pipe_bytes / (self.rate * 4)

    def _argv(self):
        if self.cmd:
            return list(self.cmd)
        a = ["aplay", "-t", "raw", "-f", "S16_LE", "-c", "2", "-r", str(self.rate),
             "--buffer-time=%d" % (self.buffer_ms * 1000), "--period-time=%d" % (int(self.period * 1e6 / self.rate))]
        if self.device:
            a += ["-D", self.device]
        return a + ["-"]

    def _drain_stderr(self):
        for line in iter(self.proc.stderr.readline, b""):
            txt = line.decode("utf-8", "replace").strip()
            if "underrun" in txt.lower() or "xrun" in txt.lower():
                self.s.underruns += 1
            elif txt and not txt.startswith("Playing raw data"):      # aplay's start-up banner, not an error
                self.s.errors.append(txt)

    def start(self):
        self.proc = subprocess.Popen(self._argv(), stdin=subprocess.PIPE, stderr=subprocess.PIPE,
                                     stdout=subprocess.DEVNULL, bufsize=0)
        try:
            fcntl.fcntl(self.proc.stdin.fileno(), 1031, self.pipe_bytes)    # F_SETPIPE_SZ: bound the latency
        except (OSError, AttributeError):
            pass
        self._err_thread = threading.Thread(target=self._drain_stderr, daemon=True)
        self._err_thread.start()
        _Base.start(self)

    def _loop(self):
        period_s = self.period / float(self.rate)
        while not self._stop.is_set():
            t0 = time.perf_counter()
            data = self._render()
            t1 = time.perf_counter()
            try:
                self.proc.stdin.write(data)
            except (BrokenPipeError, OSError) as e:
                self.s.errors.append("aplay pipe: %r" % e)
                return
            t2 = time.perf_counter()
            self.s.write_ms_max = max(self.s.write_ms_max, (t2 - t1) * 1000.0)
            self.frame += self.period
            self.s.chunks += 1
            if t2 - t0 > period_s * 1.5 and self.s.chunks > 4:
                self.s.late += 1

    def _cleanup(self):
        p = self.proc
        if p is None:
            return
        try:
            p.stdin.close()
        except OSError:
            pass
        try:
            p.wait(timeout=2)
        except subprocess.TimeoutExpired:
            p.kill()
        self.proc = None


class PygameQueueOutput(_Base):
    def __init__(self, mixer, rate=RATE, period_frames=1024, sdl_buffer=512):
        _Base.__init__(self, mixer, rate, period_frames)
        self.sdl_buffer = sdl_buffer
        self.pg = None
        self.ch = None

    def latency_ms(self):
        return 1000.0 * (self.sdl_buffer + 2 * self.period) / self.rate     # SDL buffer + queued chunks

    def start(self):
        import pygame
        self.pg = pygame
        pygame.mixer.pre_init(self.rate, -16, 2, self.sdl_buffer)
        pygame.mixer.init(self.rate, -16, 2, self.sdl_buffer)
        self.ch = pygame.mixer.Channel(0)
        _Base.start(self)

    def _loop(self):
        pg, ch = self.pg, self.ch
        started = False
        while not self._stop.is_set():
            if ch.get_queue() is None:
                data = self._render()
                snd = pg.mixer.Sound(buffer=data)
                if not ch.get_busy():
                    if started:
                        self.s.underruns += 1           # the channel ran dry before we refilled it
                    ch.play(snd)
                    started = True
                else:
                    ch.queue(snd)
                self.frame += self.period
                self.s.chunks += 1
            else:
                time.sleep(0.002)

    def _cleanup(self):
        try:
            self.pg.mixer.quit()
        except Exception:
            pass
