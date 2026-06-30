"""Play screen HUD: key, tonality, mode, effects, wet/dry, loop status."""

import pygame

from audio import loop_recorder as lr


def _chip(surf, font, text, x, y, on):
    bg = (60, 140, 90) if on else (40, 40, 50)
    fg = (240, 255, 240) if on else (120, 120, 130)
    label = font.render(text, True, fg)
    w = label.get_width() + 24
    pygame.draw.rect(surf, bg, (x, y, w, 34), border_radius=6)
    surf.blit(label, (x + 12, y + 7))
    return x + w + 10


def draw(surf, state, looper, fonts, mixer=None, sustain=False):
    surf.fill((14, 14, 20))

    # Key + tonality (big, centered-ish).
    tonality = "minor" if state.minor else "major"
    key_txt = fonts["huge"].render(f"{state.key} {tonality}", True, (250, 250, 255))
    surf.blit(key_txt, (20, 24))

    # Now-playing chord/note, top-right corner (dynamic).
    if state.now_playing:
        np_txt = fonts["huge"].render(state.now_playing, True, (255, 220, 120))
        surf.blit(np_txt, (surf.get_width() - np_txt.get_width() - 20, 24))

    # One-time "caching" feedback: shown briefly after a from-scratch render
    # (the only time there's a note-on hitch; instant from then on).
    if mixer is not None and mixer.computing_recently():
        c = fonts["small"].render("* caching...", True, (120, 200, 255))
        surf.blit(c, (surf.get_width() - c.get_width() - 20, 92))

    arping = state.arp != "off"
    oct_str = f"  oct {state.octave:+d}" if state.octave else ""
    arp_str = f"   ARP {state.arp}" if arping else ""
    mode_txt = fonts["mid"].render(
        f"{state.mode.upper()} MODE{oct_str}{arp_str}", True,
        (255, 180, 90) if arping else (160, 170, 220))
    surf.blit(mode_txt, (20, 96))

    sound_txt = fonts["small"].render(
        f"Sound: {state.sound or '(none — open Library)'}",
        True, (150, 150, 170))
    surf.blit(sound_txt, (20, 132))

    bpm_txt = fonts["small"].render(f"BPM {state.bpm}", True, (150, 150, 170))
    surf.blit(bpm_txt, (20, 158))

    # Effects chips.
    fx = state.effects
    x = 20
    y = 210
    x = _chip(surf, fonts["mid"], "REVERB", x, y, fx.reverb)
    x = _chip(surf, fonts["mid"], "DELAY", x, y, fx.delay)
    x = _chip(surf, fonts["mid"], "CHORUS", x, y, fx.chorus)
    crush_label = (f"CRUSH {fx.crush_bits}b/{fx.crush_down}x"
                   if fx.crushing() else "CRUSH")
    x = _chip(surf, fonts["mid"], crush_label, x, y, fx.crushing())
    x = _chip(surf, fonts["mid"], "SUS", x, y, sustain)
    wet_txt = fonts["mid"].render(f"Wet/Dry {fx.wetdry}%", True, (200, 200, 220))
    surf.blit(wet_txt, (20, y + 50))

    # Loop status (ASCII so it renders on the device's default font).
    loop_label = {
        lr.IDLE: "Loop: empty",
        lr.RECORDING: "Loop: (*) RECORDING",
        lr.LOOPING: "Loop: > playing",
        lr.OVERDUB: "Loop: (+) OVERDUB",
    }.get(looper.state, "")
    loop_color = ((255, 90, 90) if looper.state in (lr.RECORDING, lr.OVERDUB)
                  else (160, 200, 160))
    surf.blit(fonts["mid"].render(loop_label, True, loop_color), (20, y + 90))

    hint = fonts["small"].render(
        "Select+R1/L1: key   hold Select=sustain   Start: library   Fn: chord/note   "
        "R1/L1: fx   R2: wet/dry   L2: loop",
        True, (110, 110, 130))
    surf.blit(hint, (20, 452))
