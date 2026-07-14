#!/usr/bin/env python3
"""Minimal loading screen shown while install_deps finishes on first launch.

Runs standalone once pygame itself is installed (before numpy, before any
app module is importable). Exits cleanly on SIGTERM so the display handshake
with the compositor stays intact for main.py to take over right after —
an abrupt kill of a raw SDL/pygame process can leave the display in a bad
state on some CFWs.
"""

import os
import signal
import sys

import pygame

_stop = False


def _handle_term(signum, frame):
    global _stop
    _stop = True


signal.signal(signal.SIGTERM, _handle_term)

pygame.init()
SCREEN_W, SCREEN_H = 640, 480
if sys.platform == "darwin":
    screen = pygame.display.set_mode((SCREEN_W, SCREEN_H))
else:
    screen = pygame.display.set_mode((SCREEN_W, SCREEN_H), pygame.FULLSCREEN | pygame.NOFRAME)

_splash_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "splash.png")
_splash = pygame.image.load(_splash_path) if os.path.isfile(_splash_path) else None
_font = pygame.font.SysFont(None, 26)

clock = pygame.time.Clock()
frame = 0
while not _stop:
    for e in pygame.event.get():
        if e.type == pygame.QUIT:
            _stop = True

    screen.fill((10, 10, 16))
    if _splash:
        screen.blit(_splash, (0, 0))

    frame += 1
    dots = "." * (1 + (frame // 10) % 3)
    label = f"Installing dependencies{dots} (first run only, needs wifi)"
    text = _font.render(label, True, (240, 240, 255))

    bar_h = text.get_height() + 18
    bar = pygame.Surface((SCREEN_W, bar_h), pygame.SRCALPHA)
    bar.fill((10, 10, 18, 215))
    screen.blit(bar, (0, SCREEN_H - bar_h))
    screen.blit(text, ((SCREEN_W - text.get_width()) // 2, SCREEN_H - bar_h + 9))

    pygame.display.flip()
    clock.tick(10)

pygame.quit()
