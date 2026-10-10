use crate::grid::{GridApp, GridConfig, TrackKind};
use crate::Host;

use TrackKind::{Drum, Pitched};

static CONFIG: GridConfig = GridConfig {
    name: "HillBand",
    file: "hillband",
    tracks: &[Pitched, Pitched, Pitched, Pitched, Drum, Drum, Drum, Drum],
    chords: true,
    key_overlay: false,
    multiselect: true,
    drum_filter: true,
    decay: true,
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
        ("D-pad", "move cursor (L/R step, U/D track)"),
        ("A (tap)", "toggle step"),
        ("A (hold) + U/D", "step velocity"),
        ("Y (tap)", "mute / unmute track"),
        ("Y (hold) + U/D", "track volume"),
        ("X (hold) + D-pad", "transpose (melodic tracks 1-4)"),
        ("SELECT+L1", "clear pattern"),
        ("CHORDS & SCALES", ""),
        ("START+L2", "chord type + scale mode for a melodic track"),
        ("  in overlay", "L/R chord, U/D scale, A apply, B cancel"),
        ("  scale active", "each step moves up one scale degree"),
        ("SOUNDS & LOOP", ""),
        ("START+B", "pick a sound (drum tracks start on DRUM filter)"),
        ("  X in browser", "clear the drum filter"),
        ("START+R2", "select all melodic or all drum tracks (toggle), then START+B"),
        ("START+Y", "save"),
        ("FN+SELECT", "this help"),
    ],
};

pub fn new(host: Box<dyn Host>) -> GridApp {
    GridApp::new(&CONFIG, host)
}
