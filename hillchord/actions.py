"""Resolve an InputFrame into state mutations and audio triggers.

This is the only module that mutates AppState in response to input. It reads
the per-frame picture from ComboHandler and calls into theory + audio.
"""

import time

import config
from audio import loop_recorder as lr
from audio.arpeggiator import MODES as ARP_MODES
from input import button_map as bm
from input import combo_handler as ch
from theory import chords, notes, keys

# Held face-button set -> chord scale degree (Chord Mode).
_CHORD_COMBOS = {
    frozenset({bm.A}): 1,           # I
    frozenset({bm.B}): 4,           # IV
    frozenset({bm.X}): 5,           # V
    frozenset({bm.Y}): 6,           # vi
    frozenset({bm.A, bm.B}): 2,     # ii
    frozenset({bm.A, bm.X}): 3,     # iii
    frozenset({bm.B, bm.Y}): 7,     # vii(dim)
}

# Effective D-pad set -> chord modifier (Chord Mode). UP alone is special
# (flips key tonality) and is handled separately.
_DPAD_MOD = {
    frozenset({bm.UP, bm.RIGHT}): chords.MOD_DOM7,
    frozenset({bm.RIGHT}): chords.MOD_MAJ7,
    frozenset({bm.RIGHT, bm.DOWN}): chords.MOD_ADD9,
    frozenset({bm.DOWN}): chords.MOD_SUS4,
    frozenset({bm.DOWN, bm.LEFT}): chords.MOD_SIXTH,
    frozenset({bm.LEFT}): chords.MOD_DIM,
    frozenset({bm.UP, bm.LEFT}): chords.MOD_AUG,
}

# Note Mode: button -> scale degree (single notes).
_NOTE_BUTTON_DEGREE = {
    bm.UP: 1, bm.RIGHT: 2, bm.DOWN: 3, bm.LEFT: 4,
    bm.A: 5, bm.B: 6, bm.X: 7,
}

_FACE = {bm.A, bm.B, bm.X, bm.Y}


class Actions:
    def __init__(self, state, mixer, looper, metronome, library, arp):
        self.state = state
        self.mixer = mixer
        self.looper = looper
        self.metronome = metronome
        self.library = library
        self.arp = arp
        self._chord_sig = None   # last chord signature
        self._latched = set()    # latched D-pad chord modifiers (chord mode only)
        self._dpad_last = {}     # dir -> last press time (double-tap latch)
        self._fn_used = False    # Function used as a modifier (suppress mode-flip)
        self._arp_latch = False  # debounce for the L2+R2 arp toggle
        self._suppress_l2 = False  # consume the L2 press that toggled arp/panic
        self._suppress_l1 = False  # consume the L1 press that toggled panic
        self._panic_latch = False  # debounce for the L1+L2 panic combo
        self._crush_latch = False  # debounce for the R1+R2 bitcrush combo
        self._suppress_r1 = False  # consume the R1 press that toggled bitcrush
        self._suppress_r2 = False  # consume the R2 press that toggled bitcrush
        self._nav_next = 0.0       # next library auto-scroll time (held nav)
        self._help_latch = False   # debounce for the cheat-sheet toggle
        self._select_used = False  # SELECT held as sustain (suppresses key-cycle)
        self._sustained = set()    # note-mode voice IDs kept alive by sustain pedal

    _HELP_COMBO = {bm.START, bm.SELECT, bm.L1, bm.R1}

    def handle(self, frame: ch.InputFrame):
        # Cheat sheet: Start+Select+L1+R1 toggles it (universal). While the combo
        # is held, swallow all other input so the constituent buttons don't act.
        if self._HELP_COMBO <= frame.held:
            if not self._help_latch:
                self.state.help = not self.state.help
                self.state.help_scroll = 0
                self._help_latch = True
            return
        self._help_latch = False

        # While the cheat sheet is open it's modal: D-pad scrolls, B closes.
        if self.state.help:
            from ui import help_overlay
            if bm.B in frame.pressed or bm.A in frame.pressed:
                self.state.help = False
                return
            if bm.UP in frame.pressed:
                self.state.help_scroll = max(0, self.state.help_scroll - 1)
            if bm.DOWN in frame.pressed:
                self.state.help_scroll = min(help_overlay.max_scroll(),
                                             self.state.help_scroll + 1)
            return

        if self.state.screen == "library":
            self._handle_library(frame)
        else:
            self._handle_play(frame)

    # ---- shared global buttons ----
    def _handle_play(self, frame: ch.InputFrame):
        s = self.state

        if bm.START in frame.pressed:
            s.screen = "library"
            return
        self._handle_function(frame)
        self._handle_panic(frame)
        self._handle_sustain(frame)
        self._handle_arp_toggle(frame)
        self._handle_crush(frame)
        self._handle_effects(frame)
        self._handle_loop(frame)

        fn = bm.FUNCTION in frame.held
        if s.mode == "chord":
            self._play_chord(frame, fn)
        else:
            self._play_note(frame, fn)

    # Function: tap = switch chord/note mode; Function + D-Up/Down = octave.
    def _handle_function(self, frame: ch.InputFrame):
        s = self.state
        if bm.FUNCTION in frame.pressed:
            self._fn_used = False
        if bm.FUNCTION in frame.held:
            if bm.UP in frame.pressed:
                s.octave = min(config.OCTAVE_MAX, s.octave + 1)
                self._fn_used = True
            if bm.DOWN in frame.pressed:
                s.octave = max(config.OCTAVE_MIN, s.octave - 1)
                self._fn_used = True
        if bm.FUNCTION in frame.released and not self._fn_used:
            s.mode = "note" if s.mode == "chord" else "chord"
            if s.mode == "note":            # latched modifiers are chord-only
                self._latched.clear()

    # Panic on L1 + L2: silence stuck voices (suppresses their own actions).
    def _handle_panic(self, frame: ch.InputFrame):
        both_held = {bm.L1, bm.L2} <= frame.held
        triggered = ((bm.L1 in frame.pressed and bm.L2 in frame.held) or
                     (bm.L2 in frame.pressed and bm.L1 in frame.held))
        if triggered and not self._panic_latch:
            self.mixer.panic()
            self.looper.cancel()            # stop everything, incl. any loop
            self.arp.clear()
            self.state.now_playing = ""
            self.state.sustain = False
            self._sustained.clear()
            self._panic_latch = True
            # Leave _chord_sig as-is so a still-held chord stays silent until
            # re-pressed (otherwise panic would instantly re-trigger it).
            if bm.L1 in frame.pressed:
                self._suppress_l1 = True
            if bm.L2 in frame.pressed:
                self._suppress_l2 = True
        if not both_held:
            self._panic_latch = False

    # Sustain pedal: hold SELECT to sustain notes/chords.
    # Key cycling: SELECT + D-pad LEFT (down) / RIGHT (up) around the circle of fifths.
    def _handle_sustain(self, frame: ch.InputFrame):
        s = self.state
        if bm.SELECT in frame.pressed:
            self._select_used = False
        if bm.SELECT in frame.held:
            if bm.RIGHT in frame.pressed:
                s.key = keys.next_key(s.key, step=1)
                self._select_used = True
            if bm.LEFT in frame.pressed:
                s.key = keys.next_key(s.key, step=-1)
                self._select_used = True
        if bm.SELECT in frame.released:
            s.sustain = False
            for vid in self._sustained:
                self.mixer.stop_voice(vid)
                self.looper.on_release(vid)
            self._sustained.clear()
            if s.mode == "chord" and not (frame.held & _FACE):
                self.mixer.stop_voice("chord")
                self.looper.on_release("chord")
                self._chord_sig = None
        else:
            s.sustain = bm.SELECT in frame.held

    # Arpeggiator toggle on L2 + R2 (suppresses their individual actions).
    def _handle_arp_toggle(self, frame: ch.InputFrame):
        both_held = {bm.L2, bm.R2} <= frame.held
        triggered = ((bm.R2 in frame.pressed and bm.L2 in frame.held) or
                     (bm.L2 in frame.pressed and bm.R2 in frame.held))
        if triggered and not self._arp_latch:
            # Cycle off -> up -> down -> bounce -> random -> off.
            i = ARP_MODES.index(self.state.arp) if self.state.arp in ARP_MODES else 0
            self.state.arp = ARP_MODES[(i + 1) % len(ARP_MODES)]
            self._arp_latch = True
            # Only swallow the loop action if L2 was the press that triggered it.
            if bm.L2 in frame.pressed:
                self._suppress_l2 = True
            if self.state.arp == "off":
                self.arp.clear()
        if not both_held:
            self._arp_latch = False

    # Bitcrush (lo-fi output): R1+R2 cycles bit depth, L1+R2 cycles downsampling.
    @staticmethod
    def _cycle(value, steps):
        i = steps.index(value) if value in steps else 0
        return steps[(i + 1) % len(steps)]

    def _handle_crush(self, frame: ch.InputFrame):
        fx = self.state.effects
        held, pressed = frame.held, frame.pressed
        bits_combo = ((bm.R1 in pressed and bm.R2 in held) or
                      (bm.R2 in pressed and bm.R1 in held))
        down_combo = ((bm.L1 in pressed and bm.R2 in held) or
                      (bm.R2 in pressed and bm.L1 in held))
        if not self._crush_latch:
            if bits_combo:
                fx.crush_bits = self._cycle(fx.crush_bits, config.CRUSH_BITS_STEPS)
                self._crush_latch = True
                self._suppress_r1 = bm.R1 in pressed
                self._suppress_r2 = self._suppress_r2 or (bm.R2 in pressed)
            elif down_combo:
                fx.crush_down = self._cycle(fx.crush_down, config.CRUSH_DOWN_STEPS)
                self._crush_latch = True
                self._suppress_l1 = bm.L1 in pressed
                self._suppress_r2 = self._suppress_r2 or (bm.R2 in pressed)
        if not ({bm.R1, bm.R2} <= held or {bm.L1, bm.R2} <= held):
            self._crush_latch = False

    def _handle_effects(self, frame: ch.InputFrame):
        fx = self.state.effects
        if bm.R1 in frame.pressed:
            if self._suppress_r1:           # this R1 press toggled bitcrush
                self._suppress_r1 = False
            elif bm.L1 in frame.held:
                fx.chorus = not fx.chorus
            else:
                fx.reverb = not fx.reverb
        if bm.L1 in frame.pressed:
            if self._suppress_l1:           # this L1 press triggered panic
                self._suppress_l1 = False
            elif bm.R1 in frame.held:
                fx.chorus = not fx.chorus
            else:
                fx.delay = not fx.delay
        # R2 steps wet/dry, unless it's part of the L2+R2 arp or R1+R2 crush combo.
        if bm.R2 in frame.pressed and bm.L2 not in frame.held:
            if self._suppress_r2:
                self._suppress_r2 = False
            else:
                steps = config.WETDRY_STEPS
                i = steps.index(fx.wetdry) if fx.wetdry in steps else 0
                fx.wetdry = steps[(i + 1) % len(steps)]

    def _handle_loop(self, frame: ch.InputFrame):
        # Hold-to-cancel takes priority.
        if frame.l2_hold:
            self.looper.cancel()
            return
        if bm.L2 not in frame.pressed:
            return
        if self._suppress_l2:           # this L2 press toggled the arp (L2+R2)
            self._suppress_l2 = False
            return
        # State machine: IDLE->record, REC->loop, LOOPING->overdub, OVERDUB->merge.
        was_idle = self.looper.state == lr.IDLE
        self.looper.toggle_record(self.state.bpm)
        if was_idle and self.looper.state == lr.RECORDING:
            # Re-fire an already-held chord so it gets captured into the loop
            # (its original note-on happened before recording started).
            self._chord_sig = None

    # D-pad double-tap latches a chord modifier (chord mode only, and not while
    # Function is using D-Up/Down for octave, or SELECT is using L/R for key cycling).
    def _update_latches(self, frame: ch.InputFrame, fn: bool):
        now = time.monotonic()
        consumed = {bm.LEFT, bm.RIGHT} if bm.SELECT in frame.held else set()
        for d in (frame.pressed & (bm.DPAD) - consumed):
            if fn and d in (bm.UP, bm.DOWN):
                continue
            last = self._dpad_last.get(d, 0.0)
            if (now - last) * 1000 <= config.DOUBLE_TAP_MS:
                self._latched.discard(d) if d in self._latched else self._latched.add(d)
            self._dpad_last[d] = now

    # ---- chord mode (sustained drone, or arpeggiated when arp is on) ----
    def _play_chord(self, frame: ch.InputFrame, fn: bool):
        s = self.state
        self._update_latches(frame, fn)
        face = frozenset(frame.held & _FACE)
        dpad = (frame.dpad | self._latched)
        if fn:                                # Function owns D-Up/Down for octave
            dpad = dpad - {bm.UP, bm.DOWN}

        minor = s.minor
        if bm.UP in dpad and not (dpad - {bm.UP}):
            minor = not minor                 # D-Up flips tonality (transient)
        mods = self._resolve_mods(dpad)

        # Re-trigger only when the chord actually changes (face, mods, octave).
        sig = (face, minor, tuple(sorted(mods)), s.octave, s.arp)
        if sig == self._chord_sig:
            return
        self._chord_sig = sig

        degree = _CHORD_COMBOS.get(face)
        # With sustain held and no new chord pressed, keep the voice alive.
        sustain_holding = s.sustain and degree is None
        if not sustain_holding:
            self.mixer.stop_voice("chord")
            self.looper.on_release("chord")
        if degree is None:
            if s.sustain:
                self._select_used = True
            self.arp.clear()
            s.now_playing = ""
            return
        midis = chords.build_chord(s.key, minor, degree, mods,
                                   octave=4 + s.octave)
        if s.arp != "off":
            # The arpeggiator records its own per-note sequence to the looper.
            self.arp.set_notes(midis, s.effects, s.bpm, s.arp)
        else:
            self.mixer.play_voice("chord", midis, s.effects, s.bpm)
            self.looper.on_trigger(midis, s.effects, s.bpm,
                                   loop=self.mixer._loop_mode, key="chord")
        s.now_playing = chords.chord_name(s.key, minor, degree, mods)

    def _resolve_mods(self, dpad):
        mods = []
        key = frozenset(dpad)
        if key in _DPAD_MOD:
            mods.append(_DPAD_MOD[key])
        return mods

    # ---- note mode (sustained per button: hold = drone, release = stop) ----
    def _play_note(self, frame: ch.InputFrame, fn: bool):
        s = self.state
        pressed = frame.pressed
        if fn:                                # Function owns D-Up/Down for octave
            pressed = pressed - {bm.UP, bm.DOWN}
        if bm.SELECT in frame.held:           # SELECT owns L/R for key cycling
            pressed = pressed - {bm.LEFT, bm.RIGHT}

        # Octave shift combos (A+B down, A+X up) take priority on press.
        if pressed & {bm.A, bm.B, bm.X}:
            if {bm.A, bm.B} <= frame.held:
                s.octave = max(config.OCTAVE_MIN, s.octave - 1)
                return
            if {bm.A, bm.X} <= frame.held:
                s.octave = min(config.OCTAVE_MAX, s.octave + 1)
                return

        for btn in pressed:
            midi = None
            if btn == bm.Y:
                midi = notes.note_midi(s.key, s.minor, 1,
                                       octave_shift=s.octave, extra_octave=True)
            elif btn in _NOTE_BUTTON_DEGREE:
                midi = notes.note_midi(s.key, s.minor, _NOTE_BUTTON_DEGREE[btn],
                                       octave_shift=s.octave)
            if midi is not None:
                vid = ("note", btn)
                self._sustained.discard(vid)   # re-press lifts pedal hold on this voice
                self.mixer.play_voice(vid, [midi], s.effects, s.bpm)
                self.looper.on_trigger([midi], s.effects, s.bpm,
                                       loop=self.mixer._loop_mode, key=vid)
                s.now_playing = notes.note_name(midi)

        for btn in frame.released:
            vid = ("note", btn)
            if s.sustain:
                self._sustained.add(vid)
                self._select_used = True
            else:
                self.mixer.stop_voice(vid)
                self.looper.on_release(vid)

        if not (frame.held & _FACE) and not frame.dpad and not self._sustained:
            s.now_playing = ""

    # ---- library screen ----
    def _handle_library(self, frame: ch.InputFrame):
        s = self.state
        if bm.START in frame.pressed:
            s.screen = "play"
            return
        if bm.SELECT in frame.pressed:
            s.metronome_on = not s.metronome_on
            if s.metronome_on:
                self.metronome.reset()

        # Up/Down with hold-to-repeat, driven off the currently-held D-pad so no
        # stale direction survives leaving/re-entering the screen.
        now = time.monotonic()
        if bm.UP in frame.pressed:
            self.library.move(-1); self._nav_next = now + 0.35
        if bm.DOWN in frame.pressed:
            self.library.move(1); self._nav_next = now + 0.35
        held_dir = -1 if bm.UP in frame.dpad else (1 if bm.DOWN in frame.dpad else 0)
        if held_dir and now >= self._nav_next:
            self.library.move(held_dir)
            self._nav_next = now + 0.06

        # L1/R1 page jump; L2/R2 jump to prev/next folder.
        if bm.L1 in frame.pressed:
            self.library.move(-self.library.page())
        if bm.R1 in frame.pressed:
            self.library.move(self.library.page())
        if bm.L2 in frame.pressed:
            self.library.jump_folder(-1)
        if bm.R2 in frame.pressed:
            self.library.jump_folder(1)

        # Left/Right set BPM (and re-time a running loop to the new tempo).
        if bm.LEFT in frame.pressed:
            s.bpm = max(config.BPM_MIN, s.bpm - config.BPM_STEP)
            self.looper.retime(s.bpm)
        if bm.RIGHT in frame.pressed:
            s.bpm = min(config.BPM_MAX, s.bpm + config.BPM_STEP)
            self.looper.retime(s.bpm)

        if bm.A in frame.pressed:
            self.library.select(self.state, self.mixer)
        if bm.Y in frame.pressed:
            self.library.toggle_favorite(self.state)
        if bm.B in frame.pressed:
            self.library.toggle_fav_filter(self.state)
