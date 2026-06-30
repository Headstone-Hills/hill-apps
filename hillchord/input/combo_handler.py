"""Translate raw pygame events into a per-frame logical input picture.

Responsibilities:
  * Track the held-button set and per-frame pressed/released edges (D-pad from
    the keyboard arrows or the gamepad hat are unified into the same edges).
  * Detect L2 held >= 2s (loop cancel). The L2 press itself is a normal button
    edge in `pressed`, so record start/stop is instant (no tap deferral).
D-pad modifier latching lives in Actions (it needs to know mode/Function).
"""

import time
from dataclasses import dataclass, field

import pygame

import config
from input import button_map as bm


@dataclass
class InputFrame:
    pressed: set = field(default_factory=set)
    released: set = field(default_factory=set)
    held: set = field(default_factory=set)
    dpad: set = field(default_factory=set)      # effective (held u latched)
    l2_hold: bool = False                        # L2 held >= 2s (cancel loop)


class ComboHandler:
    def __init__(self):
        self.held = set()
        self._hat = set()
        self._prev_dirs = set()        # effective D-pad set last frame (edges)

        # L2 hold-to-cancel state (the press itself is a normal button edge,
        # so no tap/double deferral -> record start/stop is instant).
        self._l2_down_at = None
        self._l2_hold_fired = False

        self.joystick = None
        if pygame.joystick.get_count() > 0:
            self.joystick = pygame.joystick.Joystick(0)
            self.joystick.init()

    # ---- raw event ingestion ----
    def _logical(self, event):
        if event.type == pygame.KEYDOWN:
            return bm.KEYBOARD.get(event.key), True
        if event.type == pygame.KEYUP:
            return bm.KEYBOARD.get(event.key), False
        if event.type == pygame.JOYBUTTONDOWN:
            return bm.JOY_BUTTONS.get(event.button), True
        if event.type == pygame.JOYBUTTONUP:
            return bm.JOY_BUTTONS.get(event.button), False
        return None, None

    def process(self, events) -> InputFrame:
        frame = InputFrame()
        now = time.monotonic()

        for event in events:
            if event.type == pygame.JOYHATMOTION:
                self._hat = bm.hat_to_dirs(event.value)
                continue
            btn, down = self._logical(event)
            if btn is None:
                continue
            if down:
                # D-pad edges are handled uniformly below (works for both the
                # keyboard arrows and the gamepad hat); only track non-dpad
                # button edges here.
                if btn not in self.held and btn not in bm.DPAD:
                    frame.pressed.add(btn)
                    self._on_press(btn, now)
                self.held.add(btn)
            else:
                if btn in self.held and btn not in bm.DPAD:
                    frame.released.add(btn)
                    self._on_release(btn, now)
                self.held.discard(btn)

        # Unified D-pad edges: keyboard arrows (in `held`) + gamepad hat (`_hat`).
        # Latching is NOT done here — Actions handles it, gated to chord mode and
        # not while Function (octave) is held, so note mode / octave changes
        # never leave chord modifiers stuck on.
        held_dirs = (self.held & bm.DPAD) | self._hat
        for d in held_dirs - self._prev_dirs:     # newly pressed directions
            frame.pressed.add(d)
        for d in self._prev_dirs - held_dirs:     # released directions
            frame.released.add(d)
        self._prev_dirs = held_dirs

        frame.held = self.held
        frame.dpad = held_dirs                    # raw held directions only

        frame.l2_hold = self._resolve_l2_hold(now)
        return frame

    def _on_press(self, btn, now):
        if btn == bm.L2:
            self._l2_down_at = now
            self._l2_hold_fired = False

    def _on_release(self, btn, now):
        if btn == bm.L2:
            self._l2_down_at = None

    def _resolve_l2_hold(self, now) -> bool:
        """True on the frame L2 has been held for the cancel threshold."""
        if self._l2_down_at is not None and not self._l2_hold_fired:
            if (now - self._l2_down_at) * 1000 >= config.LOOP_CANCEL_HOLD_MS:
                self._l2_hold_fired = True
                self._l2_down_at = None
                return True
        return False
