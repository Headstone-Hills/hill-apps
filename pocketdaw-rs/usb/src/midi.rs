//! USB-MIDI event packets: `[cable<<4 | CIN, status, d1, d2]`.

pub const CIN_NOTE_OFF: u8 = 0x08;
pub const CIN_NOTE_ON: u8 = 0x09;
pub const CIN_CC: u8 = 0x0B;
pub const CIN_PC: u8 = 0x0C;
pub const CIN_PB: u8 = 0x0E;

pub fn note_on(ch: u8, note: u8, vel: u8) -> [u8; 4] {
    [CIN_NOTE_ON, 0x90 | (ch & 15), note & 127, vel.clamp(1, 127)] // velocity 0 would mean note-off
}
pub fn note_off(ch: u8, note: u8) -> [u8; 4] {
    [CIN_NOTE_OFF, 0x80 | (ch & 15), note & 127, 0]
}
pub fn cc(ch: u8, cc: u8, val: u8) -> [u8; 4] {
    [CIN_CC, 0xB0 | (ch & 15), cc & 127, val & 127]
}
pub fn program_change(ch: u8, prog: u8) -> [u8; 4] {
    [CIN_PC, 0xC0 | (ch & 15), prog & 127, 0]
}
pub fn pitch_bend(ch: u8, v14: u16) -> [u8; 4] {
    let v = v14.min(16383);
    [CIN_PB, 0xE0 | (ch & 15), (v & 127) as u8, ((v >> 7) & 127) as u8]
}

/// CC123 (all notes off) + CC120 (all sound off) on the given channels.
pub fn panic(channels: impl IntoIterator<Item = u8>) -> Vec<u8> {
    let mut out = Vec::new();
    for ch in channels {
        out.extend_from_slice(&cc(ch, 123, 0));
        out.extend_from_slice(&cc(ch, 120, 0));
    }
    out
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Event {
    NoteOn { ch: u8, note: u8, vel: u8 },
    NoteOff { ch: u8, note: u8 },
    Cc { ch: u8, cc: u8, val: u8 },
    Program { ch: u8, prog: u8 },
    Bend { ch: u8, value: u16 },
    Other([u8; 4]),
}

/// Decode concatenated packets. All-zero padding packets are skipped. A note-on with velocity 0
/// is reported as a note-off (the MIDI convention; the FM-1's V15 firmware does the same).
pub fn decode(data: &[u8]) -> Vec<Event> {
    data.chunks_exact(4)
        .filter(|p| *p != [0, 0, 0, 0])
        .map(|p| {
            let ch = p[1] & 15;
            match (p[0] & 15, p[1] & 0xF0) {
                (CIN_NOTE_ON, 0x90) if p[3] == 0 => Event::NoteOff { ch, note: p[2] },
                (CIN_NOTE_ON, 0x90) => Event::NoteOn { ch, note: p[2], vel: p[3] },
                (CIN_NOTE_OFF, 0x80) => Event::NoteOff { ch, note: p[2] },
                (CIN_CC, 0xB0) => Event::Cc { ch, cc: p[2], val: p[3] },
                (CIN_PC, 0xC0) => Event::Program { ch, prog: p[2] },
                (CIN_PB, 0xE0) => Event::Bend { ch, value: p[2] as u16 | (p[3] as u16) << 7 },
                _ => Event::Other([p[0], p[1], p[2], p[3]]),
            }
        })
        .collect()
}

pub fn describe(e: &Event) -> String {
    match *e {
        Event::NoteOn { ch, note, vel } => format!("ch{} NOTE ON {} vel {}", ch + 1, note, vel),
        Event::NoteOff { ch, note } => format!("ch{} NOTE OFF {}", ch + 1, note),
        Event::Cc { ch, cc, val } => format!("ch{} CC {} = {}", ch + 1, cc, val),
        Event::Program { ch, prog } => format!("ch{} PROGRAM {}", ch + 1, prog),
        Event::Bend { ch, value } => format!("ch{} BEND {}", ch + 1, value),
        Event::Other(p) => format!("raw {:02x} {:02x} {:02x} {:02x}", p[0], p[1], p[2], p[3]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_match_the_proven_python_ones() {
        assert_eq!(note_on(0, 60, 100), [0x09, 0x90, 60, 100]); // the reference script from the brief
        assert_eq!(note_off(1, 60), [0x08, 0x81, 60, 0]);
        assert_eq!(cc(2, 74, 64), [0x0B, 0xB2, 74, 64]);
        assert_eq!(program_change(0, 12), [0x0C, 0xC0, 12, 0]);
        assert_eq!(pitch_bend(0, 8192), [0x0E, 0xE0, 0, 64]);
        assert_eq!(note_on(0, 60, 0)[3], 1);
    }

    #[test]
    fn decode_roundtrip_and_padding() {
        let mut d = Vec::new();
        d.extend_from_slice(&note_on(0, 67, 90));
        d.extend_from_slice(&[0, 0, 0, 0]);
        d.extend_from_slice(&[CIN_NOTE_ON, 0x90, 67, 0]); // running-status style note-off
        d.extend_from_slice(&note_off(0, 65));
        d.extend_from_slice(&pitch_bend(3, 12345));
        assert_eq!(
            decode(&d),
            vec![
                Event::NoteOn { ch: 0, note: 67, vel: 90 },
                Event::NoteOff { ch: 0, note: 67 },
                Event::NoteOff { ch: 0, note: 65 },
                Event::Bend { ch: 3, value: 12345 },
            ]
        );
        assert_eq!(describe(&Event::NoteOn { ch: 0, note: 60, vel: 90 }), "ch1 NOTE ON 60 vel 90");
    }

    #[test]
    fn panic_hits_every_channel() {
        assert_eq!(panic(0..16).len(), 16 * 2 * 4);
    }
}
