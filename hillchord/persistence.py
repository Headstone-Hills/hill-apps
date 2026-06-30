"""Load/save AppState to JSON (spec: save automatically on exit, load on start)."""

import json
import os

import config
from state import AppState


def load_state(state: AppState) -> None:
    """Populate `state` from the JSON file if it exists; otherwise leave defaults."""
    try:
        with open(config.STATE_PATH, "r") as f:
            data = json.load(f)
        state.load_dict(data)
    except FileNotFoundError:
        pass
    except (json.JSONDecodeError, OSError) as e:
        print(f"[hillchord] could not load state: {e}")


def save_state(state: AppState) -> None:
    """Write the persisted subset of `state` to JSON."""
    try:
        os.makedirs(os.path.dirname(config.STATE_PATH), exist_ok=True)
        tmp = config.STATE_PATH + ".tmp"
        with open(tmp, "w") as f:
            json.dump(state.to_dict(), f, indent=2)
        os.replace(tmp, config.STATE_PATH)  # atomic
    except OSError as e:
        print(f"[hillchord] could not save state: {e}")
