#!/usr/bin/env python3
"""HillChord — handheld chord/note instrument. Entry point.

Pipeline each frame:
    pygame events -> ComboHandler -> Actions -> (mixer / looper / metronome)
                                             -> AppState -> UI render
"""

import os
import sys
import threading
import time

import pygame

import config
import persistence
from state import AppState
from audio.mixer import Mixer
from audio.loop_recorder import LoopRecorder
from audio.metronome import Metronome
from audio.arpeggiator import Arpeggiator
from audio.transport import Transport
from input.combo_handler import ComboHandler
from input.actions import Actions
from ui.library_screen import Library
from ui import play_screen
from ui import help_overlay
import precache


def _log(msg):
    print(f"[init] {msg}", flush=True)


def _init():
    _log("pre_init mixer")
    pygame.mixer.pre_init(config.MIXER_FREQ, config.MIXER_SIZE,
                          config.MIXER_CHANNELS_OUT, config.MIXER_BUFFER)
    _log("pygame.init")
    pygame.init()
    _log("mixer.init")
    pygame.mixer.init(config.MIXER_FREQ, config.MIXER_SIZE,
                      config.MIXER_CHANNELS_OUT, config.MIXER_BUFFER)
    # 12 voices + reserved metronome channel + reserved loop channel.
    pygame.mixer.set_num_channels(config.NUM_VOICES + 2)
    _log("joystick.init")
    pygame.joystick.init()

    _log(f"display.set_mode driver={pygame.display.get_driver() if pygame.display.get_init() else '?'}")
    screen = pygame.display.set_mode((config.SCREEN_W, config.SCREEN_H))
    _log(f"display ready, driver={pygame.display.get_driver()}")
    pygame.display.set_caption("HillChord")
    pygame.mouse.set_visible(False)
    return screen


def _make_fonts():
    pygame.font.init()
    f = pygame.font.Font  # default font
    return {
        "huge":  f(None, config.FONT_SIZE_HUGE),
        "big":   f(None, config.FONT_SIZE_LARGE),
        "mid":   f(None, config.FONT_SIZE_NORMAL),
        "small": f(None, config.FONT_SIZE_SMALL),
    }


def main():
    screen = _init()
    fonts = _make_fonts()
    clock = pygame.time.Clock()

    _splash = os.path.join(os.path.dirname(__file__), "splash.png")
    if os.path.isfile(_splash):
        screen.blit(pygame.image.load(_splash), (0, 0))
        pygame.display.flip()
        pygame.time.wait(1500)

    state = AppState()
    persistence.load_state(state)
    # Always boot in C major (other settings — sound, BPM, effects, etc. — are
    # still restored from the saved state).
    state.key = "C"
    state.minor = False

    _log("building subsystems")
    transport = Transport()
    mixer = Mixer()
    library = Library(config.SAMPLE_PATH)
    looper = LoopRecorder(mixer, transport)
    metronome = Metronome(transport)
    arp = Arpeggiator(mixer, transport, looper)
    combo = ComboHandler()
    actions = Actions(state, mixer, looper, metronome, library, arp)

    # Restore previously selected sound if it still exists.
    if state.sound:
        library.load_sid(state.sound, state, mixer)

    # Pre-render all library sounds (dry) to disk in the background so every
    # sound plays instantly on first select.  Cancelled cleanly on exit.
    _precache_stop = threading.Event()
    precache.start(_precache_stop, bpm=state.bpm)

    # Set HILLCHORD_DEBUG_INPUT=1 to log raw joystick indices/hats to log.txt.
    debug_input = os.environ.get("HILLCHORD_DEBUG_INPUT") == "1"

    from input import button_map as bm
    quit_since = None   # timestamp Select+Start were first both held

    _log("entering main loop")
    last_saved = state.to_dict()   # autosave-on-change baseline
    running = True
    while running:
        events = pygame.event.get()
        for e in events:
            if e.type == pygame.QUIT:
                running = False
            elif e.type == pygame.KEYDOWN and e.key == pygame.K_ESCAPE:
                running = False
            elif debug_input and e.type == pygame.KEYDOWN:
                print(f"[input] KEYDOWN key={pygame.key.name(e.key)}", flush=True)
            elif debug_input and e.type == pygame.JOYBUTTONDOWN:
                print(f"[input] JOYBUTTONDOWN index={e.button}", flush=True)
            elif debug_input and e.type == pygame.JOYHATMOTION:
                print(f"[input] JOYHATMOTION value={e.value}", flush=True)

        frame = combo.process(events)

        # Exit combo: hold Select + Start together for ~1.2s.
        if {bm.SELECT, bm.START} <= frame.held:
            now = pygame.time.get_ticks()
            quit_since = quit_since or now
            if now - quit_since >= int(config.EXIT_HOLD_SECONDS * 1000):
                running = False
        else:
            quit_since = None

        actions.handle(frame)
        arp.update()
        mixer.update()      # fire scheduled delay echoes
        looper.update()
        metronome.update(state)

        if state.screen == "library":
            library.draw(screen, state, fonts)
        else:
            play_screen.draw(screen, state, looper, fonts, mixer,
                             sustain=state.sustain)
        if state.help:
            help_overlay.draw(screen, fonts, state.help_scroll)

        # Lavender beat dot, top-center, pulsing on the transport grid (always,
        # whether or not the metronome is audible).
        beat = transport.beat_len(state.bpm)
        phase = ((time.monotonic() - transport.epoch) % beat) / beat
        on = phase < 0.12
        pygame.draw.circle(screen, (200, 170, 255) if on else (74, 66, 100),
                           (config.SCREEN_W // 2, 9), 6)

        pygame.display.flip()

        # Perform any deferred sound load now that the "Loading…" frame is shown.
        library.commit_pending(state, mixer)

        # Autosave when persisted state changes (crash/power-off safety).
        snap = state.to_dict()
        if snap != last_saved:
            persistence.save_state(state)
            last_saved = snap

        clock.tick(config.FPS)

    _precache_stop.set()          # cancel background precache thread
    persistence.save_state(state)
    pygame.quit()


if __name__ == "__main__":
    main()
