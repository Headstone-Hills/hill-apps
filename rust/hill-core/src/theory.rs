pub const PC_NAMES: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

pub fn note_name(midi: u8) -> String {
    format!("{}{}", PC_NAMES[(midi % 12) as usize], (midi as i32 / 12) - 1)
}

pub struct ChordType {
    pub name: &'static str,
    pub intervals: &'static [i32],
}

pub const CHORD_TYPES: [ChordType; 14] = [
    ChordType { name: "unison", intervals: &[0] },
    ChordType { name: "power", intervals: &[0, 7] },
    ChordType { name: "major", intervals: &[0, 4, 7] },
    ChordType { name: "minor", intervals: &[0, 3, 7] },
    ChordType { name: "sus2", intervals: &[0, 2, 7] },
    ChordType { name: "sus4", intervals: &[0, 5, 7] },
    ChordType { name: "dom7", intervals: &[0, 4, 7, 10] },
    ChordType { name: "maj7", intervals: &[0, 4, 7, 11] },
    ChordType { name: "min7", intervals: &[0, 3, 7, 10] },
    ChordType { name: "dim", intervals: &[0, 3, 6] },
    ChordType { name: "dim7", intervals: &[0, 3, 6, 9] },
    ChordType { name: "aug", intervals: &[0, 4, 8] },
    ChordType { name: "add9", intervals: &[0, 4, 7, 14] },
    ChordType { name: "minadd9", intervals: &[0, 3, 7, 14] },
];

pub struct ScaleMode {
    pub name: &'static str,
    pub intervals: &'static [i32],
}

/// Index 0 ("none") means "repeat the root".
pub const SCALE_MODES: [ScaleMode; 11] = [
    ScaleMode { name: "none", intervals: &[0] },
    ScaleMode { name: "major", intervals: &[0, 2, 4, 5, 7, 9, 11] },
    ScaleMode { name: "minor", intervals: &[0, 2, 3, 5, 7, 8, 10] },
    ScaleMode { name: "penta_maj", intervals: &[0, 2, 4, 7, 9] },
    ScaleMode { name: "penta_min", intervals: &[0, 3, 5, 7, 10] },
    ScaleMode { name: "blues", intervals: &[0, 3, 5, 6, 7, 10] },
    ScaleMode { name: "dorian", intervals: &[0, 2, 3, 5, 7, 9, 10] },
    ScaleMode { name: "mixolyd", intervals: &[0, 2, 4, 5, 7, 9, 10] },
    ScaleMode { name: "phrygian", intervals: &[0, 1, 3, 5, 7, 8, 10] },
    ScaleMode { name: "lydian", intervals: &[0, 2, 4, 6, 7, 9, 11] },
    ScaleMode { name: "chromatic", intervals: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] },
];

pub fn clamp_note(n: i32) -> u8 {
    n.clamp(0, 127) as u8
}

/// `count` ascending scale notes starting at `root` (midi), wrapping upward
/// through octaves.
pub fn scale_notes(root: i32, scale: usize, count: usize) -> Vec<u8> {
    let iv = SCALE_MODES[scale.min(SCALE_MODES.len() - 1)].intervals;
    (0..count)
        .map(|i| clamp_note(root + iv[i % iv.len()] + 12 * (i / iv.len()) as i32))
        .collect()
}

/// Name a chord built on `root_pc` with the given sorted intervals above it.
pub fn chord_name(root_pc: u8, intervals: &[i32], suffix: &str) -> String {
    let has = |i: i32| intervals.contains(&i);
    let q = if has(4) && has(7) {
        ""
    } else if has(3) && has(7) {
        "m"
    } else if has(3) && has(6) {
        "dim"
    } else if has(4) && has(8) {
        "aug"
    } else {
        ""
    };
    format!("{}{}{}", PC_NAMES[(root_pc % 12) as usize], q, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(0), "C-1");
    }

    #[test]
    fn scale_spread() {
        let n = scale_notes(48, 3, 8); // C major pentatonic, 5 notes then octave up
        assert_eq!(n, vec![48, 50, 52, 55, 57, 60, 62, 64]);
    }

    #[test]
    fn tables_sorted_and_sized() {
        assert_eq!(CHORD_TYPES.len(), 14);
        assert_eq!(SCALE_MODES.len(), 11);
        for s in SCALE_MODES.iter() {
            assert!(s.intervals.windows(2).all(|w| w[0] < w[1]));
        }
    }

    #[test]
    fn chord_names() {
        assert_eq!(chord_name(0, &[0, 3, 7], ""), "Cm");
        assert_eq!(chord_name(7, &[0, 4, 7], "7"), "G7");
        assert_eq!(chord_name(11, &[0, 3, 6], ""), "Bdim");
    }
}
