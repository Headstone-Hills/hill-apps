"""
HillBeat — main.py
Entry point: initialises pygame, wires all components, runs the event loop.
"""

import os
import sys
import platform
import time
import pygame

from constants import (
    SCREEN_W, SCREEN_H, FPS,
    BTN_A, BTN_B, BTN_X, BTN_Y,
    BTN_L1, BTN_R1, BTN_L2, BTN_R2,
    BTN_SELECT, BTN_START, BTN_FUNCTION,
    HAT_LEFT, HAT_RIGHT, HAT_UP, HAT_DOWN,
    NUM_VOICES, NUM_STEPS,
    VELOCITY_MIN, VELOCITY_MAX, VELOCITY_STEP,
    VOICE_VOLUME_MIN, VOICE_VOLUME_MAX, VOICE_VOLUME_STEP,
    SWING_MIN, SWING_MAX,
    BPM_HOLD_DELAY, BPM_HOLD_STEP,
    A_HOLD_THRESHOLD, Y_HOLD_THRESHOLD,
    EXIT_HOLD_SECONDS,
    SAMPLE_RATE, BUFFER_SIZE, NUM_CHANNELS,
    SAMPLE_ROOT,
)
from state        import AppState
from transport    import Transport
from sequencer    import Sequencer
from loop_player  import LoopPlayer
from library      import SampleLibrary
from chain_editor import ChainEditor
from ui           import UI
import help_overlay


def main():
    pygame.init()
    pygame.mixer.init(frequency=SAMPLE_RATE, size=-16, channels=2, buffer=BUFFER_SIZE)
    pygame.mixer.set_num_channels(NUM_CHANNELS)
    pygame.display.set_caption("HillBeat")

    # Windowed on Mac/desktop, fullscreen on device
    if platform.system() == "Darwin":
        screen = pygame.display.set_mode((SCREEN_W, SCREEN_H))
    else:
        screen = pygame.display.set_mode((SCREEN_W, SCREEN_H), pygame.FULLSCREEN | pygame.NOFRAME)

    clock = pygame.time.Clock()

    _splash = os.path.join(os.path.dirname(__file__), "splash.png")
    if os.path.isfile(_splash):
        screen.blit(pygame.image.load(_splash), (0, 0))
        pygame.display.flip()
        pygame.time.wait(1500)

    # ── Initialise components ─────────────────────────────────────────────────
    state     = AppState()
    state.load()

    transport = Transport(state)
    transport.current_pattern = state.current_pattern

    sequencer = Sequencer(state, transport)
    sequencer.reload_all_voices()

    loop_player = LoopPlayer(state, transport)
    if state.loop_file:
        loop_player.load(state.loop_file, state.loop_native_bpm)
        if state.loop_enabled:
            loop_player.enable(True)

    ui        = UI(state, transport, loop_player)
    library   = SampleLibrary(ui.font_n, ui.font_s)
    library.warmup(state.favorites)       # pre-scan sample dirs
    chain_ed  = ChainEditor(state, ui.font_n, ui.font_s,
                            reset_chain_cb=transport.reset_chain)

    # ── Gamepad ───────────────────────────────────────────────────────────────
    joystick = None
    if pygame.joystick.get_count() > 0:
        joystick = pygame.joystick.Joystick(0)
        joystick.init()

    # ── Overlay state ─────────────────────────────────────────────────────────
    show_library = False
    show_chain   = False
    show_help    = False
    help_scroll  = 0

    # Fonts for help overlay
    help_fonts = {
        "mid":   pygame.font.SysFont("monospace", 22, bold=True),
        "small": pygame.font.SysFont("monospace", 17),
    }

    # ── Input state ───────────────────────────────────────────────────────────
    # START and SELECT use a deferred-action pattern:
    #   press  → set _held flag, record as unused
    #   release → if unused, fire primary action (play/pause or tap tempo)
    #   while held + other btn → fire combo, mark as used
    start_held        = False
    start_used        = False   # True when START was used as a modifier in this press
    select_held       = False
    select_used       = False
    start_select_since = None   # monotonic time when both START+SELECT became held
    function_held     = False   # FUNCTION held only as the cheat-sheet modifier

    # A-button hold for velocity-adjust mode
    a_press_time = None
    a_held       = False

    # Y-button hold for per-voice volume adjust (Y-tap stays mute)
    y_press_time = None
    y_held       = False

    # BPM hold-to-repeat (R2/L2)
    bpm_hold_dir   = 0        # +1 or -1
    bpm_hold_start = None
    bpm_hold_last  = None

    # ── Helpers ───────────────────────────────────────────────────────────────

    def apply_library_result():
        """Write the selected sample path from the library into state and sequencer."""
        if not library.result:
            return
        if library.mode == "kit":
            folder = library.result
            if os.path.isdir(folder):
                wavs = sorted(
                    os.path.join(folder, f) for f in os.listdir(folder)
                    if f.lower().endswith(".wav") and not f.startswith("._")
                )
                for vi in range(NUM_VOICES):
                    if vi < len(wavs):
                        state.voices[vi]["sample"] = os.path.basename(wavs[vi])
                        sequencer.load_voice(vi, wavs[vi])
        elif library.mode == "loop":
            loop_player.load(library.result)
        else:  # "voice"
            vi = ui.cursor_voice
            state.voices[vi]["sample"] = os.path.basename(library.result)
            sequencer.load_voice(vi, library.result)

    # ── Hat (d-pad) handler ───────────────────────────────────────────────────

    def handle_hat(hat_val):
        nonlocal help_scroll
        if show_help:
            if hat_val == HAT_UP:
                help_scroll = max(0, help_scroll - 1)
            elif hat_val == HAT_DOWN:
                help_scroll = min(help_overlay.max_scroll(), help_scroll + 1)
            return
        if show_library:
            library.handle_hat(hat_val)
            return
        if show_chain:
            chain_ed.handle_hat(hat_val)
            return

        if a_held:
            # Velocity adjust mode while A is held
            if hat_val == HAT_UP:
                step = state.patterns[transport.current_pattern][ui.cursor_voice][ui.cursor_step]
                state.set_step_velocity(transport.current_pattern,
                                        ui.cursor_voice, ui.cursor_step,
                                        min(VELOCITY_MAX, step[1] + VELOCITY_STEP))
            elif hat_val == HAT_DOWN:
                step = state.patterns[transport.current_pattern][ui.cursor_voice][ui.cursor_step]
                state.set_step_velocity(transport.current_pattern,
                                        ui.cursor_voice, ui.cursor_step,
                                        max(VELOCITY_MIN, step[1] - VELOCITY_STEP))
            return

        if y_held:
            # Per-voice volume adjust while Y is held
            vi = ui.cursor_voice
            vol = state.voices[vi].get("volume", 1.0)
            if hat_val == HAT_UP:
                state.set_voice_volume(vi, vol + VOICE_VOLUME_STEP)
            elif hat_val == HAT_DOWN:
                state.set_voice_volume(vi, vol - VOICE_VOLUME_STEP)
            return

        if hat_val == HAT_LEFT:
            ui.cursor_step  = (ui.cursor_step - 1) % NUM_STEPS
        elif hat_val == HAT_RIGHT:
            ui.cursor_step  = (ui.cursor_step + 1) % NUM_STEPS
        elif hat_val == HAT_UP:
            ui.cursor_voice = max(0, ui.cursor_voice - 1)
        elif hat_val == HAT_DOWN:
            ui.cursor_voice = min(NUM_VOICES - 1, ui.cursor_voice + 1)

    # ── Button-down handler ───────────────────────────────────────────────────

    def handle_button_down(btn):
        nonlocal a_held, a_press_time, y_held, y_press_time
        nonlocal start_held, start_used, select_held, select_used
        nonlocal bpm_hold_dir, bpm_hold_start, bpm_hold_last
        nonlocal show_library, show_chain, show_help, help_scroll
        nonlocal start_select_since, function_held

        # ── Modifier keys: set flag, defer primary action to release ─────────
        if btn == BTN_START:
            start_held = True
            start_used = False
            if select_held:
                start_select_since = time.monotonic()
            return
        if btn == BTN_SELECT:
            select_held = True
            select_used = False
            if start_held:
                start_select_since = time.monotonic()
            if function_held:
                select_used = True
                show_help   = True
                help_scroll = 0
            return
        if btn == BTN_FUNCTION:
            function_held = True
            if select_held:
                select_used = True
                show_help   = True
                help_scroll = 0
            return

        # Track A-hold for velocity mode
        if btn == BTN_A:
            a_press_time = time.monotonic()
            a_held = False

        # Track Y-hold for per-voice volume mode
        if btn == BTN_Y:
            y_press_time = time.monotonic()
            y_held = False

        # ── Help overlay: A or B closes it ───────────────────────────────────
        if show_help:
            if btn == BTN_A or btn == BTN_B:
                show_help = False
            return

        # ── Route to overlays (non-modifier buttons) ─────────────────────────
        if show_library:
            library.handle_button(btn)
            if library.closed:
                show_library = False
                apply_library_result()
                state.favorites[:] = library._fav_sids
                a_press_time = None   # discard any pending toggle
            return
        if show_chain:
            chain_ed.handle_button(btn)
            if chain_ed.closed:
                show_chain = False
                a_press_time = None
            return

        # ── START-combo actions ───────────────────────────────────────────────
        if start_held:
            start_used = True
            if btn == BTN_A:
                a_press_time = None    # cancel pending step toggle
                loop_player.toggle()
            elif btn == BTN_B:
                library.open("voice", state.favorites)
                show_library = True
            elif btn == BTN_X:
                chain_ed.open()
                show_chain = True
            elif btn == BTN_Y:
                state.save()
                print("[HillBeat] State saved.")
            elif btn == BTN_R1:
                state.copy_pattern(transport.current_pattern)
            elif btn == BTN_L1:
                state.paste_pattern(transport.current_pattern)
            return

        # ── SELECT-combo actions ──────────────────────────────────────────────
        if select_held:
            select_used = True
            if btn == BTN_R2:
                transport.swing = min(SWING_MAX, state.swing + 5)
            elif btn == BTN_L2:
                transport.swing = max(SWING_MIN, state.swing - 5)
            return

        # ── Regular button actions ────────────────────────────────────────────
        if btn == BTN_A:
            pass   # handled on release (toggle_step) or hold+dpad (velocity)
        elif btn == BTN_Y:
            pass   # handled on release (mute toggle) or hold+dpad (volume)
        elif btn == BTN_R1:
            state.chain_enabled = False
            transport.queue_next_pattern()
        elif btn == BTN_L1:
            state.chain_enabled = False
            transport.queue_prev_pattern()
        elif btn == BTN_R2:
            bpm_hold_dir   = 1
            bpm_hold_start = time.monotonic()
            bpm_hold_last  = bpm_hold_start
            transport.bpm  = state.bpm + 1
            loop_player.reload_if_bpm_changed()
        elif btn == BTN_L2:
            bpm_hold_dir   = -1
            bpm_hold_start = time.monotonic()
            bpm_hold_last  = bpm_hold_start
            transport.bpm  = state.bpm - 1
            loop_player.reload_if_bpm_changed()

    # ── Button-up handler ─────────────────────────────────────────────────────

    def handle_button_up(btn):
        nonlocal a_held, a_press_time, y_held, y_press_time
        nonlocal start_held, start_used, select_held, select_used
        nonlocal bpm_hold_dir, bpm_hold_start, bpm_hold_last
        nonlocal start_select_since, function_held

        if btn == BTN_START:
            if start_select_since is not None:
                # Released before exit threshold → stop+reset
                if time.monotonic() - start_select_since < EXIT_HOLD_SECONDS:
                    sequencer.stop_and_reset()
                    transport.stop_and_reset()
                start_select_since = None
            elif not start_used:
                # START alone = play / pause
                sequencer.toggle()
            start_held = False
            start_used = False

        elif btn == BTN_SELECT:
            if start_select_since is not None:
                start_select_since = None    # BTN_START release will handle it
            elif not select_used:
                # SELECT alone = tap tempo
                bpm = transport.tap()
                ui.notify_tap(bpm)
                if bpm:
                    loop_player.reload_if_bpm_changed()
            select_held = False
            select_used = False

        elif btn == BTN_FUNCTION:
            function_held = False

        elif btn == BTN_A:
            # Quick-tap A = toggle step; held A was velocity-adjust mode
            if a_press_time is not None and not a_held:
                if time.monotonic() - a_press_time < A_HOLD_THRESHOLD:
                    state.toggle_step(transport.current_pattern,
                                      ui.cursor_voice, ui.cursor_step)
            a_held       = False
            a_press_time = None

        elif btn == BTN_Y:
            # Quick-tap Y = mute toggle; held Y was volume-adjust mode
            if y_press_time is not None and not y_held:
                if time.monotonic() - y_press_time < Y_HOLD_THRESHOLD:
                    state.voices[ui.cursor_voice]["muted"] = \
                        not state.voices[ui.cursor_voice]["muted"]
            y_held       = False
            y_press_time = None

        elif btn in (BTN_R2, BTN_L2):
            bpm_hold_dir   = 0
            bpm_hold_start = None
            bpm_hold_last  = None

    # ── Keyboard → button/hat map (dev/desktop fallback) ─────────────────────
    KB_HAT = {
        pygame.K_LEFT:  HAT_LEFT,
        pygame.K_RIGHT: HAT_RIGHT,
        pygame.K_UP:    HAT_UP,
        pygame.K_DOWN:  HAT_DOWN,
    }
    # Z=A  A=B  X=Y(mute)  S=X(chain/add)  Return=Start  RShift=Select  U=Function
    # Q=L1 E=R1 W=L2 D=R2  ]=R1 [=L1 .=R2 ,=L2
    KB_BTN = {
        pygame.K_z:            BTN_A,
        pygame.K_a:            BTN_B,
        pygame.K_x:            BTN_Y,
        pygame.K_s:            BTN_X,
        pygame.K_RETURN:       BTN_START,
        pygame.K_RSHIFT:       BTN_SELECT,
        pygame.K_u:            BTN_FUNCTION,
        pygame.K_e:            BTN_R1,
        pygame.K_q:            BTN_L1,
        pygame.K_RIGHTBRACKET: BTN_R1,
        pygame.K_LEFTBRACKET:  BTN_L1,
        pygame.K_d:            BTN_R2,
        pygame.K_w:            BTN_L2,
        pygame.K_PERIOD:       BTN_R2,
        pygame.K_COMMA:        BTN_L2,
    }

    # ── Main loop ─────────────────────────────────────────────────────────────
    running = True
    while running:
        now = time.monotonic()

        # START+SELECT held = quit
        if start_held and select_held and start_select_since is not None:
            if now - start_select_since >= EXIT_HOLD_SECONDS:
                running = False

        # A-hold threshold check
        if a_press_time is not None and not a_held:
            if now - a_press_time >= A_HOLD_THRESHOLD:
                a_held = True

        # Y-hold threshold check
        if y_press_time is not None and not y_held:
            if now - y_press_time >= Y_HOLD_THRESHOLD:
                y_held = True

        # BPM hold auto-repeat (+5/sec while R2 or L2 held)
        if bpm_hold_dir != 0 and bpm_hold_start is not None:
            if now - bpm_hold_start >= BPM_HOLD_DELAY:
                if bpm_hold_last is None or now - bpm_hold_last >= 0.2:
                    transport.bpm = state.bpm + bpm_hold_dir * BPM_HOLD_STEP
                    loop_player.reload_if_bpm_changed()
                    bpm_hold_last = now

        # ── Event processing ──────────────────────────────────────────────────
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                running = False

            elif event.type == pygame.JOYHATMOTION:
                if event.value != (0, 0):
                    handle_hat(event.value)

            elif event.type == pygame.JOYBUTTONDOWN:
                handle_button_down(event.button)

            elif event.type == pygame.JOYBUTTONUP:
                handle_button_up(event.button)

            elif event.type == pygame.KEYDOWN:
                if event.key == pygame.K_ESCAPE:
                    if show_help:
                        show_help = False
                    elif show_library:
                        show_library = False
                    elif show_chain:
                        show_chain = False
                    else:
                        running = False
                elif show_library:
                    # While library is open, raw keys go to it exclusively —
                    # no KB_BTN translation to avoid double-processing.
                    library.handle_key(event.key)
                    if library.closed:
                        show_library = False
                        apply_library_result()
                elif show_chain:
                    # While chain editor is open, same exclusive routing.
                    chain_ed.handle_key(event.key)
                    if chain_ed.closed:
                        show_chain = False
                elif event.key in KB_HAT:
                    handle_hat(KB_HAT[event.key])
                elif event.key in KB_BTN:
                    handle_button_down(KB_BTN[event.key])

            elif event.type == pygame.KEYUP:
                # Only fire button-up when no overlay is consuming input.
                if not show_library and not show_chain:
                    if event.key in KB_BTN:
                        handle_button_up(KB_BTN[event.key])

        # ── Draw ──────────────────────────────────────────────────────────────
        ui.draw(screen,
                show_library=show_library,
                show_chain=show_chain,
                library=library,
                chain_editor=chain_ed)

        if show_help:
            help_overlay.draw(screen, help_fonts, scroll=help_scroll)

        pygame.display.flip()
        clock.tick(FPS)

    # ── Shutdown ──────────────────────────────────────────────────────────────
    sequencer.stop()
    state.save()
    pygame.quit()
    sys.exit(0)


if __name__ == "__main__":
    main()
