//! Backing-loop player. The loop's tempo is known (entered once, parsed from
//! the file name, or derived from a bar count) and it is resampled by
//! `app_bpm / loop_bpm`, so it follows the sequencer tempo (varispeed: pitch
//! moves with tempo).

use crate::wav::Sample;
use std::sync::Arc;

pub struct LoopPlayer {
    pub sample: Option<Arc<Sample>>,
    pub file: String,
    pub bpm: f32,
    pub on: bool,
    pos: f64,
}

impl Default for LoopPlayer {
    fn default() -> Self {
        LoopPlayer { sample: None, file: String::new(), bpm: 120.0, on: false, pos: 0.0 }
    }
}

impl LoopPlayer {
    pub fn set(&mut self, file: &str, s: Arc<Sample>, bpm: f32) {
        self.file = file.to_string();
        self.sample = Some(s);
        self.bpm = bpm.clamp(20.0, 400.0);
        self.pos = 0.0;
    }

    pub fn clear(&mut self) {
        *self = LoopPlayer::default();
    }

    pub fn rewind(&mut self) {
        self.pos = 0.0;
    }

    /// Playback ratio for a given app tempo.
    pub fn ratio(&self, app_bpm: f32) -> f64 {
        app_bpm as f64 / self.bpm as f64
    }

    /// Add `out.len()/2` frames of loop audio. Call only while the transport
    /// is playing; position is held otherwise.
    pub fn render(&mut self, out: &mut [f32], app_bpm: f32, out_rate: f32) {
        if !self.on {
            return;
        }
        let Some(s) = self.sample.as_ref() else { return };
        let n = s.frames();
        if n < 2 {
            return;
        }
        let step = self.ratio(app_bpm) * s.rate as f64 / out_rate as f64;
        let gain = 0.7;
        for f in 0..out.len() / 2 {
            let i = self.pos as usize;
            let j = if i + 1 >= n { 0 } else { i + 1 };
            let fr = (self.pos - i as f64) as f32;
            let (l0, r0) = s.frame(i);
            let (l1, r1) = s.frame(j);
            let l = l0 + (l1 - l0) * fr;
            let r = r0 + (r1 - r0) * fr;
            out[f * 2] += l * gain;
            out[f * 2 + 1] += r * gain;
            self.pos += step;
            if self.pos >= n as f64 {
                self.pos -= n as f64;
            }
        }
    }
}

/// Tempo token in a file name: `120bpm`, `120 bpm`, `bpm120`, `bpm_120`.
pub fn parse_bpm(name: &str) -> Option<f32> {
    let l = name.to_lowercase();
    let b = l.as_bytes();
    let digits_before = |end: usize| -> Option<f32> {
        let mut s = end;
        while s > 0 && (b[s - 1].is_ascii_digit() || b[s - 1] == b'.') {
            s -= 1;
        }
        let mut e = end;
        // allow a single separator between number and "bpm"
        if s == end && e > 0 && matches!(b[e - 1], b'_' | b'-' | b' ') {
            e -= 1;
            s = e;
            while s > 0 && (b[s - 1].is_ascii_digit() || b[s - 1] == b'.') {
                s -= 1;
            }
        }
        l.get(s..e)?.parse().ok().filter(|v: &f32| (30.0..=300.0).contains(v))
    };
    let digits_after = |start: usize| -> Option<f32> {
        let mut s = start;
        if s < b.len() && matches!(b[s], b'_' | b'-' | b' ') {
            s += 1;
        }
        let mut e = s;
        while e < b.len() && (b[e].is_ascii_digit() || b[e] == b'.') {
            e += 1;
        }
        l.get(s..e)?.parse().ok().filter(|v: &f32| (30.0..=300.0).contains(v))
    };
    let mut from = 0;
    while let Some(i) = l[from..].find("bpm") {
        let at = from + i;
        if let Some(v) = digits_before(at).or_else(|| digits_after(at + 3)) {
            return Some(v);
        }
        from = at + 3;
    }
    None
}

/// BPM implied by a loop that is `bars` bars of 4/4 long.
pub fn bpm_from_bars(bars: u32, seconds: f32) -> f32 {
    if seconds <= 0.0 {
        return 120.0;
    }
    bars as f32 * 4.0 * 60.0 / seconds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bpm_tokens() {
        assert_eq!(parse_bpm("Funk_Loop_96bpm.wav"), Some(96.0));
        assert_eq!(parse_bpm("drums 120 BPM.wav"), Some(120.0));
        assert_eq!(parse_bpm("loop_bpm_88.wav"), Some(88.0));
        assert_eq!(parse_bpm("pad_C.wav"), None);
        assert_eq!(parse_bpm("x_9999bpm.wav"), None);
    }

    #[test]
    fn bars_to_bpm() {
        assert!((bpm_from_bars(4, 8.0) - 120.0).abs() < 1e-4);
        assert!((bpm_from_bars(2, 5.0) - 96.0).abs() < 1e-4);
    }

    #[test]
    fn varispeed_ratio_and_wrap() {
        let s = Arc::new(Sample::new((0..2000).map(|i| (i / 2) as f32 / 1000.0).collect(), 44100));
        let mut lp = LoopPlayer::default();
        lp.set("x_100bpm.wav", s, 100.0);
        lp.on = true;
        assert!((lp.ratio(120.0) - 1.2).abs() < 1e-9);
        let mut out = vec![0.0; 2 * 3000];
        lp.render(&mut out, 100.0, 44100.0);
        assert!(out.iter().any(|x| *x > 0.0));
        // wrapped without panicking and kept position in range
        assert!(lp.pos < 1000.0);
    }

    #[test]
    fn off_renders_nothing() {
        let s = Arc::new(Sample::new(vec![1.0; 200], 44100));
        let mut lp = LoopPlayer::default();
        lp.set("a", s, 100.0);
        let mut out = vec![0.0; 100];
        lp.render(&mut out, 100.0, 44100.0);
        assert!(out.iter().all(|x| *x == 0.0));
    }
}
