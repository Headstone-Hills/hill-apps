#!/usr/bin/env python3
"""On-device input probe. Run on the RG35XXSP, press each button, and note the
reported index/hat values to fill in input/button_map.py JOY_BUTTONS.
"""

import pygame

pygame.init()
pygame.joystick.init()
print("joysticks:", pygame.joystick.get_count())
js = []
for i in range(pygame.joystick.get_count()):
    j = pygame.joystick.Joystick(i)
    j.init()
    js.append(j)
    print(f"  [{i}] {j.get_name()}  buttons={j.get_numbuttons()} hats={j.get_numhats()}")

pygame.display.set_mode((320, 240))
print("Press buttons (close window to quit)...")
running = True
while running:
    for e in pygame.event.get():
        if e.type == pygame.QUIT:
            running = False
        elif e.type == pygame.JOYBUTTONDOWN:
            print("BUTTON DOWN index:", e.button)
        elif e.type == pygame.JOYHATMOTION:
            print("HAT:", e.value)
        elif e.type == pygame.JOYAXISMOTION and abs(e.value) > 0.5:
            print("AXIS:", e.axis, round(e.value, 2))
pygame.quit()
