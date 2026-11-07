//! Note recording from the FM-1's keys (brief feature 11) and playing notes back.
//!
//! Notes are stored as frames on the project timeline. The app timestamps each MIDI event with the transport
//! position when it was read (minus the MIDI-input latency). The same note list can drive a sampler track
//! or be sent back to the FM-1 over MIDI, so a part can be re-recorded or retriggered.

use crate::project::NoteModel;
use crate::sampler;
use crate::transport::Transport;

/// Pairs note-ons with note-offs into notes. Retriggering a held note closes the earlier one; notes still held
/// at the end are closed there so nothing is left stuck.
#[derive(Default)]
pub struct NoteRecorder {
    open: Vec<(u8, u64, u8)>, // (note, start, vel)
    done: Vec<NoteModel>,
}

impl NoteRecorder {
    pub fn on_note_on(&mut self, frame: u64, note: u8, vel: u8) {
        self.close(frame, note);
        self.open.push((note, frame, vel.clamp(1, 127)));
    }

    pub fn on_note_off(&mut self, frame: u64, note: u8) {
        self.close(frame, note);
    }

    fn close(&mut self, frame: u64, note: u8) {
        if let Some(i) = self.open.iter().position(|o| o.0 == note) {
            let (n, start, vel) = self.open.remove(i);
            // a zero-length note (on and off in the same frame) is kept as 1 frame so it is not lost
            self.done.push(NoteModel { start, len: frame.saturating_sub(start).max(1), note: n, vel });
        }
    }

    pub fn held(&self) -> usize {
        self.open.len()
    }

    /// Stop recording at `frame`, closing anything still held. Notes come back sorted by start.
    pub fn finish(mut self, frame: u64) -> Vec<NoteModel> {
        let held: Vec<u8> = self.open.iter().map(|o| o.0).collect();
        for n in held {
            self.close(frame, n);
        }
        self.done.sort_by_key(|n| (n.start, n.note));
        self.done
    }
}

/// Move note starts towards the 1/`div`-beat grid by `strength` (0 = untouched, 1 = on the grid). Lengths are kept.
pub fn quantize(notes: &mut [NoteModel], tr: &Transport, div: u32, strength: f64) {
    let s = strength.clamp(0.0, 1.0);
    for n in notes.iter_mut() {
        let target = tr.snap(n.start, div) as f64;
        n.start = (n.start as f64 + (target - n.start as f64) * s).round() as u64;
    }
    notes.sort_by_key(|n| (n.start, n.note));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMsg {
    On { note: u8, vel: u8 },
    Off { note: u8 },
}

/// Note-on/off messages for a note list, sorted by frame. At the same frame offs come before ons, so a note
/// that ends exactly where the same pitch begins again retriggers instead of cutting the new note.
pub fn midi_schedule(notes: &[NoteModel]) -> Vec<(u64, MidiMsg)> {
    let mut v: Vec<(u64, MidiMsg)> = Vec::with_capacity(notes.len() * 2);
    for n in notes {
        v.push((n.start, MidiMsg::On { note: n.note, vel: n.vel }));
        v.push((n.start + n.len, MidiMsg::Off { note: n.note }));
    }
    v.sort_by_key(|(f, m)| (*f, matches!(m, MidiMsg::On { .. })));
    v
}

/// The slice of a sorted schedule with `from <= frame < to` (binary search; cheap in the audio/clock path).
pub fn events_in_range(sched: &[(u64, MidiMsg)], from: u64, to: u64) -> &[(u64, MidiMsg)] {
    let a = sched.partition_point(|(f, _)| *f < from);
    let b = sched.partition_point(|(f, _)| *f < to);
    &sched[a..b]
}

pub fn to_sampler_notes(notes: &[NoteModel]) -> Vec<sampler::Note> {
    notes.iter().map(|n| sampler::Note { start: n.start, end: Some(n.start + n.len), note: n.note, vel: n.vel }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nm(start: u64, len: u64, note: u8) -> NoteModel {
        NoteModel { start, len, note, vel: 90 }
    }

    #[test]
    fn pairs_ons_and_offs_including_overlaps_and_out_of_order_offs() {
        // the shape of the real FM-1 stream seen on the SP: overlapping notes, offs in a different order
        let mut r = NoteRecorder::default();
        r.on_note_on(1000, 68, 90);
        r.on_note_on(1010, 70, 90);
        r.on_note_off(2000, 68);
        r.on_note_off(2100, 70);
        r.on_note_on(3000, 73, 90);
        r.on_note_on(3200, 71, 90);
        r.on_note_off(3300, 71);
        r.on_note_off(3400, 73);
        let n = r.finish(10_000);
        assert_eq!(n, vec![nm(1000, 1000, 68), nm(1010, 1090, 70), nm(3000, 400, 73), nm(3200, 100, 71)]);
    }

    #[test]
    fn retrigger_and_held_notes_and_stray_offs() {
        let mut r = NoteRecorder::default();
        r.on_note_off(5, 60); // stray off: ignored
        r.on_note_on(100, 60, 90);
        r.on_note_on(200, 60, 100); // same key again while held: the first note ends here
        r.on_note_on(300, 64, 0); // velocity 0 on a note-on is clamped to 1, not dropped
        assert_eq!(r.held(), 2);
        let n = r.finish(500);
        assert_eq!(n.len(), 3);
        assert_eq!(n[0], NoteModel { start: 100, len: 100, note: 60, vel: 90 });
        assert_eq!(n[1], NoteModel { start: 200, len: 300, note: 60, vel: 100 });
        assert_eq!((n[2].note, n[2].vel, n[2].len), (64, 1, 200));
        assert_eq!(NoteRecorder::default().finish(10), vec![]);
        // on and off in the same frame still yields a 1-frame note
        let mut r = NoteRecorder::default();
        r.on_note_on(50, 60, 90);
        r.on_note_off(50, 60);
        assert_eq!(r.finish(60)[0].len, 1);
    }

    #[test]
    fn quantize_moves_starts_by_strength_and_keeps_lengths() {
        let mut tr = Transport::new(44100);
        tr.bpm = 60.0 * 44100.0 / 1000.0; // 1000 frames per beat
        let mut n = vec![nm(1090, 300, 60), nm(2400, 50, 62), nm(10, 10, 64)];
        quantize(&mut n, &tr, 1, 1.0);
        assert_eq!(n.iter().map(|x| x.start).collect::<Vec<_>>(), vec![0, 1000, 2000]);
        assert_eq!(n.iter().map(|x| x.len).collect::<Vec<_>>(), vec![10, 300, 50]);
        let mut h = vec![nm(1100, 5, 60)];
        quantize(&mut h, &tr, 1, 0.5);
        assert_eq!(h[0].start, 1050);
        let mut z = vec![nm(1100, 5, 60)];
        quantize(&mut z, &tr, 1, 0.0);
        assert_eq!(z[0].start, 1100);
    }

    #[test]
    fn midi_schedule_orders_events_and_retriggers_cleanly() {
        let s = midi_schedule(&[nm(1000, 500, 60), nm(1500, 200, 60), nm(900, 100, 64)]);
        let frames: Vec<u64> = s.iter().map(|e| e.0).collect();
        assert_eq!(frames, vec![900, 1000, 1000, 1500, 1500, 1700]);
        // at frame 1000 the note-off for 64 comes before the note-on for 60; at 1500 off(60) precedes on(60)
        assert_eq!(s[1].1, MidiMsg::Off { note: 64 });
        assert_eq!(s[2].1, MidiMsg::On { note: 60, vel: 90 });
        assert_eq!(s[3].1, MidiMsg::Off { note: 60 });
        assert_eq!(s[4].1, MidiMsg::On { note: 60, vel: 90 });
    }

    #[test]
    fn events_in_range_is_exact_and_chunk_independent() {
        let notes: Vec<NoteModel> = (0..50).map(|i| nm(i * 700 + 13, 300, 40 + (i % 12) as u8)).collect();
        let s = midi_schedule(&notes);
        let mut gathered = Vec::new();
        let mut pos = 0u64;
        while pos < 40_000 {
            gathered.extend_from_slice(events_in_range(&s, pos, pos + 1024));
            pos += 1024;
        }
        assert_eq!(gathered, s[..gathered.len()].to_vec());
        assert_eq!(gathered.len(), s.iter().filter(|e| e.0 < 40_960).count());
        assert!(events_in_range(&s, 5, 5).is_empty());
        assert!(events_in_range(&s, 10_000_000, 10_000_100).is_empty());
    }

    #[test]
    fn notes_convert_for_the_sampler() {
        let sn = to_sampler_notes(&[nm(100, 50, 60)]);
        assert_eq!((sn[0].start, sn[0].end, sn[0].note, sn[0].vel), (100, Some(150), 60, 90));
    }
}
