"""Polyphonic sample playback over pygame's 12-channel mixer.

A "sound" (timbre) is a folder of per-pitch WAV files (spec: single timbre
across all notes, no runtime pitch-shifting). Filenames are parsed to MIDI
note numbers; a requested note plays the nearest available sample.

At note-on, effects.apply_effects() bakes the active reverb/delay/chorus +
wet/dry into the voice (Option D), with a small cache keyed by (midi, fx).
"""

import glob
import hashlib
import os
import re
import time
import wave
from collections import OrderedDict

import numpy as np
import pygame

import config
from audio import effects

_NAME_PC = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}
_DEFAULT_OCTAVE = 3       # assumed octave when a filename gives only a pitch class

# Note + octave embedded anywhere ('C4', 'F#2' in 'Piano.ff.C4', 'cello_F#2_v2'),
# bounded so it isn't part of a longer word or a 'mf1' dynamic.
_NOTE_OCT = re.compile(r"(?:^|[^A-Za-z0-9])([A-Ga-g])([#sb]?)(-?[0-9])(?=$|[^0-9])")
# A bare pitch-class TOKEN (no octave), optional trailing 'm' for minor keys:
# 'Db', 'A', 'Gb', 'Gm' as a standalone token ('FMLead_Z_013_Db'). Token-bounded
# so words like 'Belli' / 'Drum' / 'anotherworld' aren't misread as notes.
_PC_TOKEN = re.compile(r"(?:^|[^A-Za-z0-9])([A-Ga-g])([#sb]?)(m?)(?=$|[^A-Za-z0-9])")


def _pc_value(letter, accidental):
    pc = _NAME_PC[letter.upper()]
    if accidental in ("#", "s"):
        pc += 1
    elif accidental == "b":
        pc -= 1
    return pc


def _find_note(stem: str):
    """Locate the pitch in a filename stem -> (midi, start, end) or None.
    Prefers an explicit note+octave; falls back to a bare pitch-class token
    (mapped to a default octave) for libraries that omit the octave."""
    m = _NOTE_OCT.search(stem)
    if m:
        midi = (int(m.group(3)) + 1) * 12 + _pc_value(m.group(1), m.group(2))
        return midi, m.start(1), m.end(3)
    m = _PC_TOKEN.search(stem)
    if m:
        midi = (_DEFAULT_OCTAVE + 1) * 12 + (_pc_value(m.group(1), m.group(2)) % 12)
        return midi, m.start(1), m.end(3)   # span covers optional trailing 'm'
    return None


def _parse_midi(filename: str):
    """Map a WAV filename to a MIDI note number, or None. Accepts a bare MIDI
    number ('060.wav'), a note+octave ('Vibraphone_C4_mf1.wav'), or a bare
    pitch class ('FMLead_Z_013_Db.wav')."""
    stem = os.path.splitext(os.path.basename(filename))[0]
    if stem.lstrip("-").isdigit():
        return int(stem)
    f = _find_note(stem)
    return f[0] if f else None


class SoundBank:
    """Loads a set of per-pitch WAV files: MIDI note -> raw int16 stereo PCM."""

    def __init__(self, files):
        self.pcm = {}        # midi -> np.ndarray (n,2) int16
        self._midis = []
        for path in files:
            midi = _parse_midi(path)
            if midi is None:
                continue
            try:
                snd = pygame.mixer.Sound(path)
                arr = pygame.sndarray.array(snd)
                if arr.ndim == 1:
                    arr = np.stack([arr, arr], axis=1)
                self.pcm[midi] = arr.astype(np.int16)
            except (pygame.error, ValueError) as e:
                print(f"[hillchord] skip {path}: {e}")
        self._midis = sorted(self.pcm)

    def nearest(self, midi: int):
        """Nearest available sampled pitch (no pitch-shift)."""
        if not self._midis:
            return None
        return min(self._midis, key=lambda m: abs(m - midi))


RELEASE_MS = 120   # fade-out on note release (click-free)
RETRIGGER_MS = 10  # near-instant cut when re-triggering a still-sounding voice
                   # (avoids same-frequency phase interference -> random volume)

# Delay (echo) — scheduled note repeats, one per beat. Real beat-spaced echoes
# that work on sustained drones, vs. baking a delay into the looped buffer.
DELAY_TAPS = 4
DELAY_FEEDBACK = 0.55


def _write_wav_cache(path: str, pcm: np.ndarray) -> None:
    """Persist a rendered int16 stereo buffer to the on-disk render cache."""
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        a = pcm if pcm.ndim == 2 else np.stack([pcm, pcm], axis=1)
        # Unique temp name so the app and a concurrent prerender process never
        # clobber each other's temp file before the atomic replace.
        tmp = f"{path}.{os.getpid()}.tmp"
        with wave.open(tmp, "w") as w:
            w.setnchannels(2)
            w.setsampwidth(2)
            w.setframerate(config.SAMPLE_RATE)
            w.writeframes(np.ascontiguousarray(a.astype(np.int16)).tobytes())
        os.replace(tmp, path)
    except OSError as e:
        print(f"[hillchord] render cache write failed: {e}")


def _norm_gain(arrays) -> float:
    """RMS-match gain for a set of int16 buffers, limited so the peak stays
    below NORM_PEAK_LIMIT and quiet samples aren't over-amplified."""
    sq = 0.0
    count = 0
    peak = 1.0
    for a in arrays:
        af = a.astype(np.float64)
        sq += float(np.sum(af * af))
        count += af.size
        peak = max(peak, float(np.abs(a).max()))
    if count == 0:
        return 1.0
    rms = (sq / count) ** 0.5
    if rms < 1.0:
        return 1.0
    g = (config.NORM_TARGET_RMS * 32768.0) / rms
    g = min(g, (config.NORM_PEAK_LIMIT * 32768.0) / peak)  # peak limit
    return max(0.05, min(g, config.NORM_MAX_GAIN))


def _apply_gain(a: np.ndarray, g: float) -> np.ndarray:
    if abs(g - 1.0) < 1e-3:
        return a
    return np.clip(a.astype(np.float32) * g, -32768, 32767).astype(np.int16)


def _is_sustaining(pcm: np.ndarray) -> bool:
    """Heuristic: does the sample hold a sustained body (drone/pad/organ -> loop)
    or decay continuously like a pluck (one-shot -> play once and ring out)?

    Compares early vs middle energy: a pluck decays the whole way (middle is much
    lower than early); a sustained note keeps a flat body (middle ~= early).
    Ignores the very end so a natural release tail doesn't fool it."""
    a = np.abs(pcm[:, 0].astype(np.float32)) if pcm.ndim == 2 else np.abs(pcm.astype(np.float32))
    n = len(a)
    if n < 4000:
        return False
    early = a[int(n * 0.10):int(n * 0.25)].mean() + 1e-9
    mid = a[int(n * 0.45):int(n * 0.65)].mean()
    return mid > 0.5 * early


def parse_root(filename: str, default: int = 60) -> int:
    """Guess a single-sample's root MIDI note from its filename.

    Priority:
      1. An explicit note+octave token ('Pad_A2' -> A2, 'drone_F#3' -> F#3),
         including a bare MIDI number ('060').
      2. A bare key tag with no octave ('120_A_IcePad', '75_Gb_Pad') -> octave 3.
      3. `default` (C4) if nothing recognizable.
    """
    # 1) explicit octave (reuses the embedded note-token parser).
    m = _parse_midi(filename)
    if m is not None:
        return m
    # 2) bare key letter (no octave) -> assume octave 3.
    stem = os.path.splitext(os.path.basename(filename))[0]
    for tok in re.split(r"[ _\-.]+", stem):
        km = re.fullmatch(r"([A-Ga-g])([#b]?)m?", tok)
        if km:
            pc = _NAME_PC[km.group(1).upper()]
            if km.group(2) == "#":
                pc += 1
            elif km.group(2) == "b":
                pc -= 1
            return (3 + 1) * 12 + (pc % 12)   # root at octave 3
    return default


class Mixer:
    def __init__(self):
        self.bank = None
        self.mode = "multi"          # "multi" (per-pitch folder) | "single"
        self._single_pcm = None      # source PCM for single-sample mode
        self._single_root = 60
        self._loop_mode = True       # held note loops (drone) vs plays once (pluck)
        self._token = ""             # portable id of the loaded sound (for disk cache)
        self._cache = OrderedDict()  # LRU: key -> (Sound, nbytes)
        self._cache_bytes = 0
        self._next_channel = 0
        self._voices = {}    # voice_id -> list of channel indices (sustained)
        self._echoes = []    # pending (fire_time, sound, gain) delay repeats
        self._last_compute = 0.0   # monotonic time of last from-scratch render

    def computing_recently(self, window=0.6):
        """True briefly after a first-time (uncached) render, for UI feedback."""
        return (time.monotonic() - self._last_compute) < window

    def load_sound(self, folder: str, token: str = None):
        """Load a per-pitch multisample folder (all WAVs under it)."""
        files = glob.glob(os.path.join(folder, "**", "*.wav"), recursive=True)
        self.load_files(files, token or os.path.basename(folder.rstrip("/")))

    def load_files(self, files, token: str = None):
        """Load a multisample from a specific list of WAV files (one instrument)."""
        self.bank = SoundBank(files)
        self.mode = "multi"
        if config.NORMALIZE and self.bank.pcm:
            g = _norm_gain(self.bank.pcm.values())
            for k in self.bank.pcm:
                self.bank.pcm[k] = _apply_gain(self.bank.pcm[k], g)
        # Loop drones; let plucked/decaying instruments ring out. Majority vote
        # across pads so one ringing sample (e.g. a hat in a drum kit) doesn't
        # flip a whole kit into loop mode.
        mids = sorted(self.bank.pcm)
        votes = [_is_sustaining(self.bank.pcm[m]) for m in mids]
        self._loop_mode = bool(votes) and sum(votes) * 2 > len(votes)
        self._token = token or (files[0] if files else "")
        self._cache_clear()

    def load_single(self, wav_path: str, root: int = None, token: str = None):
        """Load one WAV as a pitch-shifted instrument across the keyboard."""
        try:
            snd = pygame.mixer.Sound(wav_path)
            arr = pygame.sndarray.array(snd)
            if arr.ndim == 1:
                arr = np.stack([arr, arr], axis=1)
            self._single_pcm = arr.astype(np.int16)
        except (pygame.error, ValueError) as e:
            print(f"[hillchord] could not load {wav_path}: {e}")
            return
        if config.NORMALIZE:
            self._single_pcm = _apply_gain(self._single_pcm,
                                           _norm_gain([self._single_pcm]))
        self._loop_mode = _is_sustaining(self._single_pcm)
        self._single_root = parse_root(wav_path) if root is None else root
        self.mode = "single"
        self._token = token or os.path.basename(wav_path)
        self._cache_clear()

    # ---- RAM cache (bounded LRU) ----
    def _cache_clear(self):
        self._cache.clear()
        self._cache_bytes = 0

    def _lru_get(self, key):
        v = self._cache.get(key)
        if v is None:
            return None
        self._cache.move_to_end(key)
        return v[0]

    def _lru_put(self, key, snd, nbytes):
        self._cache[key] = (snd, nbytes)
        self._cache.move_to_end(key)
        self._cache_bytes += nbytes
        while self._cache_bytes > config.RAM_CACHE_BYTES and len(self._cache) > 1:
            _k, (_s, n) = self._cache.popitem(last=False)
            self._cache_bytes -= n

    @staticmethod
    def _resample(pcm: np.ndarray, semitones: float) -> np.ndarray:
        """Pitch-shift by resampling (linear interp). +semitones -> higher."""
        if semitones == 0:
            return pcm
        ratio = 2.0 ** (semitones / 12.0)
        n = len(pcm)
        new_n = max(1, int(round(n / ratio)))
        x = np.linspace(0, n - 1, new_n)
        i0 = np.floor(x).astype(np.int32)
        i1 = np.minimum(i0 + 1, n - 1)
        frac = (x - i0).astype(np.float32)[:, None]
        out = pcm[i0].astype(np.float32) * (1 - frac) + pcm[i1].astype(np.float32) * frac
        return out.astype(np.int16)

    def _free_index(self):
        """A channel index not currently held by a sustained voice."""
        used = set()
        for idxs in self._voices.values():
            used.update(idxs)
        for i in range(config.NUM_VOICES):
            if i not in used and not pygame.mixer.Channel(i).get_busy():
                return i
        for i in range(config.NUM_VOICES):
            if i not in used:
                return i
        i = self._next_channel
        self._next_channel = (i + 1) % config.NUM_VOICES
        return i

    def play_voice(self, voice_id, midis, fx, bpm):
        """Start a sustained (looping) voice; hold = drone, release = stop."""
        # Quick cut of any prior instance so a retrigger doesn't overlap and
        # beat against itself.
        self.stop_voice(voice_id, fade=RETRIGGER_MS)
        loop = self._loop_mode
        # Render ALL notes first (the slow part on a first-ever, uncached chord),
        # then start them together so the chord doesn't arrive note-by-note.
        snds = [self._render(midi, fx, bpm, loop=loop) for midi in midis]
        idxs = []
        for snd in snds:
            if snd is None:
                continue
            i = self._free_index()
            ch = pygame.mixer.Channel(i)
            ch.set_volume(1.0)            # reset any leftover echo gain
            ch.play(snd, loops=-1 if loop else 0)
            idxs.append(i)
        if idxs:
            self._voices[voice_id] = idxs
        self._schedule_echoes(midis, fx, bpm)

    def stop_voice(self, voice_id, fade=RELEASE_MS):
        for i in self._voices.pop(voice_id, []):
            pygame.mixer.Channel(i).fadeout(max(1, fade))

    @staticmethod
    def _fx_key(fx, loop: bool):
        # Delay is scheduled (not baked), so only reverb/chorus/wet + bitcrush
        # affect the rendered buffer.
        tonal = (fx.reverb or fx.chorus) and fx.wetdry > 0
        if not tonal and not fx.crushing():
            return ("dry", loop)
        return (fx.reverb, fx.chorus, fx.wetdry, fx.crush_bits, fx.crush_down, loop)

    def _cache_id(self, midi: int):
        """Cheap identity for the cache (no resample); None if nothing loaded."""
        if self.mode == "single":
            return ("single", midi) if self._single_pcm is not None else None
        if self.bank is None:
            return None
        src = self.bank.nearest(midi)
        # Keyed by (nearest sample, semitone shift) so every requested note is a
        # distinct pitch even when the library is sparsely sampled.
        return ("multi", src, midi - src) if src is not None else None

    def _source_pcm(self, midi: int):
        """Raw int16 stereo for this note, pitch-shifted to the exact pitch.

        Multisample libraries are usually sampled every few semitones, so we
        pick the nearest sample and resample it to the requested note (a no-op
        when an exact sample exists, e.g. fully chromatic libraries)."""
        if self.mode == "single":
            semi = midi - self._single_root
            # Loops keep their tempo (time-stretch); one-shots repitch normally.
            if config.PITCH_PRESERVE_TEMPO and self._loop_mode:
                return effects.pitch_shift_preserve(self._single_pcm, semi)
            return self._resample(self._single_pcm, semi)
        src = self.bank.nearest(midi)
        return self._resample(self.bank.pcm[src], midi - src)

    def _compute_buffer(self, midi, fx, bpm, loop):
        """The expensive part: pitch-shift + bake effects -> int16 stereo."""
        pcm = self._source_pcm(midi)
        if loop:
            xfade = int(config.SAMPLE_RATE * config.LOOP_XFADE_MS / 1000)
            pcm = effects.loop_xfade(pcm, xfade)
        return effects.apply_effects(pcm, fx, bpm, config.SAMPLE_RATE, loop=loop)

    def _disk_path(self, cid, fxkey, loop):
        h = hashlib.md5(repr((self._token, cid, fxkey, loop)).encode()).hexdigest()
        # Shard into 256 subdirs by hash prefix: a flat dir with thousands of
        # files makes exFAT directory lookups crawl (every note-on stats here).
        return os.path.join(config.RENDER_CACHE_DIR, h[:2], h + ".wav")

    def _render(self, midi: int, fx, bpm: int, loop: bool = False):
        cid = self._cache_id(midi)
        if cid is None:
            return None
        fxkey = self._fx_key(fx, loop)
        key = (cid, fxkey)

        # 1) RAM (instant).
        snd = self._lru_get(key)
        if snd is not None:
            return snd

        # 2) Disk (fast: a file read, no DSP). Trades storage for compute/RAM.
        path = self._disk_path(cid, fxkey, loop)
        if os.path.exists(path):
            try:
                snd = pygame.mixer.Sound(path)
                self._lru_put(key, snd, os.path.getsize(path))
                return snd
            except pygame.error:
                pass

        # 3) Compute (slow, one-time), persist to disk, cache in RAM.
        self._last_compute = time.monotonic()
        buf = self._compute_buffer(midi, fx, bpm, loop)
        _write_wav_cache(path, buf)
        snd = pygame.sndarray.make_sound(np.ascontiguousarray(buf))
        self._lru_put(key, snd, buf.nbytes)
        return snd

    def prerender(self, midis, fx, bpm, loop=None, cancel=None, to_disk_only=False):
        """Warm the cache for a set of notes (e.g. at load, or offline tool).

        With to_disk_only=True it skips creating pygame Sounds (safe to call
        before audio is needed / from a build script) and just writes the disk
        cache, so first play is a fast disk load."""
        loop = self._loop_mode if loop is None else loop
        for midi in midis:
            if cancel is not None and cancel():
                return
            cid = self._cache_id(midi)
            if cid is None:
                continue
            fxkey = self._fx_key(fx, loop)
            path = self._disk_path(cid, fxkey, loop)
            if os.path.exists(path):
                continue
            buf = self._compute_buffer(midi, fx, bpm, loop)
            _write_wav_cache(path, buf)
            if not to_disk_only:
                self._lru_put((cid, fxkey),
                              pygame.sndarray.make_sound(np.ascontiguousarray(buf)),
                              buf.nbytes)

    def play_oneshot(self, midi, fx, bpm):
        """Play a single note one-shot; return its channel index (for the arp to
        gate it). None if nothing to play."""
        snd = self._render(midi, fx, bpm, loop=False)
        if snd is None:
            return None
        i = self._free_index()
        ch = pygame.mixer.Channel(i)
        ch.set_volume(1.0)
        ch.play(snd)
        self._schedule_echoes([midi], fx, bpm)
        return i

    def fade_index(self, i, ms=RELEASE_MS):
        if i is not None:
            pygame.mixer.Channel(i).fadeout(max(1, ms))

    # ---- Delay: scheduled, beat-spaced note repeats ----
    def _schedule_echoes(self, midis, fx, bpm):
        if not fx.delay or bpm <= 0:
            return
        step = 60.0 / bpm                 # one quarter-note beat, in seconds
        now = time.monotonic()
        for k in range(1, DELAY_TAPS + 1):
            gain = DELAY_FEEDBACK ** k
            if gain < 0.02:
                break
            for midi in midis:
                snd = self._render(midi, fx, bpm, loop=False)
                if snd is not None:
                    self._echoes.append((now + step * k, snd, gain))

    def update(self):
        """Fire any due delay echoes. Call once per frame from the main loop."""
        if not self._echoes:
            return
        now = time.monotonic()
        pending = []
        for fire_t, snd, gain in self._echoes:
            if fire_t <= now:
                ch = pygame.mixer.Channel(self._free_index())
                ch.play(snd)
                ch.set_volume(gain)
            else:
                pending.append((fire_t, snd, gain))
        self._echoes = pending

    def stop_all(self):
        pygame.mixer.stop()
        self._echoes = []
        self._voices.clear()

    def panic(self):
        """Silence stuck voices + pending echoes, leaving loop/metronome alone."""
        for i in range(config.NUM_VOICES):
            pygame.mixer.Channel(i).stop()
        self._voices.clear()
        self._echoes = []
