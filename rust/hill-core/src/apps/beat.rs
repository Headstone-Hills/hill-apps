use crate::grid::{GridApp, GridConfig, TrackKind};
use crate::Host;

static CONFIG: GridConfig = GridConfig {
    name: "HillBeat",
    file: "hillbeat",
    tracks: &[TrackKind::Drum; 8],
    chords: false,
    key_overlay: false,
    multiselect: false,
    drum_filter: true,
    decay: false,
    presets: true,
    vol_boost: true,
    help: &[
        ("PLAYBACK", ""),
        ("START", "play / pause"),
        ("SELECT", "tap tempo"),
        ("START+SELECT", "tap: stop & reset, hold 1.5s: quit"),
        ("L1 / R1", "previous / next pattern"),
        ("L2 / R2", "BPM - / + (hold repeats)"),
        ("SELECT+X", "swing on / off"),
        ("SELECT+L2/R2", "swing - / +"),
        ("EDITING", ""),
        ("D-pad", "move cursor (L/R step, U/D voice)"),
        ("A (tap)", "toggle step"),
        ("A (hold) + U/D", "step velocity"),
        ("Y (tap)", "mute / unmute voice"),
        ("Y (hold) + U/D", "voice volume (to 400%)"),
        ("SELECT+L1", "clear pattern"),
        ("SOUNDS & LOOP", ""),
        ("START+B", "drum sounds (X: show all). [KIT] fills all voices"),
        ("START+L1", "drum pattern presets (again: close)"),
        ("  in presets", "A open/load  B back  X listen  L1/R1 style"),
        ("START+Y", "save"),
        ("FN+SELECT", "this help"),
    ],
};

pub fn new(host: Box<dyn Host>) -> GridApp {
    GridApp::new(&CONFIG, host)
}
