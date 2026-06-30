"""
HillBeat — loop_player.py
Loads, timestretch-es (via pyrubberband), and loops a WAV file.
"""

from __future__ import annotations

import os
import re
import wave
import struct
import numpy as np
import pygame

from config import (
    LOOP_CHANNEL, LOOP_CACHE, SAMPLE_RATE,
)

_rubberband_available = False
try:
    import pyrubberband as pyrb
    _rubberband_available = True
except ImportError:
    pass


def _parse_native_bpm(filepath: str) -> int | None:
    """Extract BPM from filenames like loop_name_120bpm.wav"""
    m = re.search(r'(\d+)bpm', os.path.basename(filepath), re.IGNORECASE)
    if m:
        return int(m.group(1))
    return None


def _read_wav(filepath: str):
    """Return (samples_float32 ndarray shape [n, channels], samplerate)."""
    with wave.open(filepath, 'rb') as wf:
        n_channels   = wf.getnchannels()
        sampwidth    = wf.getsampwidth()
        framerate    = wf.getframerate()
        n_frames     = wf.getnframes()
        raw          = wf.readframes(n_frames)

    if sampwidth == 2:
        fmt = f"<{len(raw)//2}h"
        samples = np.array(struct.unpack(fmt, raw), dtype=np.float32) / 32768.0
    elif sampwidth == 4:
        fmt = f"<{len(raw)//4}i"
        samples = np.array(struct.unpack(fmt, raw), dtype=np.float32) / 2147483648.0
    elif sampwidth == 1:
        fmt = f"<{len(raw)}B"
        samples = (np.array(struct.unpack(fmt, raw), dtype=np.float32) - 128.0) / 128.0
    else:
        raise ValueError(f"Unsupported sample width: {sampwidth}")

    if n_channels > 1:
        samples = samples.reshape(-1, n_channels)
    else:
        samples = samples.reshape(-1, 1)

    return samples, framerate


def _write_wav(filepath: str, samples: np.ndarray, samplerate: int):
    """Write float32 ndarray (n, ch) to a WAV file at 16-bit."""
    clipped = np.clip(samples, -1.0, 1.0)
    pcm = (clipped * 32767).astype(np.int16)
    n_ch = pcm.shape[1] if pcm.ndim > 1 else 1
    if pcm.ndim == 1:
        pcm = pcm.reshape(-1, 1)

    with wave.open(filepath, 'wb') as wf:
        wf.setnchannels(n_ch)
        wf.setsampwidth(2)
        wf.setframerate(samplerate)
        wf.writeframes(pcm.tobytes())


class LoopPlayer:
    """
    Manages a single looping WAV channel (pygame mixer channel LOOP_CHANNEL).
    Timestretch is done synchronously on load; the stretched file is cached.
    """

    def __init__(self, state, transport):
        self.state     = state
        self.transport = transport

        self._channel = pygame.mixer.Channel(LOOP_CHANNEL)
        self._sound   = None
        self._loaded_bpm   = None
        self._loaded_file  = None
        self.stretch_warning = False   # True if pyrubberband unavailable

    # ── Public API ────────────────────────────────────────────────────────────

    def load(self, filepath: str | None, native_bpm: int | None = None):
        """
        Load and timestretch a loop file to match current transport BPM.
        native_bpm can be inferred from the filename if not supplied.
        """
        self._channel.stop()
        self._sound = None
        self.stretch_warning = False

        if filepath is None:
            self._loaded_file = None
            self._loaded_bpm  = None
            self.state.loop_file       = None
            self.state.loop_native_bpm = None
            return

        if native_bpm is None:
            native_bpm = _parse_native_bpm(filepath)

        self.state.loop_file       = filepath
        self.state.loop_native_bpm = native_bpm
        self._loaded_file = filepath
        self._loaded_bpm  = self.transport.bpm

        target_path = self._stretch_to_cache(filepath, native_bpm)
        try:
            self._sound = pygame.mixer.Sound(target_path)
        except Exception as e:
            print(f"[loop_player] Failed to load sound: {e}")
            self._sound = None

        if self.state.loop_enabled and self._sound:
            self._channel.play(self._sound, loops=-1)

    def reload_if_bpm_changed(self):
        """Call this when BPM changes; reloads only if needed."""
        if self._loaded_file and self._loaded_bpm != self.transport.bpm:
            self.load(self._loaded_file, self.state.loop_native_bpm)

    def enable(self, enabled: bool):
        self.state.loop_enabled = enabled
        if enabled and self._sound:
            if not self._channel.get_busy():
                self._channel.play(self._sound, loops=-1)
        else:
            self._channel.stop()

    def toggle(self):
        self.enable(not self.state.loop_enabled)

    def position_fraction(self) -> float:
        """
        Rough playback position 0.0–1.0 within the current loop cycle.
        pygame doesn't expose true frame position, so we approximate via time.
        """
        if self._sound is None or not self._channel.get_busy():
            return 0.0
        import time
        # Use sound length as denominator
        length = self._sound.get_length()
        if length <= 0:
            return 0.0
        # We don't have a true start timestamp, so tick modulo length
        pos = (time.monotonic() % length) / length
        return pos

    # ── Internal ──────────────────────────────────────────────────────────────

    def _stretch_to_cache(self, filepath: str, native_bpm: int | None) -> str:
        """Timestretch filepath to current BPM, write to LOOP_CACHE, return path."""
        target_bpm = self.transport.bpm

        # No stretch needed
        if native_bpm is None or native_bpm == target_bpm:
            return filepath

        ratio = native_bpm / target_bpm

        if not _rubberband_available:
            self.stretch_warning = True
            print("[loop_player] pyrubberband not available — playing at native tempo")
            return filepath

        try:
            samples, sr = _read_wav(filepath)
            # pyrubberband expects (n,) for mono or (n, channels) for stereo
            if samples.shape[1] == 1:
                samples = samples[:, 0]
            stretched = pyrb.time_stretch(samples, sr, ratio)
            # Re-add channel dim if mono so _write_wav stays uniform
            if stretched.ndim == 1:
                stretched = stretched.reshape(-1, 1)
            _write_wav(LOOP_CACHE, stretched, sr)
            return LOOP_CACHE
        except Exception as e:
            print(f"[loop_player] Stretch failed: {e}")
            self.stretch_warning = True
            return filepath
