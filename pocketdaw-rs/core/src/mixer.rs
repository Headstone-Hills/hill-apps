//! Multitrack mixer. Everything renders into interleaved stereo `f32` buffers.
//!
//! Sources are positioned by an absolute frame number, so output never depends on how the host
//! chunks the work. Per track: gain, pan (standard -3 dB pan law), mute/solo. Then master gain and a
//! brickwall limiter whose state is carried sample by sample (so it is chunk-size independent too).

use std::f32::consts::PI;
use std::sync::Arc;

/// Adds its audio for absolute frames `[start, start + out.len()/2)` into `out` (interleaved L,R).
pub trait Source: Send {
    fn render_add(&mut self, start: u64, out: &mut [f32], rate: u32);
}

/// Continuous sine, phase locked to the absolute frame (no clicks across chunk boundaries).
pub struct Tone {
    pub freq: f64,
    pub amp: f32,
}

impl Source for Tone {
    fn render_add(&mut self, start: u64, out: &mut [f32], rate: u32) {
        for (i, f) in out.chunks_exact_mut(2).enumerate() {
            let t = (start + i as u64) as f64 / rate as f64;
            let v = self.amp * (2.0 * std::f64::consts::PI * self.freq * t).sin() as f32;
            f[0] += v;
            f[1] += v;
        }
    }
}

/// Metronome: a short decaying sine on every beat, higher and louder on beat 1 of the bar.
pub struct Click {
    pub bpm: f64,
    pub beats_per_bar: u64,
    pub amp: f32,
    pub enabled: bool,
}

impl Source for Click {
    fn render_add(&mut self, start: u64, out: &mut [f32], rate: u32) {
        if !self.enabled {
            return;
        }
        let spb = rate as f64 * 60.0 / self.bpm;
        let click_len = (0.03 * rate as f64) as u64;
        for (i, f) in out.chunks_exact_mut(2).enumerate() {
            let n = start + i as u64;
            let mut b = (n as f64 / spb).floor() as u64;
            let mut s = (b as f64 * spb).round() as u64;
            if s > n {
                b = b.saturating_sub(1); // rounding put this beat's start just after n
                s = (b as f64 * spb).round() as u64;
            }
            let k = n - s;
            if k >= click_len {
                continue;
            }
            let accent = b.is_multiple_of(self.beats_per_bar.max(1));
            let (freq, level) = if accent { (1500.0, 1.0) } else { (1000.0, 0.6) };
            let env = (-(k as f64) / (0.006 * rate as f64)).exp();
            let v = self.amp * level * (env * (2.0 * std::f64::consts::PI * freq * k as f64 / rate as f64).sin()) as f32;
            f[0] += v;
            f[1] += v;
        }
    }
}

/// A block of interleaved stereo audio placed on the timeline at `offset` frames (optionally looped).
pub struct Clip {
    pub data: Arc<Vec<f32>>,
    pub offset: u64,
    pub looped: bool,
    pub gain: f32,
}

impl Source for Clip {
    fn render_add(&mut self, start: u64, out: &mut [f32], _rate: u32) {
        let len = (self.data.len() / 2) as u64;
        if len == 0 {
            return;
        }
        for (i, f) in out.chunks_exact_mut(2).enumerate() {
            let n = start + i as u64;
            if n < self.offset {
                continue;
            }
            let mut p = n - self.offset;
            if p >= len {
                if !self.looped {
                    break; // frames only increase within a chunk, so the rest is past the end too
                }
                p %= len;
            }
            f[0] += self.data[2 * p as usize] * self.gain;
            f[1] += self.data[2 * p as usize + 1] * self.gain;
        }
    }
}

pub struct Track {
    pub name: String,
    pub gain: f32,
    /// -1.0 (left) .. 1.0 (right)
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub sources: Vec<Box<dyn Source>>,
    /// peak of the last rendered chunk, after gain and pan (for meters)
    pub peak: f32,
}

impl Track {
    pub fn new(name: &str) -> Self {
        Track { name: name.into(), gain: 1.0, pan: 0.0, mute: false, solo: false, sources: Vec::new(), peak: 0.0 }
    }
}

/// Standard -3 dB pan law: centre is 0.707 on each side, hard left/right is 1.0 on one side.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
    (a.cos(), a.sin())
}

pub struct Limiter {
    pub ceiling: f32,
    pub release: f32, // per-sample smoothing coefficient
    gain: f32,
}

impl Limiter {
    pub fn new(ceiling: f32, release_s: f32, rate: u32) -> Self {
        Limiter { ceiling, release: 1.0 - (-1.0 / (release_s * rate as f32)).exp(), gain: 1.0 }
    }
    /// Instant attack, smooth release; the output never exceeds `ceiling`.
    pub fn process(&mut self, f: &mut [f32]) {
        for s in f.chunks_exact_mut(2) {
            let peak = s[0].abs().max(s[1].abs());
            let want = if peak > self.ceiling { self.ceiling / peak } else { 1.0 };
            if want < self.gain {
                self.gain = want;
            } else {
                self.gain += (want - self.gain) * self.release;
            }
            s[0] *= self.gain;
            s[1] *= self.gain;
        }
    }
    pub fn current_gain(&self) -> f32 {
        self.gain
    }
}

pub struct Mixer {
    pub rate: u32,
    pub tracks: Vec<Track>,
    pub master: f32,
    pub limiter: Limiter,
    pub master_peak: f32,
    scratch: Vec<f32>,
}

impl Mixer {
    pub fn new(rate: u32, max_frames: usize) -> Self {
        Mixer { rate, tracks: Vec::new(), master: 1.0, limiter: Limiter::new(0.98, 0.05, rate), master_peak: 0.0, scratch: vec![0.0; max_frames * 2] }
    }

    /// Solo overrides mute: if any track is soloed only soloed tracks sound.
    pub fn audible(&self, i: usize) -> bool {
        if self.tracks.iter().any(|t| t.solo) {
            self.tracks[i].solo
        } else {
            !self.tracks[i].mute
        }
    }

    /// Render `out.len()/2` frames starting at absolute frame `start`. `out` is overwritten.
    pub fn render(&mut self, start: u64, out: &mut [f32]) {
        out.fill(0.0);
        if self.scratch.len() < out.len() {
            self.scratch.resize(out.len(), 0.0); // only the first oversized call allocates
        }
        let any_solo = self.tracks.iter().any(|t| t.solo);
        let rate = self.rate;
        for t in self.tracks.iter_mut() {
            let sounds = if any_solo { t.solo } else { !t.mute };
            t.peak = 0.0;
            if !sounds {
                continue;
            }
            let sc = &mut self.scratch[..out.len()];
            sc.fill(0.0);
            for s in t.sources.iter_mut() {
                s.render_add(start, sc, rate);
            }
            let (gl, gr) = pan_gains(t.pan);
            let (gl, gr) = (gl * t.gain, gr * t.gain);
            let mut peak = 0.0f32;
            for (o, i) in out.chunks_exact_mut(2).zip(sc.chunks_exact(2)) {
                let (l, r) = (i[0] * gl, i[1] * gr);
                o[0] += l;
                o[1] += r;
                peak = peak.max(l.abs()).max(r.abs());
            }
            t.peak = peak;
        }
        for s in out.iter_mut() {
            *s *= self.master;
        }
        self.limiter.process(out);
        self.master_peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    }
}

/// Float -> interleaved little-endian i16 (what the SP's sound card path takes).
pub fn to_i16(out: &[f32], dst: &mut [i16]) {
    for (d, s) in dst.iter_mut().zip(out) {
        *d = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44100;

    fn song() -> Mixer {
        let mut m = Mixer::new(RATE, 4096);
        let mut a = Track::new("click");
        a.sources.push(Box::new(Click { bpm: 120.0, beats_per_bar: 4, amp: 0.5, enabled: true }));
        let mut b = Track::new("tone");
        b.sources.push(Box::new(Tone { freq: 333.0, amp: 0.3 }));
        b.pan = -0.4;
        let mut c = Track::new("clip");
        c.gain = 0.7;
        c.sources.push(Box::new(Clip { data: Arc::new((0..2000).map(|i| ((i as f32) * 0.01).sin() * 0.4).collect()), offset: 5000, looped: true, gain: 1.0 }));
        m.tracks = vec![a, b, c];
        m.master = 1.2; // push the limiter a little
        m
    }

    fn render_chunked(chunk: usize, total: usize) -> Vec<f32> {
        let mut m = song();
        let mut out = Vec::new();
        let mut buf = vec![0.0f32; chunk * 2];
        let mut pos = 0;
        while pos < total {
            let n = chunk.min(total - pos);
            m.render(pos as u64, &mut buf[..n * 2]);
            out.extend_from_slice(&buf[..n * 2]);
            pos += n;
        }
        out
    }

    #[test]
    fn output_is_bit_identical_for_any_chunk_size() {
        let a = render_chunked(1024, RATE as usize * 2);
        let b = render_chunked(777, RATE as usize * 2);
        let c = render_chunked(4096, RATE as usize * 2);
        let d = render_chunked(1, 3000); // single-frame chunks, shorter
        assert!(a == b, "1024 vs 777");
        assert!(a == c, "1024 vs 4096");
        assert!(a[..6000] == d[..], "1024 vs 1");
    }

    #[test]
    fn click_starts_on_the_beat_to_the_sample() {
        let mut m = Mixer::new(RATE, 1024);
        let mut t = Track::new("c");
        t.sources.push(Box::new(Click { bpm: 120.0, beats_per_bar: 4, amp: 0.8, enabled: true }));
        t.pan = -1.0; // hard left: left channel carries full level
        m.tracks.push(t);
        let mut out = vec![0.0f32; RATE as usize * 2 * 2];
        m.render(0, &mut out);
        let beat = 22050usize;
        // a click is a ~30 ms burst that crosses the threshold many times: take the first loud frame of each burst
        let loud: Vec<usize> = (0..out.len() / 2).filter(|&i| out[2 * i].abs() > 0.05).collect();
        let starts: Vec<usize> = loud.iter().enumerate().filter(|&(k, &i)| k == 0 || i - loud[k - 1] > 1000).map(|(_, &i)| i).collect();
        assert!(starts.len() >= 4, "{starts:?}");
        for (k, s) in starts.iter().take(4).enumerate() {
            let d = *s as i64 - (k * beat) as i64;
            assert!(d == 0 || d == 1, "click {k} at {s}"); // a sine's first sample is exactly 0
        }
    }

    #[test]
    fn clips_stay_aligned_to_the_timeline() {
        let mut m = Mixer::new(RATE, 64);
        m.limiter.ceiling = 10.0;
        for off in [3000u64, 3000 + RATE as u64] {
            let mut t = Track::new("c");
            t.pan = 0.0;
            t.sources.push(Box::new(Clip { data: Arc::new(vec![0.5, 0.5]), offset: off, looped: false, gain: 1.0 }));
            m.tracks.push(t);
        }
        let mut out = vec![0.0f32; (RATE as usize + 6000) * 2]; // frames 0..RATE+6000 cover both clips
        let frames = out.len() / 2;
        m.render(0, &mut out);
        let hits: Vec<usize> = (0..frames).filter(|&i| out[2 * i] != 0.0).collect();
        assert_eq!(hits, vec![3000, 3000 + RATE as usize]);
    }

    #[test]
    fn pan_law_mute_solo_and_gain() {
        let (l, r) = pan_gains(0.0);
        assert!((l - 0.70710677).abs() < 1e-6 && (r - 0.70710677).abs() < 1e-6);
        assert!(pan_gains(-1.0).1.abs() < 1e-6);
        assert!((pan_gains(1.0).1 - 1.0).abs() < 1e-6);
        let mk = || {
            let mut m = Mixer::new(RATE, 64);
            m.limiter.ceiling = 10.0;
            for _ in 0..2 {
                let mut t = Track::new("t");
                t.sources.push(Box::new(Clip { data: Arc::new(vec![0.25; 200]), offset: 0, looped: true, gain: 1.0 }));
                m.tracks.push(t);
            }
            m
        };
        let mut m = mk();
        let mut o = vec![0.0f32; 20];
        m.render(0, &mut o);
        assert!((o[0] - 0.25 * 0.70710677 * 2.0).abs() < 1e-5); // two tracks at centre
        m.tracks[0].mute = true;
        m.render(0, &mut o);
        assert!((o[0] - 0.25 * 0.70710677).abs() < 1e-5);
        m.tracks[1].solo = true;
        m.tracks[0].mute = false;
        assert!(!m.audible(0) && m.audible(1));
        m.tracks[0].solo = true; // both soloed
        assert!(m.audible(0));
        m.tracks[0].solo = false;
        m.tracks[1].solo = false;
        m.tracks[1].gain = 0.0;
        m.render(0, &mut o);
        assert!((o[0] - 0.25 * 0.70710677).abs() < 1e-5);
    }

    #[test]
    fn limiter_never_exceeds_the_ceiling_and_recovers() {
        let mut lim = Limiter::new(0.9, 0.05, RATE);
        let mut hot: Vec<f32> = (0..4000).map(|i| 3.0 * (i as f32 * 0.05).sin()).collect();
        lim.process(&mut hot);
        assert!(hot.iter().all(|s| s.abs() <= 0.9 + 1e-6));
        let mut quiet = vec![0.1f32; 2 * RATE as usize];
        lim.process(&mut quiet);
        assert!((quiet.last().unwrap() - 0.1).abs() < 1e-3, "gain recovers to unity after the peak");
    }

    #[test]
    fn meters_report_per_track_and_master_peaks() {
        let mut m = Mixer::new(RATE, 64);
        let mut t = Track::new("t");
        t.pan = -1.0;
        t.gain = 0.5;
        t.sources.push(Box::new(Clip { data: Arc::new(vec![0.8; 128]), offset: 0, looped: true, gain: 1.0 }));
        m.tracks.push(t);
        let mut o = vec![0.0f32; 64];
        m.render(0, &mut o);
        assert!((m.tracks[0].peak - 0.4).abs() < 1e-5);
        assert!((m.master_peak - 0.4).abs() < 1e-5);
    }

    #[test]
    fn i16_conversion_clamps() {
        let mut d = [0i16; 3];
        to_i16(&[0.5, 2.0, -2.0], &mut d);
        assert_eq!(d, [16383, 32767, -32767]);
    }
}
