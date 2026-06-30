#!/usr/bin/env python3
"""Headless smoke test: drives the pipeline without a display/audio device."""

import os
import sys

os.environ.setdefault("SDL_VIDEODRIVER", "dummy")
os.environ.setdefault("SDL_AUDIODRIVER", "dummy")
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import pygame
import config
from state import AppState
from audio.mixer import Mixer
from audio.loop_recorder import LoopRecorder
from audio.metronome import Metronome
from audio.arpeggiator import Arpeggiator
from input.actions import Actions
from input.combo_handler import InputFrame
from input import button_map as bm
from ui.library_screen import Library
from theory import chords

pygame.mixer.pre_init(config.MIXER_FREQ, config.MIXER_SIZE, 2, config.MIXER_BUFFER)
pygame.init()
pygame.mixer.init(config.MIXER_FREQ, config.MIXER_SIZE, 2, config.MIXER_BUFFER)
pygame.mixer.set_num_channels(config.NUM_VOICES + 2)

from audio.transport import Transport
state = AppState()
mixer = Mixer()
library = Library(config.SAMPLE_PATH)
transport = Transport()
looper = LoopRecorder(mixer, transport)
metro = Metronome(transport)
arp = Arpeggiator(mixer, transport, looper)
actions = Actions(state, mixer, looper, metro, library, arp)

# 1) theory sanity
cmaj = chords.build_chord("C", False, 1, [])
assert cmaj == [60, 64, 67], cmaj
cmin = chords.build_chord("C", True, 1, [])
assert cmin == [60, 63, 67], cmin
g7 = chords.build_chord("C", False, 5, [chords.MOD_DOM7])
assert g7 == [67, 71, 74, 77], g7
print("theory OK:", cmaj, cmin, g7)

# 2) load a sound bank
folder = os.path.join(config.SAMPLE_PATH, "Synth")
mixer.load_sound(folder)
assert mixer.bank and mixer.bank.pcm, "no samples loaded"
print("bank loaded:", len(mixer.bank.pcm), "pitches")

# 3) fire a dry chord (press A)
f = InputFrame(pressed={bm.A}, held={bm.A})
actions.handle(f)

# 4) enable reverb+delay+chorus at 75% and fire again (exercises DSP)
state.effects.reverb = state.effects.delay = state.effects.chorus = True
state.effects.wetdry = 75
f = InputFrame(pressed={bm.X}, held={bm.X}, dpad={bm.UP, bm.RIGHT})  # V dom7
actions.handle(f)
print("wet render cache entries:", len(mixer._cache))

# 5) looper record -> loop -> update (wait past the beat-aligned count-in)
import time as _t
looper.toggle_record()
while _t.monotonic() < looper._t0 + 0.01:
    _t.sleep(0.002)
actions.handle(InputFrame(pressed={bm.B}, held={bm.B}))
looper.toggle_record()
assert looper.state == "looping", looper.state
looper.update()
print("looper OK, events:", len(looper._events))

# 6) metronome tick
state.metronome_on = True
metro.reset()
metro.update(state)
print("metronome OK")

# 7) note mode
state.mode = "note"
actions.handle(InputFrame(pressed={bm.UP}, held={bm.UP}))
print("note mode OK")

pygame.quit()
print("\nALL SMOKE TESTS PASSED")
