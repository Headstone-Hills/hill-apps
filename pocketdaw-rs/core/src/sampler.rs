//! Sampler: one sample pitched across the keyboard by resampling (varispeed), with a root note.
//!
//! Notes are stateless events `(start, end, note, vel)`: the output at any frame is computed directly from the
//! note list, so playback can start anywhere, loop, and be rendered in any chunk size with identical results.
//! Live keyboard playing uses the same renderer (note-on adds a Note with `end: None`, note-off sets `end`),
//! and a recorded performance is just the stored note list (feature 11).

use crate::mixer::Source;
use std::sync::Arc;

#[derive(Clone)]
pub struct SampleData {
    /// interleaved stereo
    pub audio: Arc<Vec<f32>>,
    pub rate: u32,
    /// the MIDI note at which the sample plays back at its original pitch
    pub root_note: u8,
    pub tune_cents: f32,
    /// sustain loop `[start, end)` in sample frames; None = play once
    pub loop_region: Option<(u64, u64)>,
}

impl SampleData {
    pub fn frames(&self) -> u64 {
        (self.audio.len() / 2) as u64
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub start: u64,
    /// None = still held (live playing)
    pub end: Option<u64>,
    pub note: u8,
    pub vel: u8,
}

pub struct Sampler {
    pub sample: SampleData,
    pub notes: Vec<Note>,
    pub gain: f32,
    pub attack: u64,
    pub release: u64,
    pub max_voices: usize,
}

impl Sampler {
    pub fn new(sample: SampleData) -> Self {
        Sampler { sample, notes: Vec::new(), gain: 1.0, attack: 32, release: 441, max_voices: 16 }
    }

    /// Playback-speed ratio for `note` at a given project rate: 2^((note-root)/12) corrected for the sample's own rate.
    pub fn ratio(&self, note: u8, project_rate: u32) -> f64 {
        let semis = note as f64 - self.sample.root_note as f64 + self.sample.tune_cents as f64 / 100.0;
        2f64.powf(semis / 12.0) * self.sample.rate as f64 / project_rate as f64
    }

    /// Live: start a held note now.
    pub fn note_on(&mut self, frame: u64, note: u8, vel: u8) {
        self.notes.push(Note { start: frame, end: None, note, vel });
    }

    /// Live: release the most recent held instance of `note`.
    pub fn note_off(&mut self, frame: u64, note: u8) {
        if let Some(n) = self.notes.iter_mut().rev().find(|n| n.note == note && n.end.is_none()) {
            n.end = Some(frame.max(n.start));
        }
    }

    /// Drop notes that can no longer sound at or after `frame` (keeps a long live session cheap).
    pub fn prune(&mut self, frame: u64) {
        let rel = self.release;
        self.notes.retain(|n| n.end.is_none_or(|e| e + rel > frame));
    }
}

/// 4-point, 3rd-order Hermite interpolation between y1 (t=0) and y2 (t=1).
fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c0 = y1;
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + c0
}

impl Source for Sampler {
    fn render_add(&mut self, start: u64, out: &mut [f32], rate: u32) {
        let frames = (out.len() / 2) as u64;
        let end_chunk = start + frames;
        let len = self.sample.frames();
        if len < 2 {
            return;
        }
        // notes that can sound in this chunk
        let mut active: Vec<&Note> = self
            .notes
            .iter()
            .filter(|n| n.start < end_chunk && n.end.is_none_or(|e| e + self.release > start))
            .collect();
        if active.len() > self.max_voices {
            active.sort_by_key(|n| n.start); // steal the oldest
            active.drain(..active.len() - self.max_voices);
        }
        let audio = &self.sample.audio;
        for n in active {
            let ratio = self.ratio(n.note, rate);
            let vel = n.vel as f32 / 127.0;
            let from = start.max(n.start);
            let to = end_chunk.min(n.end.map_or(u64::MAX, |e| e + self.release));
            for f in from..to {
                let age = f - n.start;
                let mut pos = age as f64 * ratio;
                match self.sample.loop_region {
                    Some((ls, le)) if le > ls + 1 && le <= len => {
                        if pos >= ls as f64 {
                            pos = ls as f64 + (pos - ls as f64) % (le - ls) as f64;
                        }
                    }
                    _ => {
                        if pos >= (len - 1) as f64 {
                            break; // the sample has ended
                        }
                    }
                }
                let i = pos.floor() as i64;
                let t = (pos - i as f64) as f32;
                let at = |k: i64, c: usize| -> f32 {
                    let kk = k.clamp(0, len as i64 - 1) as usize;
                    audio[2 * kk + c]
                };
                let mut env = vel * self.gain;
                if age < self.attack {
                    env *= age as f32 / self.attack as f32;
                }
                if let Some(e) = n.end {
                    if f >= e {
                        env *= 1.0 - (f - e) as f32 / self.release.max(1) as f32;
                    }
                }
                let o = (f - start) as usize * 2;
                for c in 0..2 {
                    out[o + c] += hermite(at(i - 1, c), at(i, c), at(i + 1, c), at(i + 2, c), t) * env;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const R: u32 = 44100;

    fn sine_sample(freq: f64, secs: f64, rate: u32, root: u8) -> SampleData {
        let n = (secs * rate as f64) as usize;
        let audio: Vec<f32> = (0..n).flat_map(|i| {
            let v = (2.0 * std::f64::consts::PI * freq * i as f64 / rate as f64).sin() as f32 * 0.5;
            [v, v]
        }).collect();
        SampleData { audio: Arc::new(audio), rate, root_note: root, tune_cents: 0.0, loop_region: None }
    }

    /// frequency estimated from rising zero crossings of the left channel in `[from, to)`
    fn freq_of(buf: &[f32], from: usize, to: usize) -> f64 {
        let mut crossings = Vec::new();
        for i in from.max(1)..to {
            let (a, b) = (buf[2 * (i - 1)], buf[2 * i]);
            if a < 0.0 && b >= 0.0 {
                crossings.push(i as f64 - 1.0 + (-a / (b - a)) as f64); // sub-sample crossing
            }
        }
        assert!(crossings.len() > 4, "not enough cycles");
        (crossings.len() - 1) as f64 * R as f64 / (crossings[crossings.len() - 1] - crossings[0])
    }

    fn play(s: &mut Sampler, start: u64, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        s.render_add(start, &mut out, R);
        out
    }

    #[test]
    fn pitch_follows_the_keyboard_by_the_equal_tempered_ratio() {
        // A4 (69) sine at 440 Hz as the root
        for (note, want) in [(69u8, 440.0), (81, 880.0), (57, 220.0), (72, 523.2511), (60, 261.6256)] {
            let mut s = Sampler::new(sine_sample(440.0, 2.0, R, 69));
            s.note_on(0, note, 127);
            let b = play(&mut s, 0, 40000);
            let f = freq_of(&b, 2000, 38000);
            assert!((f - want).abs() / want < 0.002, "note {note}: {f} vs {want}");
        }
    }

    #[test]
    fn a_sample_at_a_different_rate_still_plays_at_its_root_pitch() {
        let mut s = Sampler::new(sine_sample(440.0, 2.0, 22050, 69));
        s.note_on(0, 69, 127);
        let b = play(&mut s, 0, 40000);
        assert!((freq_of(&b, 2000, 38000) - 440.0).abs() < 1.0);
        assert_eq!(s.ratio(69, R), 0.5);
    }

    #[test]
    fn fine_tune_in_cents() {
        let mut s = Sampler::new(sine_sample(440.0, 2.0, R, 69));
        s.sample.tune_cents = 100.0; // +1 semitone
        s.note_on(0, 69, 127);
        let b = play(&mut s, 0, 40000);
        assert!((freq_of(&b, 2000, 38000) - 466.1638).abs() < 1.0);
    }

    #[test]
    fn velocity_scales_level_attack_ramps_and_release_ends_the_sound() {
        let mut s = Sampler::new(sine_sample(440.0, 3.0, R, 69));
        s.note_on(1000, 69, 127);
        s.note_on(1000, 69, 64);
        s.notes[1].note = 70; // distinct note so both sound; compare levels below via separate samplers
        let mut loud = Sampler::new(sine_sample(440.0, 3.0, R, 69));
        loud.note_on(0, 69, 127);
        let mut soft = Sampler::new(sine_sample(440.0, 3.0, R, 69));
        soft.note_on(0, 69, 64);
        let (l, q) = (play(&mut loud, 0, 4000), play(&mut soft, 0, 4000));
        let pk = |b: &[f32]| b.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!((pk(&q) / pk(&l) - 64.0 / 127.0).abs() < 0.01);
        assert_eq!(l[0], 0.0, "attack starts from silence (no click)");
        assert!(l[2 * 5].abs() < l[2 * 400].abs().max(1e-6) + 0.5);
        // release: after note-off + release the output is exactly silent
        let mut r = Sampler::new(sine_sample(440.0, 3.0, R, 69));
        r.note_on(0, 69, 127);
        r.note_off(10000, 69);
        let b = play(&mut r, 0, 12000);
        assert!(b[2 * 9000].abs() > 0.0 || b[2 * 9001].abs() > 0.0);
        assert!(b[2 * 11000..].iter().all(|x| *x == 0.0), "silent after the release ends");
        assert_eq!(r.notes[0].end, Some(10000));
    }

    #[test]
    fn plays_once_without_a_loop_and_sustains_with_one() {
        let mut s = Sampler::new(sine_sample(440.0, 0.1, R, 69)); // 4410 frames
        s.note_on(0, 69, 127);
        let b = play(&mut s, 0, 20000);
        assert!(b[2 * 5000..].iter().all(|x| *x == 0.0), "ends with the sample");
        s.sample.loop_region = Some((1000, 3000));
        let b = play(&mut s, 0, 20000);
        assert!(b[2 * 19000..2 * 20000].iter().any(|x| x.abs() > 0.1), "keeps sounding while held");
    }

    #[test]
    fn output_is_bit_identical_for_any_chunk_size_and_seekable() {
        let mut s = Sampler::new(sine_sample(330.0, 1.0, R, 60));
        s.sample.loop_region = Some((500, 20000));
        for (st, en, n) in [(0u64, Some(9000u64), 60u8), (3000, Some(15000), 67), (5000, None, 48), (7000, Some(30000), 75)] {
            s.notes.push(Note { start: st, end: en, note: n, vel: 100 });
        }
        let whole = play(&mut s, 0, 30000);
        for chunk in [1usize, 64, 777, 4096] {
            let mut out = Vec::new();
            let mut pos = 0;
            while pos < 30000 {
                let n = chunk.min(30000 - pos);
                out.extend(play(&mut s, pos as u64, n));
                pos += n;
            }
            assert!(out == whole, "chunk {chunk}");
        }
        // seeking into the middle gives exactly the same samples as playing from the start
        assert_eq!(play(&mut s, 12345, 500), whole[2 * 12345..2 * 12845].to_vec());
    }

    #[test]
    fn polyphony_cap_steals_the_oldest_and_prune_drops_dead_notes() {
        let mut s = Sampler::new(sine_sample(440.0, 2.0, R, 69));
        s.max_voices = 2;
        for (i, n) in [60u8, 64, 67].iter().enumerate() {
            s.note_on(i as u64 * 10, *n, 100);
        }
        let capped = play(&mut s, 5000, 100);
        let mut two = Sampler::new(sine_sample(440.0, 2.0, R, 69));
        two.notes = s.notes[1..].to_vec();
        assert_eq!(capped, play(&mut two, 5000, 100), "oldest voice is the one dropped");
        s.note_off(100, 60);
        s.prune(100 + s.release + 1);
        assert_eq!(s.notes.len(), 2);
    }

    #[test]
    fn empty_or_tiny_samples_and_far_notes_do_not_panic() {
        let mut s = Sampler::new(SampleData { audio: Arc::new(vec![]), rate: R, root_note: 60, tune_cents: 0.0, loop_region: None });
        s.note_on(0, 60, 100);
        play(&mut s, 0, 100);
        let mut s = Sampler::new(sine_sample(440.0, 0.01, R, 69));
        for n in [0u8, 127] {
            s.note_on(0, n, 127);
        }
        play(&mut s, 0, 1000);
    }
}
