"""Project save/load: versioned JSON, atomic writes, autosave + recovery file.

Layout inside the project dir:
  slot1.json .. slot8.json   explicit saves
  recovery.json              autosave; present only if the app did not exit cleanly
                             or the project has unsaved edits
"""

from __future__ import annotations

import json
import os
import time

from model import Project, project_from_dict, project_to_dict

NUM_SLOTS = 8


def slot_path(dirpath, slot):
    return os.path.join(dirpath, "slot%d.json" % slot)


def recovery_path(dirpath):
    return os.path.join(dirpath, "recovery.json")


def _write_atomic(path, data):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(data, f, separators=(",", ":"))
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def save_project(project, path):
    _write_atomic(path, project_to_dict(project))


def load_project(path):
    with open(path, "r") as f:
        return project_from_dict(json.load(f))


def slot_exists(dirpath, slot):
    return os.path.isfile(slot_path(dirpath, slot))


def slot_label(dirpath, slot):
    p = slot_path(dirpath, slot)
    try:
        with open(p, "r") as f:
            return json.load(f).get("name", "?")
    except (OSError, ValueError):
        return None


class AutoSaver(object):
    """Call tick(project, dirty) from the UI loop; writes recovery.json at most every
    `interval` seconds while dirty. File I/O stays out of the clock thread."""

    def __init__(self, dirpath, interval=30.0):
        self.dir, self.interval = dirpath, interval
        self._last = time.monotonic()
        self.last_error = ""

    def tick(self, project, dirty):
        now = time.monotonic()
        if dirty and now - self._last >= self.interval:
            self.write(project)

    def write(self, project):
        self._last = time.monotonic()
        try:
            save_project(project, recovery_path(self.dir))
            self.last_error = ""
            return True
        except OSError as e:
            self.last_error = str(e)
            return False

    def clear(self):
        try:
            os.remove(recovery_path(self.dir))
        except OSError:
            pass

    def has_recovery(self):
        return os.path.isfile(recovery_path(self.dir))

    def load_recovery(self):
        try:
            return load_project(recovery_path(self.dir))
        except (OSError, ValueError, KeyError, TypeError):
            return None
