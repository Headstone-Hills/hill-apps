use crate::grid::{GridApp, GridConfig, TrackKind};
use crate::Host;

static CONFIG: GridConfig = GridConfig {
    name: "HillBeat",
    file: "hillbeat",
    tracks: &[TrackKind::Drum, TrackKind::Drum, TrackKind::Drum, TrackKind::Drum],
    chords: false,
    key_overlay: false,
    multiselect: false,
    loop_player: true,
    drum_filter: false,
    help: &[
        ("PLAYBACK", ""),
        ("START", "play / pause"),
        ("SELECT", "tap tempo"),
        ("START+SELECT", "tap: stop & reset, hold 2s: quit"),
        ("L1 / R1", "previous / next pattern"),
        ("L2 / R2", "BPM - / + (hold repeats)"),
        ("SELECT+X", "swing on / off"),
        ("SELECT+L2/R2", "swing - / +"),
        ("EDITING", ""),
        ("D-pad", "move cursor (L/R step, U/D voice)"),
        ("A (tap)", "toggle step"),
        ("A (hold) + U/D", "step velocity"),
        ("Y (tap)", "mute / unmute voice"),
        ("Y (hold) + U/D", "voice volume"),
        ("SELECT+L1", "clear pattern"),
        ("SOUNDS & LOOP", ""),
        ("START+B", "pick a sample for this voice"),
        ("START+A", "loop on / off (picks a file first time)"),
        ("START+X", "choose a different loop file"),
        ("START+Y", "save"),
        ("FN+SELECT", "this help"),
    ],
};

pub fn new(host: Box<dyn Host>) -> GridApp {
    GridApp::new(&CONFIG, host)
}
