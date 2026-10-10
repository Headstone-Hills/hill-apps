use crate::grid::{GridApp, GridConfig, TrackKind};
use crate::Host;

static CONFIG: GridConfig = GridConfig {
    name: "HillSequencer",
    file: "hillsequencer",
    tracks: &[TrackKind::Pitched; 8],
    chords: false,
    key_overlay: true,
    multiselect: true,
    loop_player: false,
    drum_filter: false,
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
        ("X (hold) + L/R", "transpose track +/- 1 semitone"),
        ("X (hold) + U/D", "transpose track +/- 1 octave"),
        ("SELECT+L1", "clear pattern"),
        ("SOUNDS & KEY", ""),
        ("START+B", "pick an instrument for the track"),
        ("START+R2", "add / remove track from multi-select"),
        ("  then START+B", "assigns the sound to every selected track"),
        ("START+L2", "key / scale: spreads a scale over all tracks"),
        ("START+Y", "save"),
        ("FN+SELECT", "this help"),
    ],
};

pub fn new(host: Box<dyn Host>) -> GridApp {
    GridApp::new(&CONFIG, host)
}
