"""On-screen volume indicator.

The physical volume buttons adjust the system mixer (via wpctl/PipeWire on
Knulli, or an equivalent on other CFWs) directly — our app never sees a
button-press event for them at all. The only way to notice a change is to
poll the system volume periodically and pop up a toast when it moves. Polling
runs in a background thread so it can never stall the render loop; if
`wpctl` isn't available (e.g. dev machine), the OSD silently does nothing.
"""

import subprocess
import threading
import time

import pygame

_POLL_INTERVAL = 0.3   # seconds between system-volume checks
_DISPLAY_SECONDS = 1.5  # how long the toast stays up after a change


def _read_system_volume():
    """Return (level 0..1, muted bool) or None if unavailable."""
    try:
        out = subprocess.run(
            ["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"],
            capture_output=True, text=True, timeout=1.0,
        ).stdout.strip()
        if not out.startswith("Volume:"):
            return None
        parts = out.split()
        return float(parts[1]), "[MUTED]" in out
    except Exception:
        return None


class VolumeOSD:
    def __init__(self):
        self._level = None
        self._muted = False
        self._visible_until = 0.0
        self._stop = False
        self._thread = threading.Thread(target=self._poll_loop, daemon=True)
        self._thread.start()

    def _poll_loop(self):
        while not self._stop:
            result = _read_system_volume()
            if result is not None:
                level, muted = result
                if self._level is not None and (level != self._level or muted != self._muted):
                    self._visible_until = time.monotonic() + _DISPLAY_SECONDS
                self._level, self._muted = level, muted
            time.sleep(_POLL_INTERVAL)

    def stop(self):
        self._stop = True

    def draw(self, surf, font):
        if self._level is None or time.monotonic() >= self._visible_until:
            return

        w, h = surf.get_size()
        box_w, box_h = 220, 56
        x, y = (w - box_w) // 2, h - box_h - 24

        box = pygame.Surface((box_w, box_h), pygame.SRCALPHA)
        box.fill((10, 10, 18, 210))
        pygame.draw.rect(box, (120, 140, 190), box.get_rect(), width=2, border_radius=10)

        pct = round(self._level * 100)
        label = "MUTED" if self._muted else f"VOLUME {pct}%"
        text = font.render(label, True, (240, 240, 255))
        box.blit(text, ((box_w - text.get_width()) // 2, 8))

        bar_x, bar_y, bar_w, bar_h = 16, 34, box_w - 32, 10
        pygame.draw.rect(box, (45, 45, 60), (bar_x, bar_y, bar_w, bar_h), border_radius=5)
        if not self._muted:
            fill_w = int(bar_w * max(0.0, min(1.0, self._level)))
            if fill_w > 0:
                pygame.draw.rect(box, (80, 180, 255), (bar_x, bar_y, fill_w, bar_h), border_radius=5)

        surf.blit(box, (x, y))
