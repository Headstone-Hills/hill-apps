"""Logical button names + raw-input mappings.

Two input sources are supported:
  * Keyboard  -> for desktop development on the Mac.
  * Joystick  -> the RG35XXSP's gamepad on muOS.

The joystick button indices below are PLACEHOLDERS and must be verified
on-device (see tools/probe_input.py). They are isolated here so remapping is a
one-file change.
"""

import pygame

# --- Logical buttons --------------------------------------------------------
A, B, X, Y = "A", "B", "X", "Y"
L1, R1, L2, R2 = "L1", "R1", "L2", "R2"
SELECT, START, FUNCTION = "SELECT", "START", "FUNCTION"
UP, DOWN, LEFT, RIGHT = "UP", "DOWN", "LEFT", "RIGHT"

DPAD = {UP, DOWN, LEFT, RIGHT}

# --- Keyboard mapping -------------------------------------------------------
# On-device these keys are produced by gptokeyb (see hillchord.gptk); on the
# desktop they double as the dev keybinds. The two MUST stay in sync.
KEYBOARD = {
    pygame.K_a: A, pygame.K_b: B, pygame.K_x: X, pygame.K_y: Y,
    pygame.K_q: L1, pygame.K_w: R1, pygame.K_e: L2, pygame.K_r: R2,
    pygame.K_o: SELECT, pygame.K_p: START, pygame.K_u: FUNCTION,
    pygame.K_UP: UP, pygame.K_DOWN: DOWN, pygame.K_LEFT: LEFT, pygame.K_RIGHT: RIGHT,
}

# --- Joystick button mapping (RG35XXSP, verified on hardware) ----------------
JOY_BUTTONS = {
    3: A, 4: B, 6: X, 5: Y,
    7: L1, 8: R1, 12: L2, 13: R2,
    9: SELECT, 10: START, 11: FUNCTION,
}

# D-pad arrives as a hat (-1/0/1 per axis) on most SDL gamepads.
def hat_to_dirs(value):
    """(x, y) hat tuple -> set of logical D-pad directions."""
    x, y = value
    dirs = set()
    if y == 1:
        dirs.add(UP)
    elif y == -1:
        dirs.add(DOWN)
    if x == 1:
        dirs.add(RIGHT)
    elif x == -1:
        dirs.add(LEFT)
    return dirs
