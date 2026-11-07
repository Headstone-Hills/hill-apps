//! Drum track: a chain of pattern sections rendered as a stateless mixer [`Source`].
//!
//! Every hit's time is a pure function of (section start, pattern cycle, step, tempo, swing), and every
//! sample is read at a position computed from the absolute output frame. Nothing is carried from one
//! `render_add` call to the next, so seeking, looping the transport and offline export are all correct
//! and the output is identical for any chunk size (tested).
//!
//! This module knows nothing about HillBeat presets or sample files: `pocketdaw-drums` turns presets into
//! [`DrumPattern`]s. The timing rules below mirror HillBeat so a pattern grooves the same way in both apps:
//! step = one `1/div` of a beat, odd steps are delayed by `swing * step`, a hit's gain is
//! `velocity/127 * voice volume * 0.9`, and a flam adds a grace note 30 ms early at 0.4 of the level.

use crate::mixer::Source;
use std::sync::Arc;

/// HillBeat has 8 drum voices; per-voice volume and mute live on the track and survive preset changes.
pub const VOICES: usize = 8;
/// Bit set in a cell for a flam. The low 7 bits are the velocity (0 = no hit).
pub const FLAM_BIT: u8 = 0x80;
pub const FLAM_GRACE_SECS: f64 = 0.030;
pub const FLAM_GRACE_LEVEL: f32 = 0.4;
/// Hit gain headroom, same constant as HillBeat.
pub const HIT_HEADROOM: f32 = 0.9;
pub const MAX_SWING: f32 = 0.25;
pub const MAX_VOICE_VOL: f32 = 4.0;
/// A hit sounds for at most this long. It bounds how far back `render_add` must look for ringing tails;
/// the cut applies to every chunk size alike, so it does not make output chunk dependent.
pub const MAX_HIT_SECS: f64 = 4.0;

/// One drum sound: interleaved stereo at its own sample rate (resampled on the fly).
#[derive(Clone)]
pub struct DrumVoice {
    pub data: Arc<Vec<f32>>,
    pub rate: u32,
}

impl DrumVoice {
    pub fn frames(&self) -> usize {
        self.data.len() / 2
    }
}

/// One loopable pattern with the sounds it was laid out for.
#[derive(Clone)]
pub struct DrumPattern {
    /// Steps per cycle (HillBeat: up to 16).
    pub steps: usize,
    /// Steps per beat: 4 = sixteenths, 3 = triplets / 12-8.
    pub div: u32,
    /// `cells[voice][step]`: velocity | [`FLAM_BIT`].
    pub cells: Vec<Vec<u8>>,
    /// `pitch[voice][step]`: semitones (toms that share one sound).
    pub pitch: Vec<Vec<i8>>,
    /// One entry per voice; `None` = silent voice.
    pub voices: Vec<Option<DrumVoice>>,
}

impl DrumPattern {
    pub fn silent(steps: usize, div: u32) -> Self {
        DrumPattern { steps, div, cells: vec![vec![0; steps]; VOICES], pitch: vec![vec![0; steps]; VOICES], voices: vec![None; VOICES] }
    }
}

/// A pattern repeated `cycles` times. For a 4/4 sixteenth pattern one cycle is one bar.
#[derive(Clone)]
pub struct DrumSection {
    pub name: String,
    pub pattern: Arc<DrumPattern>,
    pub cycles: u32,
}

pub struct DrumTrack {
    /// Sections play back to back starting at `start`.
    pub sections: Vec<DrumSection>,
    pub bpm: f64,
    /// Fraction of a step by which odd steps are delayed, 0..=0.25 (50%..75% swing).
    pub swing: f32,
    /// Timeline position of the first section (frames).
    pub start: u64,
    pub vol: [f32; VOICES],
    pub mute: [bool; VOICES],
    pub gain: f32,
}

impl DrumTrack {
    pub fn new(bpm: f64) -> Self {
        DrumTrack { sections: vec![], bpm, swing: 0.0, start: 0, vol: [1.0; VOICES], mute: [false; VOICES], gain: 1.0 }
    }

    fn step_len(&self, p: &DrumPattern, rate: u32) -> f64 {
        rate as f64 * 60.0 / self.bpm.max(1.0) / p.div.max(1) as f64
    }

    /// Total length of the chain in frames.
    pub fn total_frames(&self, rate: u32) -> u64 {
        self.sections.iter().map(|s| self.section_len(s, rate)).sum::<f64>().round() as u64
    }

    fn section_len(&self, s: &DrumSection, rate: u32) -> f64 {
        self.step_len(&s.pattern, rate) * s.pattern.steps as f64 * s.cycles as f64
    }

    /// `(section index, start frame)` pairs, for the arrangement view.
    pub fn section_starts(&self, rate: u32) -> Vec<(usize, u64)> {
        let mut at = self.start as f64;
        self.sections
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let r = (i, at.round() as u64);
                at += self.section_len(s, rate);
                r
            })
            .collect()
    }

    /// Mix one sound into `out` for the frames `[start, end)`. `hit` is the frame the sound begins at
    /// (may be negative: a grace note before frame 0).
    #[allow(clippy::too_many_arguments)]
    fn play(v: &DrumVoice, hit: i64, ratio: f64, gain: f32, start: u64, end: u64, rate: u32, out: &mut [f32]) {
        let nframes = v.frames();
        if nframes == 0 || gain == 0.0 {
            return;
        }
        let step = ratio * v.rate as f64 / rate as f64;
        let cap = (MAX_HIT_SECS * rate as f64) as i64;
        let len = (((nframes as f64) / step).ceil() as i64).min(cap);
        let from = (start as i64).max(hit);
        let to = (end as i64).min(hit + len);
        for f in from..to {
            let pos = (f - hit) as f64 * step;
            let i0 = pos as usize;
            if i0 >= nframes {
                break;
            }
            let i1 = (i0 + 1).min(nframes - 1);
            let fr = (pos - i0 as f64) as f32;
            let o = (f as u64 - start) as usize * 2;
            let d = &v.data;
            out[o] += (d[i0 * 2] + (d[i1 * 2] - d[i0 * 2]) * fr) * gain;
            out[o + 1] += (d[i0 * 2 + 1] + (d[i1 * 2 + 1] - d[i0 * 2 + 1]) * fr) * gain;
        }
    }
}

impl Source for DrumTrack {
    fn render_add(&mut self, start: u64, out: &mut [f32], rate: u32) {
        let end = start + (out.len() / 2) as u64;
        let lead = FLAM_GRACE_SECS * rate as f64;
        // a hit that began before `lo` has finished (its length is capped), one after `hi` has not begun
        let lo = start as f64 - MAX_HIT_SECS * rate as f64 - 1.0;
        let hi = end as f64 + lead + 1.0;
        let swing = self.swing.clamp(0.0, MAX_SWING) as f64;
        let mut at = self.start as f64;
        for sec in &self.sections {
            let p = &*sec.pattern;
            let sl = self.step_len(p, rate);
            let cyc = sl * p.steps as f64;
            let s0 = at;
            at += cyc * sec.cycles as f64;
            if at < lo {
                continue;
            }
            if s0 > hi {
                break; // sections only get later
            }
            if p.steps == 0 || sec.cycles == 0 {
                continue;
            }
            let c_lo = (((lo - s0) / cyc).floor().max(0.0)) as u32;
            let c_hi = ((((hi - s0) / cyc).floor().max(0.0)) as u32).min(sec.cycles - 1);
            for c in c_lo..=c_hi {
                for s in 0..p.steps {
                    let t = s0 + c as f64 * cyc + s as f64 * sl + if s % 2 == 1 { swing * sl } else { 0.0 };
                    let hit = t.round() as i64;
                    for vi in 0..VOICES.min(p.cells.len()) {
                        let cell = p.cells[vi].get(s).copied().unwrap_or(0);
                        let vel = cell & !FLAM_BIT;
                        let Some(Some(voice)) = p.voices.get(vi) else { continue };
                        if vel == 0 || self.mute[vi] {
                            continue;
                        }
                        let semis = p.pitch.get(vi).and_then(|r| r.get(s)).copied().unwrap_or(0);
                        let ratio = 2f64.powf(semis as f64 / 12.0);
                        let g = vel as f32 / 127.0 * self.vol[vi].clamp(0.0, MAX_VOICE_VOL) * HIT_HEADROOM * self.gain;
                        Self::play(voice, hit, ratio, g, start, end, rate, out);
                        if cell & FLAM_BIT != 0 {
                            Self::play(voice, hit - lead.round() as i64, ratio, g * FLAM_GRACE_LEVEL, start, end, rate, out);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = 44100;

    /// A 100-frame constant-1.0 sound: easy to see exactly where it lands.
    fn blip() -> DrumVoice {
        DrumVoice { data: Arc::new(vec![1.0; 200]), rate: R }
    }

    fn one_hit_pattern(steps: usize, div: u32, step: usize, cell: u8) -> DrumPattern {
        let mut p = DrumPattern::silent(steps, div);
        p.cells[0][step] = cell;
        p.voices[0] = Some(blip());
        p
    }

    fn track(p: DrumPattern, cycles: u32) -> DrumTrack {
        let mut t = DrumTrack::new(120.0);
        t.sections.push(DrumSection { name: "A".into(), pattern: Arc::new(p), cycles });
        t
    }

    fn render(t: &mut DrumTrack, from: u64, frames: usize) -> Vec<f32> {
        let mut o = vec![0.0; frames * 2];
        t.render_add(from, &mut o, R);
        o
    }

    /// First frame whose left sample is non-zero.
    fn first_on(o: &[f32]) -> Option<usize> {
        o.chunks_exact(2).position(|f| f[0] != 0.0)
    }

    #[test]
    fn hit_lands_on_its_step_with_the_hillbeat_gain() {
        // 120 bpm sixteenths: step = 5512.5 frames; step 4 = 22050 exactly
        let mut t = track(one_hit_pattern(16, 4, 4, 127), 1);
        let o = render(&mut t, 0, 30000);
        assert_eq!(first_on(&o), Some(22050));
        assert!((o[22050 * 2] - 0.9).abs() < 1e-6, "vel 127, vol 1 -> 0.9");
        assert_eq!(o[(22050 + 99) * 2], 0.9);
        assert_eq!(o[(22050 + 100) * 2], 0.0, "100-frame sound ends");
    }

    #[test]
    fn velocity_and_voice_volume_scale_the_gain() {
        let mut t = track(one_hit_pattern(16, 4, 0, 64), 1);
        t.vol[0] = 2.0;
        let o = render(&mut t, 0, 10);
        assert!((o[0] - 64.0 / 127.0 * 2.0 * 0.9).abs() < 1e-6);
        t.vol[0] = 99.0; // clamped to the 400% HillBeat allows
        let o = render(&mut t, 0, 10);
        assert!((o[0] - 64.0 / 127.0 * 4.0 * 0.9).abs() < 1e-6);
    }

    #[test]
    fn mute_silences_one_voice_only() {
        let mut p = one_hit_pattern(16, 4, 0, 127);
        p.cells[1][0] = 127;
        p.voices[1] = Some(blip());
        let mut t = track(p, 1);
        t.mute[0] = true;
        let o = render(&mut t, 0, 10);
        assert!((o[0] - 0.9).abs() < 1e-6, "voice 1 still sounds");
        t.mute[1] = true;
        assert_eq!(render(&mut t, 0, 10)[0], 0.0);
    }

    #[test]
    fn swing_delays_odd_steps_only() {
        let mut p = DrumPattern::silent(16, 4);
        p.voices[0] = Some(blip());
        p.cells[0][1] = 100;
        p.cells[0][2] = 100;
        let mut t = track(p, 1);
        t.swing = 0.2;
        let o = render(&mut t, 0, 20000);
        // step 1 = 5512.5 + 0.2*5512.5 = 6615 ; step 2 = 11025 (even: not delayed)
        assert_eq!(first_on(&o), Some(6615));
        assert!(o[11025 * 2] != 0.0 && o[11024 * 2] == 0.0);
    }

    #[test]
    fn triplet_division_changes_the_step_length() {
        // div 3 at 120 bpm: step = 22050/3 = 7350 frames
        let mut t = track(one_hit_pattern(12, 3, 1, 100), 1);
        assert_eq!(first_on(&render(&mut t, 0, 10000)), Some(7350));
    }

    #[test]
    fn flam_adds_a_quiet_grace_note_30ms_early() {
        let mut t = track(one_hit_pattern(16, 4, 4, 100 | FLAM_BIT), 1);
        let o = render(&mut t, 0, 30000);
        let lead = (0.030 * R as f64).round() as usize; // 1323
        let g = 100.0 / 127.0 * 0.9;
        assert_eq!(first_on(&o), Some(22050 - lead));
        assert!((o[(22050 - lead) * 2] - g * 0.4).abs() < 1e-6);
        assert!((o[22050 * 2] - g).abs() < 1e-6, "the main hit is still full level");
    }

    #[test]
    fn a_flam_on_the_first_step_does_not_underflow_frame_zero() {
        let mut t = track(one_hit_pattern(16, 4, 0, 100 | FLAM_BIT), 1);
        let o = render(&mut t, 0, 3000);
        // grace note began 1323 frames before 0: frames 0..(100-1323<0) are already over, main hit at 0 only
        assert!(o[0] > 0.0);
        assert_eq!(o[200], 0.0, "hit is 100 frames long");
    }

    #[test]
    fn pitch_shift_resamples_by_semitones() {
        let mut p = one_hit_pattern(16, 4, 0, 127);
        p.pitch[0][0] = 12; // an octave up: twice as fast, half as long
        let mut t = track(p, 1);
        let o = render(&mut t, 0, 200);
        assert_eq!(o.chunks_exact(2).filter(|f| f[0] != 0.0).count(), 50);
    }

    #[test]
    fn sections_chain_back_to_back_and_cycles_repeat() {
        // 120 bpm, 4 steps of sixteenths: a cycle is 22050 frames
        let a = one_hit_pattern(4, 4, 0, 100);
        let mut b = DrumPattern::silent(4, 4);
        b.voices[0] = Some(blip());
        b.cells[0][2] = 100;
        let mut t = DrumTrack::new(120.0);
        t.sections.push(DrumSection { name: "A".into(), pattern: Arc::new(a), cycles: 2 });
        t.sections.push(DrumSection { name: "B".into(), pattern: Arc::new(b), cycles: 1 });
        let o = render(&mut t, 0, 80000);
        let hits: Vec<usize> = (0..o.len() / 2).filter(|&i| o[i * 2] != 0.0 && (i == 0 || o[(i - 1) * 2] == 0.0)).collect();
        // A: 0 and 11025 (cycle = 4 steps * 5512.5 = 22050 ... see below), B step 2 after both A cycles
        let cyc: f64 = 4.0 * 5512.5;
        let expect = [0usize, cyc.round() as usize, (2.0 * cyc + 2.0 * 5512.5).round() as usize];
        assert_eq!(hits, expect);
        assert_eq!(t.total_frames(R), (3.0 * cyc).round() as u64);
        assert_eq!(t.section_starts(R), vec![(0, 0), (1, (2.0 * cyc) as u64)]);
        assert_eq!(render(&mut t, 3 * cyc as u64 + 5000, 2000).iter().sum::<f32>(), 0.0, "nothing after the chain ends");
    }

    #[test]
    fn start_offset_moves_the_whole_chain() {
        let mut t = track(one_hit_pattern(16, 4, 0, 100), 1);
        t.start = 1000;
        assert_eq!(first_on(&render(&mut t, 0, 3000)), Some(1000));
    }

    #[test]
    fn a_long_tail_keeps_ringing_across_a_chunk_boundary() {
        let mut p = DrumPattern::silent(16, 4);
        p.voices[0] = Some(DrumVoice { data: Arc::new(vec![0.5; 2 * 30000]), rate: R });
        p.cells[0][0] = 127;
        let mut t = track(p, 1);
        let o = render(&mut t, 25000, 1000); // starts long after the hit began
        assert!(o.iter().all(|&x| x > 0.0), "still ringing 25000 frames in");
        assert_eq!(render(&mut t, 30000, 100).iter().sum::<f32>(), 0.0, "sound is 30000 frames long");
    }

    #[test]
    fn output_is_identical_for_any_chunk_size() {
        let mut p = DrumPattern::silent(16, 4);
        for v in 0..3 {
            p.voices[v] = Some(DrumVoice { data: Arc::new((0..4000).map(|i| ((i * (v + 3)) as f32 * 0.01).sin()).collect()), rate: 22050 });
            for s in (v..16).step_by(3) {
                p.cells[v][s] = 90 | if s % 5 == 0 { FLAM_BIT } else { 0 };
                p.pitch[v][s] = (s as i8 % 7) - 3;
            }
        }
        let p = Arc::new(p);
        let mk = || {
            let mut t = DrumTrack::new(133.0);
            t.swing = 0.17;
            t.sections.push(DrumSection { name: "A".into(), pattern: p.clone(), cycles: 2 });
            t.sections.push(DrumSection { name: "B".into(), pattern: p.clone(), cycles: 1 });
            t
        };
        let total = 140000;
        let whole = render(&mut mk(), 0, total);
        assert!(whole.iter().any(|&x| x != 0.0));
        for chunk in [1usize, 7, 441, 1024, 9999] {
            let mut t = mk();
            let mut got = Vec::with_capacity(whole.len());
            let mut at = 0;
            while at < total {
                let n = chunk.min(total - at);
                got.extend(render(&mut t, at as u64, n));
                at += n;
            }
            assert!(got == whole, "chunk size {chunk} changed the output");
        }
    }

    #[test]
    fn sections_with_zero_cycles_or_steps_are_harmless() {
        let mut t = DrumTrack::new(120.0);
        t.sections.push(DrumSection { name: "z".into(), pattern: Arc::new(DrumPattern::silent(0, 4)), cycles: 3 });
        t.sections.push(DrumSection { name: "z".into(), pattern: Arc::new(one_hit_pattern(16, 4, 0, 100)), cycles: 0 });
        assert_eq!(render(&mut t, 0, 100).iter().sum::<f32>(), 0.0);
    }
}
