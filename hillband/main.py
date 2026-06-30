#!/usr/bin/env python3
"""HillBand — entry point.

A 16-step × 8-track hybrid sequencer:
  * Tracks 0–3 (top / melodic): each plays a pitched HillChord instrument,
    with configurable chord type and optional scale-step cycling.
  * Tracks 4–7 (bottom / drums): fixed roles KICK/SNARE/HAT/PERC; library
    pre-filtered to percussion; no pitch-shift, no effects.

Shares HillSequencer's transport, chain, swing, tap-tempo, loop-player,
sequence-save, and the same button indices for the RG35XXSP.
"""

from __future__ import annotations

import os
import platform
import sys
import time

import pygame

import config
from state import AppState
from transport import Transport
from sequencer import Sequencer
from instruments import TrackRack
from loop_player import LoopPlayer
from audio.metronome import Metronome
from library_overlay import Library
from chain_editor import ChainEditor
from sequence_manager import SequenceManager
from help_overlay import HelpOverlay
from track_mode_overlay import TrackModeOverlay
from ui import UI
from input import button_map as bm
from config import (
    NUM_TRACKS, NUM_STEPS, NUM_PATTERNS,
    VELOCITY_MIN, VELOCITY_MAX, VELOCITY_STEP,
    TRACK_VOLUME_STEP, SWING_MIN, SWING_MAX, SWING_STEP,
    BPM_HOLD_DELAY, BPM_HOLD_STEP, A_HOLD_THRESHOLD, EXIT_HOLD_SECONDS,
    WETDRY_STEPS, CRUSH_BITS_STEPS, CRUSH_DOWN_STEPS,
)

_DIR_HAT = {bm.UP: (0, 1), bm.DOWN: (0, -1), bm.LEFT: (-1, 0), bm.RIGHT: (1, 0)}


def _init():
    pygame.mixer.pre_init(config.MIXER_FREQ, config.MIXER_SIZE,
                          config.MIXER_CHANNELS_OUT, config.MIXER_BUFFER)
    pygame.init()
    pygame.mixer.init(config.MIXER_FREQ, config.MIXER_SIZE,
                      config.MIXER_CHANNELS_OUT, config.MIXER_BUFFER)
    pygame.mixer.set_num_channels(config.TOTAL_CHANNELS)
    pygame.joystick.init()
    pygame.display.set_caption("HillBand")
    if platform.system() == "Darwin":
        screen = pygame.display.set_mode((config.SCREEN_W, config.SCREEN_H))
    else:
        screen = pygame.display.set_mode((config.SCREEN_W, config.SCREEN_H),
                                         pygame.FULLSCREEN | pygame.NOFRAME)
    pygame.mouse.set_visible(False)
    return screen


def main():
    screen = _init()
    clock  = pygame.time.Clock()

    _splash = os.path.join(os.path.dirname(__file__), "splash.png")
    if os.path.isfile(_splash):
        screen.blit(pygame.image.load(_splash), (0, 0))
        pygame.display.flip()
        pygame.time.wait(1500)

    state = AppState()
    state.load()

    transport = Transport(state)
    transport.current_pattern = state.current_pattern
    rack      = TrackRack(state)
    rack.reload_all()
    sequencer   = Sequencer(state, transport, rack)
    loop_player = LoopPlayer(state, transport)
    if state.loop_file:
        loop_player.load(state.loop_file, state.loop_native_bpm)
        if state.loop_enabled:
            loop_player.enable(True)
    metronome = Metronome(transport)

    ui       = UI(state, transport, loop_player)
    library  = Library(config.SAMPLE_PATH, ui.font_n, ui.font_s,
                       drum_root=config.DRUM_SAMPLE_PATH)
    chain_ed = ChainEditor(state, ui.font_n, ui.font_s, reset_chain_cb=transport.reset_chain)
    seq_mgr  = SequenceManager(state, ui.font_n, ui.font_s)
    help_ov  = HelpOverlay(ui.font_n, ui.font_s)
    tmode_ov = TrackModeOverlay(state, ui.font_n, ui.font_s)

    joystick = None
    if pygame.joystick.get_count() > 0:
        joystick = pygame.joystick.Joystick(0)
        joystick.init()

    # ── Overlay flags ─────────────────────────────────────────────────────────
    show_library = show_chain = show_seq = show_help = show_tmode = False

    def any_overlay():
        return show_library or show_chain or show_seq or show_help or show_tmode

    # ── Input state ───────────────────────────────────────────────────────────
    start_held = start_used = False
    select_held = select_used = False
    fn_held = False
    ss_active = ss_fired = False
    ss_time = None

    a_press_time, a_held = None, False
    y_press_time, y_held = None, False
    x_press_time, x_held = None, False

    bpm_hold_dir  = 0
    bpm_hold_start = bpm_hold_last = None

    last_swing = state.swing if state.swing > 0 else 50

    # ── Helpers ───────────────────────────────────────────────────────────────
    def apply_library_pending():
        if not library.pending:
            return
        kind, payload, _name, sid = library.pending
        library.pending = None
        for t in library.target_tracks:
            try:
                rack.assign(t, kind, payload, sid)
            except Exception as e:
                print(f"[main] assign to track {t} failed: {e}")
        ui.selected_tracks.clear()

    def apply_loaded_sequence():
        sequencer.stop()
        rack.reload_all()
        transport.current_step    = 0
        transport.pending_pattern = None
        if state.chain_enabled and state.chain:
            transport.reset_chain()
        else:
            transport.current_pattern = max(0, min(state.current_pattern, NUM_PATTERNS - 1))
        if state.loop_file:
            loop_player.load(state.loop_file, state.loop_native_bpm)
            loop_player.enable(state.loop_enabled)
        else:
            loop_player.load(None)
        ui.cursor_track = 0
        ui.cursor_step  = 0

    def change_pattern(direction):
        state.chain_enabled = False
        new = (transport.current_pattern + direction) % NUM_PATTERNS
        if transport.playing:
            transport.queue_pattern(new)
        else:
            transport.current_pattern = new
            state.current_pattern     = new
            transport.pending_pattern = None

    def begin_ss():
        nonlocal ss_active, ss_time, ss_fired, start_used, select_used
        ss_active = True
        ss_time   = time.monotonic()
        ss_fired  = False
        start_used = select_used = True

    # ── Hat handler ───────────────────────────────────────────────────────────
    def handle_hat(hat):
        dirs = bm.hat_to_dirs(hat)
        if show_library:  library.handle_hat(hat, state); return
        if show_chain:    chain_ed.handle_hat(hat); return
        if show_seq:      seq_mgr.handle_hat(hat); return
        if show_help:     help_ov.handle_hat(hat); return
        if show_tmode:    tmode_ov.handle_hat(hat); return

        cur = ui.cursor_track
        if a_held:
            cell = state.patterns[transport.current_pattern][cur][ui.cursor_step]
            if bm.UP in dirs:
                state.set_step_velocity(transport.current_pattern, cur, ui.cursor_step,
                                        min(VELOCITY_MAX, cell[1] + VELOCITY_STEP))
            elif bm.DOWN in dirs:
                state.set_step_velocity(transport.current_pattern, cur, ui.cursor_step,
                                        max(VELOCITY_MIN, cell[1] - VELOCITY_STEP))
            return
        if y_held:
            vol = state.tracks[cur].get("volume", 1.0)
            if bm.UP in dirs:
                state.set_track_volume(cur, vol + TRACK_VOLUME_STEP)
            elif bm.DOWN in dirs:
                state.set_track_volume(cur, vol - TRACK_VOLUME_STEP)
            return
        if x_held:
            if not state.is_drum_track(cur):
                if bm.LEFT in dirs:
                    state.transpose_track(cur, -1);  rack.prewarm_track(cur)
                elif bm.RIGHT in dirs:
                    state.transpose_track(cur, +1);  rack.prewarm_track(cur)
                elif bm.UP in dirs:
                    state.transpose_track(cur, +12); rack.prewarm_track(cur)
                elif bm.DOWN in dirs:
                    state.transpose_track(cur, -12); rack.prewarm_track(cur)
            return
        # Cursor movement
        if bm.LEFT in dirs:
            ui.cursor_step = (ui.cursor_step - 1) % NUM_STEPS
        elif bm.RIGHT in dirs:
            ui.cursor_step = (ui.cursor_step + 1) % NUM_STEPS
        elif bm.UP in dirs:
            ui.cursor_track = max(0, ui.cursor_track - 1)
        elif bm.DOWN in dirs:
            ui.cursor_track = min(NUM_TRACKS - 1, ui.cursor_track + 1)

    # ── Effects (Fn modifier, melodic tracks only) ────────────────────────────
    def fx_action(b):
        if state.is_drum_track(ui.cursor_track):
            return
        fx = state.tracks[ui.cursor_track]["effects"]
        if b == bm.A:
            fx.reverb = not fx.reverb
        elif b == bm.B:
            fx.delay = not fx.delay
        elif b == bm.X:
            fx.chorus = not fx.chorus
        elif b == bm.Y:
            i = WETDRY_STEPS.index(fx.wetdry) if fx.wetdry in WETDRY_STEPS else 0
            fx.wetdry = WETDRY_STEPS[(i + 1) % len(WETDRY_STEPS)]
        elif b == bm.R1:
            i = CRUSH_BITS_STEPS.index(fx.crush_bits) if fx.crush_bits in CRUSH_BITS_STEPS else 0
            fx.crush_bits = CRUSH_BITS_STEPS[(i + 1) % len(CRUSH_BITS_STEPS)]
        elif b == bm.L1:
            i = CRUSH_DOWN_STEPS.index(fx.crush_down) if fx.crush_down in CRUSH_DOWN_STEPS else 0
            fx.crush_down = CRUSH_DOWN_STEPS[(i + 1) % len(CRUSH_DOWN_STEPS)]
        else:
            return
        rack.prewarm_track(ui.cursor_track)

    # ── Button-down ───────────────────────────────────────────────────────────
    def handle_button_down(b):
        nonlocal start_held, start_used, select_held, select_used, fn_held
        nonlocal a_press_time, a_held, y_press_time, y_held, x_press_time, x_held
        nonlocal bpm_hold_dir, bpm_hold_start, bpm_hold_last
        nonlocal show_library, show_chain, show_seq, show_help, show_tmode, last_swing

        # Modifiers
        if b == bm.START:
            start_held = True; start_used = False
            if select_held:
                begin_ss()
            return
        if b == bm.SELECT:
            select_held = True; select_used = False
            if start_held:
                begin_ss()
            if fn_held:
                select_used = True
                help_ov.open(); show_help = True
            return
        if b == bm.FUNCTION:
            fn_held = True
            if select_held:
                select_used = True
                help_ov.open(); show_help = True
            return

        base = not (start_held or select_held or fn_held) and not any_overlay()
        if base:
            now = time.monotonic()
            if b == bm.A:
                a_press_time, a_held = now, False
            elif b == bm.Y:
                y_press_time, y_held = now, False
            elif b == bm.X:
                x_press_time, x_held = now, False

        # Overlay routing
        if show_library:
            library.handle_button(b, state)
            if library.closed:
                show_library = False
                apply_library_pending()
            return
        if show_chain:
            chain_ed.handle_button(b)
            if chain_ed.closed:
                show_chain = False
            return
        if show_seq:
            seq_mgr.handle_button(b)
            if seq_mgr.closed:
                show_seq = False
                if seq_mgr.loaded:
                    apply_loaded_sequence(); seq_mgr.loaded = False
            return
        if show_help:
            help_ov.handle_button(b)
            if help_ov.closed:
                show_help = False
            return
        if show_tmode:
            tmode_ov.handle_button(b)
            if tmode_ov.closed:
                show_tmode = False
                if tmode_ov.applied:
                    rack.prewarm_track(ui.cursor_track)
            return

        # Fn layer (melodic tracks only)
        if fn_held:
            fx_action(b)
            return

        # Start combos
        if start_held:
            start_used = True
            if b == bm.A:
                a_press_time = None
                loop_player.toggle()
            elif b == bm.B:
                targets  = sorted(ui.selected_tracks) or [ui.cursor_track]
                all_drum = all(state.is_drum_track(t) for t in targets)
                library.open(targets, state, drum_only=all_drum)
                show_library = True
            elif b == bm.X:
                chain_ed.open(); show_chain = True
            elif b == bm.Y:
                state.save(); print("[HillBand] state saved")
            elif b == bm.R1:
                state.copy_pattern(transport.current_pattern)
            elif b == bm.L1:
                state.paste_pattern(transport.current_pattern)
            elif b == bm.L2:
                # Chord/scale overlay — only for melodic tracks
                if not state.is_drum_track(ui.cursor_track):
                    tmode_ov.open(ui.cursor_track); show_tmode = True
            elif b == bm.R2:
                cur = ui.cursor_track
                if cur in ui.selected_tracks:
                    ui.selected_tracks.discard(cur)
                else:
                    ui.selected_tracks.add(cur)
            return

        # Select combos
        if select_held:
            select_used = True
            if b == bm.Y:
                seq_mgr.open(); show_seq = True
            elif b == bm.A:
                state.metronome_on = not state.metronome_on
                metronome.reset()
            elif b == bm.X:
                if state.swing > 0:
                    last_swing = state.swing
                    transport.swing = 0
                else:
                    transport.swing = last_swing if last_swing > 0 else 50
            elif b == bm.R2:
                transport.swing = min(SWING_MAX, state.swing + SWING_STEP)
            elif b == bm.L2:
                transport.swing = max(SWING_MIN, state.swing - SWING_STEP)
            elif b == bm.L1:
                state.clear_pattern(transport.current_pattern)
            elif b == bm.R1:
                state.clear_instruments()
                rack.unload_all()
            return

        # Base layer
        if b == bm.R1:
            change_pattern(+1)
        elif b == bm.L1:
            change_pattern(-1)
        elif b == bm.R2:
            bpm_hold_dir = 1
            bpm_hold_start = bpm_hold_last = time.monotonic()
            transport.bpm = state.bpm + 1
            loop_player.reload_if_bpm_changed()
        elif b == bm.L2:
            bpm_hold_dir = -1
            bpm_hold_start = bpm_hold_last = time.monotonic()
            transport.bpm = state.bpm - 1
            loop_player.reload_if_bpm_changed()

    # ── Button-up ─────────────────────────────────────────────────────────────
    def handle_button_up(b):
        nonlocal start_held, start_used, select_held, select_used, fn_held
        nonlocal a_press_time, a_held, y_press_time, y_held, x_press_time, x_held
        nonlocal bpm_hold_dir, bpm_hold_start, bpm_hold_last
        nonlocal ss_active, ss_fired

        if b == bm.START:
            if ss_active and not ss_fired:
                sequencer.stop_and_reset(); transport.stop_and_reset(); ss_active = False
            elif not start_used:
                sequencer.toggle()
            start_held = False; start_used = False
            return
        if b == bm.SELECT:
            if ss_active and not ss_fired:
                sequencer.stop_and_reset(); transport.stop_and_reset(); ss_active = False
            elif not select_used:
                bpm = transport.tap()
                ui.notify_tap(bpm)
                if bpm:
                    loop_player.reload_if_bpm_changed()
            select_held = False; select_used = False
            return
        if b == bm.FUNCTION:
            fn_held = False
            return
        if b == bm.A:
            if a_press_time is not None and not a_held and \
               time.monotonic() - a_press_time < A_HOLD_THRESHOLD:
                state.toggle_step(transport.current_pattern, ui.cursor_track, ui.cursor_step)
            a_press_time, a_held = None, False
        elif b == bm.Y:
            if y_press_time is not None and not y_held and \
               time.monotonic() - y_press_time < A_HOLD_THRESHOLD:
                t = state.tracks[ui.cursor_track]
                t["muted"] = not t["muted"]
            y_press_time, y_held = None, False
        elif b == bm.X:
            x_press_time, x_held = None, False
        elif b in (bm.R2, bm.L2):
            bpm_hold_dir = 0
            bpm_hold_start = bpm_hold_last = None

    # ── Keyboard escape ───────────────────────────────────────────────────────
    def close_top_overlay():
        nonlocal show_library, show_chain, show_seq, show_help, show_tmode
        if show_library:  show_library = False
        elif show_chain:  show_chain   = False
        elif show_seq:    show_seq     = False
        elif show_help:   show_help    = False
        elif show_tmode:  show_tmode   = False
        else:             return False
        return True

    # ── Main loop ─────────────────────────────────────────────────────────────
    running = True
    while running:
        now = time.monotonic()

        if a_press_time is not None and not a_held and now - a_press_time >= A_HOLD_THRESHOLD:
            a_held = True
        if y_press_time is not None and not y_held and now - y_press_time >= A_HOLD_THRESHOLD:
            y_held = True
        if x_press_time is not None and not x_held and now - x_press_time >= A_HOLD_THRESHOLD:
            x_held = True

        if bpm_hold_dir != 0 and bpm_hold_start is not None:
            if now - bpm_hold_start >= BPM_HOLD_DELAY:
                if bpm_hold_last is None or now - bpm_hold_last >= 0.2:
                    transport.bpm = state.bpm + bpm_hold_dir * BPM_HOLD_STEP
                    loop_player.reload_if_bpm_changed()
                    bpm_hold_last = now

        if ss_active and not ss_fired and ss_time is not None:
            if now - ss_time >= EXIT_HOLD_SECONDS:
                ss_fired = True
                pygame.event.post(pygame.event.Event(pygame.QUIT))

        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                running = False
            elif event.type == pygame.JOYHATMOTION:
                if event.value != (0, 0):
                    handle_hat(event.value)
            elif event.type == pygame.JOYBUTTONDOWN:
                logical = bm.JOY_BUTTONS.get(event.button)
                if logical:
                    handle_button_down(logical)
            elif event.type == pygame.JOYBUTTONUP:
                logical = bm.JOY_BUTTONS.get(event.button)
                if logical:
                    handle_button_up(logical)
            elif event.type == pygame.KEYDOWN:
                if event.key == pygame.K_ESCAPE:
                    if not close_top_overlay():
                        running = False
                elif show_library:
                    library.handle_key(event.key, state)
                    if library.closed:
                        show_library = False; apply_library_pending()
                elif show_chain:
                    chain_ed.handle_key(event.key)
                    if chain_ed.closed:
                        show_chain = False
                elif show_seq:
                    seq_mgr.handle_key(event.key)
                    if seq_mgr.closed:
                        show_seq = False
                        if seq_mgr.loaded:
                            apply_loaded_sequence(); seq_mgr.loaded = False
                elif show_help:
                    help_ov.handle_key(event.key)
                    if help_ov.closed:
                        show_help = False
                elif show_tmode:
                    tmode_ov.handle_key(event.key)
                    if tmode_ov.closed:
                        show_tmode = False
                        if tmode_ov.applied:
                            rack.prewarm_track(ui.cursor_track)
                else:
                    logical = bm.KEYBOARD.get(event.key)
                    if logical in _DIR_HAT:
                        handle_hat(_DIR_HAT[logical])
                    elif logical:
                        handle_button_down(logical)
            elif event.type == pygame.KEYUP:
                if not any_overlay():
                    logical = bm.KEYBOARD.get(event.key)
                    if logical and logical not in _DIR_HAT:
                        handle_button_up(logical)

        rack.update()
        metronome.update(state)

        ui.draw(screen)
        if show_library:
            library.draw(screen, state)
        elif show_chain:
            chain_ed.draw(screen)
        elif show_seq:
            seq_mgr.draw(screen)
        elif show_help:
            help_ov.draw(screen)
        elif show_tmode:
            tmode_ov.draw(screen)

        pygame.display.flip()
        clock.tick(config.FPS)

    sequencer.stop()
    state.save()
    pygame.quit()
    sys.exit(0)


if __name__ == "__main__":
    main()
