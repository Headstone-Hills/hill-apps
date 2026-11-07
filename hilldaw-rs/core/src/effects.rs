//! Cheap per-track effects: low-pass / high-pass filter, delay, and a shared send reverb.
//! All state advances sample by sample, so output never depends on how the host chunks the audio.
//! Buffers are interleaved stereo f32.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FilterKind {
    LowPass,
    HighPass,
}

/// RBJ biquad (transposed direct form II), independent state per channel.
#[derive(Clone, Debug)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z: [[f64; 2]; 2], // [channel][z1, z2]
}

impl Biquad {
    pub fn new(kind: FilterKind, cutoff_hz: f64, q: f64, rate: u32) -> Self {
        let f = cutoff_hz.clamp(10.0, rate as f64 * 0.45);
        let w0 = 2.0 * PI * f / rate as f64;
        let (s, c) = (w0.sin(), w0.cos());
        let alpha = s / (2.0 * q.clamp(0.1, 20.0));
        let (b0, b1, b2) = match kind {
            FilterKind::LowPass => ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0),
            FilterKind::HighPass => ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0),
        };
        let a0 = 1.0 + alpha;
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: -2.0 * c / a0, a2: (1.0 - alpha) / a0, z: [[0.0; 2]; 2] }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for fr in buf.chunks_exact_mut(2) {
            for (c, x) in fr.iter_mut().enumerate() {
                let i = *x as f64;
                let o = self.b0 * i + self.z[c][0];
                self.z[c][0] = self.b1 * i - self.a1 * o + self.z[c][1];
                self.z[c][1] = self.b2 * i - self.a2 * o;
                *x = o as f32;
            }
        }
    }
}

/// Stereo delay with feedback. `mix` is added to the dry signal.
#[derive(Clone, Debug)]
pub struct Delay {
    buf: Vec<f32>,
    pos: usize,
    delay_frames: usize,
    pub feedback: f32,
    pub mix: f32,
}

impl Delay {
    pub fn new(max_ms: f64, rate: u32) -> Self {
        let max_frames = ((max_ms * rate as f64 / 1000.0) as usize).max(2);
        Delay { buf: vec![0.0; (max_frames + 1) * 2], pos: 0, delay_frames: max_frames / 2, feedback: 0.35, mix: 0.3 }
    }
    pub fn set_time_frames(&mut self, frames: usize) {
        self.delay_frames = frames.clamp(1, self.buf.len() / 2 - 1);
    }
    pub fn process(&mut self, io: &mut [f32]) {
        let len = self.buf.len() / 2;
        for fr in io.chunks_exact_mut(2) {
            let rd = (self.pos + len - self.delay_frames) % len;
            for (c, x) in fr.iter_mut().enumerate() {
                let d = self.buf[rd * 2 + c];
                let d = if d.abs() < 1e-20 { 0.0 } else { d }; // flush denormals in the feedback loop
                self.buf[self.pos * 2 + c] = *x + self.feedback * d;
                *x += self.mix * d;
            }
            self.pos = (self.pos + 1) % len;
        }
    }
}

struct Comb {
    buf: Vec<f32>,
    i: usize,
    store: f32,
}
impl Comb {
    fn new(n: usize) -> Self {
        Comb { buf: vec![0.0; n.max(1)], i: 0, store: 0.0 }
    }
    fn tick(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.i];
        self.store = out * (1.0 - damp) + self.store * damp;
        if self.store.abs() < 1e-20 {
            self.store = 0.0;
        }
        self.buf[self.i] = x + self.store * feedback;
        self.i = (self.i + 1) % self.buf.len();
        out
    }
}
struct Allpass {
    buf: Vec<f32>,
    i: usize,
}
impl Allpass {
    fn new(n: usize) -> Self {
        Allpass { buf: vec![0.0; n.max(1)], i: 0 }
    }
    fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.i];
        let out = b - x;
        self.buf[self.i] = x + b * 0.5;
        self.i = (self.i + 1) % self.buf.len();
        out
    }
}

/// Small Schroeder/Freeverb-style reverb (8 combs + 4 allpasses per side). Returns the WET signal only,
/// so it is used as a shared send: tracks feed a bus, the bus goes through one reverb, the result is added back.
pub struct Reverb {
    combs: [Vec<Comb>; 2],
    allpass: [Vec<Allpass>; 2],
    pub room: f32,  // 0..1: decay time
    pub damp: f32,  // 0..1: high-frequency damping
    pub width: f32, // 0..1: stereo spread
}

impl Reverb {
    pub fn new(rate: u32) -> Self {
        const COMB: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
        const AP: [usize; 4] = [556, 441, 341, 225];
        const SPREAD: usize = 23;
        let k = rate as f64 / 44100.0;
        let sc = |n: usize| ((n as f64 * k).round() as usize).max(1);
        Reverb {
            combs: [COMB.iter().map(|&n| Comb::new(sc(n))).collect(), COMB.iter().map(|&n| Comb::new(sc(n + SPREAD))).collect()],
            allpass: [AP.iter().map(|&n| Allpass::new(sc(n))).collect(), AP.iter().map(|&n| Allpass::new(sc(n + SPREAD))).collect()],
            room: 0.5,
            damp: 0.5,
            width: 1.0,
        }
    }

    /// `io` holds the dry bus on entry and the wet reverb on return.
    pub fn process(&mut self, io: &mut [f32]) {
        let feedback = 0.7 + 0.28 * self.room.clamp(0.0, 1.0);
        let damp = self.damp.clamp(0.0, 1.0) * 0.4;
        let w1 = self.width * 0.5 + 0.5;
        let w2 = (1.0 - self.width) * 0.5;
        for fr in io.chunks_exact_mut(2) {
            let input = (fr[0] + fr[1]) * 0.015;
            let mut out = [0.0f32; 2];
            for (s, o) in out.iter_mut().enumerate() {
                let mut acc = 0.0;
                for c in self.combs[s].iter_mut() {
                    acc += c.tick(input, feedback, damp);
                }
                for a in self.allpass[s].iter_mut() {
                    acc = a.tick(acc);
                }
                *o = acc;
            }
            fr[0] = out[0] * w1 + out[1] * w2;
            fr[1] = out[1] * w1 + out[0] * w2;
        }
    }
}

/// The per-track chain: filter(s) -> delay, plus the amount sent to the shared reverb.
#[derive(Default)]
pub struct TrackFx {
    pub lowpass: Option<Biquad>,
    pub highpass: Option<Biquad>,
    pub delay: Option<Delay>,
    /// 0..1 amount sent to the shared reverb bus
    pub send: f32,
}

impl TrackFx {
    pub fn process(&mut self, buf: &mut [f32]) {
        if let Some(f) = self.highpass.as_mut() {
            f.process(buf);
        }
        if let Some(f) = self.lowpass.as_mut() {
            f.process(buf);
        }
        if let Some(d) = self.delay.as_mut() {
            d.process(buf);
        }
    }
}

use crate::project::TrackFxModel;

impl TrackFxModel {
    /// Set up a track's effect chain from this model (the same code serves live playback and export, so they sound alike).
    pub fn apply(&self, fx: &mut TrackFx, rate: u32) {
        fx.lowpass = (self.lowpass_hz > 0).then(|| Biquad::new(FilterKind::LowPass, self.lowpass_hz as f64, 0.707, rate));
        fx.highpass = (self.highpass_hz > 0).then(|| Biquad::new(FilterKind::HighPass, self.highpass_hz as f64, 0.707, rate));
        fx.delay = (self.delay_mix > 0.0).then(|| {
            let mut d = Delay::new(1000.0, rate);
            d.set_time_frames((self.delay_ms as f64 * rate as f64 / 1000.0) as usize);
            d.feedback = self.delay_feedback;
            d.mix = self.delay_mix;
            d
        });
        fx.send = self.reverb_send;
    }
}

/// The shared reverb exists only while some track sends to it (an idle reverb costs CPU for nothing).
pub fn sync_reverb(mixer: &mut crate::mixer::Mixer, room: f32) {
    let any = mixer.tracks.iter().any(|t| t.fx.send > 0.0);
    match (any, mixer.reverb.as_mut()) {
        (true, Some(r)) => r.room = room,
        (true, None) => {
            let mut r = Reverb::new(mixer.rate);
            r.room = room;
            mixer.reverb = Some(r);
        }
        (false, _) => mixer.reverb = None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const R: u32 = 44100;

    #[test]
    fn a_model_builds_the_chain_and_the_reverb_exists_only_while_something_sends() {
        let mut fx = TrackFx::default();
        TrackFxModel::default().apply(&mut fx, R);
        assert!(fx.lowpass.is_none() && fx.highpass.is_none() && fx.delay.is_none() && fx.send == 0.0, "default = all off");
        let m = TrackFxModel { lowpass_hz: 2000, highpass_hz: 100, delay_ms: 250, delay_feedback: 0.4, delay_mix: 0.3, reverb_send: 0.5 };
        m.apply(&mut fx, R);
        assert!(fx.lowpass.is_some() && fx.highpass.is_some() && fx.delay.is_some() && fx.send == 0.5);
        // the chain really filters: a 10 kHz tone through the 2 kHz low-pass is far quieter
        let mut b: Vec<f32> = (0..2 * 20_000).map(|i| ((i / 2) as f32 * 2.0 * std::f32::consts::PI * 10_000.0 / R as f32).sin()).collect();
        let before = b.iter().map(|x| x * x).sum::<f32>();
        fx.process(&mut b);
        assert!(b[20_000..].iter().map(|x| x * x).sum::<f32>() < before * 0.01);

        let mut mx = crate::mixer::Mixer::new(R, 1024);
        mx.tracks.push(crate::mixer::Track::new("A"));
        sync_reverb(&mut mx, 0.7);
        assert!(mx.reverb.is_none(), "nothing sends: no reverb");
        mx.tracks[0].fx.send = 0.3;
        sync_reverb(&mut mx, 0.7);
        assert!((mx.reverb.as_ref().unwrap().room - 0.7).abs() < 1e-6);
        sync_reverb(&mut mx, 0.2);
        assert!((mx.reverb.as_ref().unwrap().room - 0.2).abs() < 1e-6, "the room follows the setting");
        mx.tracks[0].fx.send = 0.0;
        sync_reverb(&mut mx, 0.2);
        assert!(mx.reverb.is_none());
    }

    fn sine(freq: f64, frames: usize) -> Vec<f32> {
        (0..frames).flat_map(|i| {
            let v = (2.0 * PI * freq * i as f64 / R as f64).sin() as f32;
            [v, v]
        }).collect()
    }
    fn rms_tail(b: &[f32]) -> f64 {
        let t = &b[b.len() / 2..]; // skip the filter's settling
        (t.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / t.len() as f64).sqrt()
    }
    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn lowpass_and_highpass_have_the_right_shape() {
        let reference = rms_tail(&sine(1000.0, 44100));
        let gain = |kind, cutoff, freq| {
            let mut f = Biquad::new(kind, cutoff, 0.707, R);
            let mut b = sine(freq, 44100);
            f.process(&mut b);
            db(rms_tail(&b) / reference)
        };
        assert!(gain(FilterKind::LowPass, 1000.0, 100.0).abs() < 0.5, "LP passes 100 Hz");
        assert!(gain(FilterKind::LowPass, 1000.0, 8000.0) < -30.0, "LP kills 8 kHz");
        assert!((gain(FilterKind::LowPass, 1000.0, 1000.0) - -3.0).abs() < 0.5, "-3 dB at the cutoff");
        assert!(gain(FilterKind::HighPass, 1000.0, 8000.0).abs() < 0.5, "HP passes 8 kHz");
        assert!(gain(FilterKind::HighPass, 1000.0, 100.0) < -30.0, "HP kills 100 Hz");
    }

    #[test]
    fn delay_repeats_at_the_exact_time_and_decays() {
        let mut d = Delay::new(500.0, R);
        d.set_time_frames(100);
        d.feedback = 0.5;
        d.mix = 1.0;
        let mut b = vec![0.0f32; 2 * 500];
        b[0] = 1.0;
        b[1] = -1.0;
        d.process(&mut b);
        let at = |i: usize| (b[2 * i], b[2 * i + 1]);
        assert_eq!(at(0), (1.0, -1.0), "dry passes through");
        assert_eq!(at(100), (1.0, -1.0));
        assert_eq!(at(200), (0.5, -0.5));
        assert_eq!(at(300), (0.25, -0.25));
        assert_eq!(at(150), (0.0, 0.0));
    }

    #[test]
    fn reverb_tail_is_stereo_finite_decaying_and_stable() {
        let mut r = Reverb::new(R);
        r.room = 0.6;
        let mut b = vec![0.0f32; 2 * R as usize * 4];
        b[0] = 1.0;
        b[1] = 1.0;
        r.process(&mut b);
        let seg = |a: f64, c: f64| -> f64 {
            let (i, j) = ((a * R as f64) as usize * 2, (c * R as f64) as usize * 2);
            b[i..j].iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
        };
        assert!(b.iter().all(|x| x.is_finite()));
        assert!(seg(0.05, 0.5) > 0.0, "there is a tail");
        assert!(seg(2.0, 2.5) < seg(0.05, 0.55) * 0.5, "the tail decays");
        assert!(b.chunks_exact(2).skip(2000).any(|f| (f[0] - f[1]).abs() > 1e-6), "left and right differ (stereo)");
        // mono width collapses the two sides
        let mut m = Reverb::new(R);
        m.width = 0.0;
        let mut c = vec![0.0f32; 2 * 8000];
        c[0] = 1.0;
        c[1] = 1.0;
        m.process(&mut c);
        assert!(c.chunks_exact(2).all(|f| (f[0] - f[1]).abs() < 1e-6));
    }

    #[test]
    fn reverb_stays_bounded_under_loud_continuous_input() {
        let mut r = Reverb::new(R);
        r.room = 1.0;
        r.damp = 0.0;
        let mut peak = 0.0f32;
        for _ in 0..40 {
            let mut b = sine(220.0, 44100);
            r.process(&mut b);
            peak = b.iter().fold(peak, |m, x| m.max(x.abs()));
        }
        assert!(peak.is_finite() && peak < 5.0, "peak {peak}");
    }

    #[test]
    fn filter_and_delay_chain_is_bit_identical_for_any_chunk_size() {
        let src = sine(330.0, 20000);
        let run = |chunk: usize| {
            let mut fx = TrackFx {
                lowpass: Some(Biquad::new(FilterKind::LowPass, 2000.0, 1.0, R)),
                highpass: Some(Biquad::new(FilterKind::HighPass, 80.0, 0.7, R)),
                delay: Some(Delay::new(300.0, R)),
                send: 0.0,
            };
            fx.delay.as_mut().unwrap().set_time_frames(4410);
            let mut out = Vec::new();
            for c in src.chunks(chunk * 2) {
                let mut b = c.to_vec();
                fx.process(&mut b);
                out.extend(b);
            }
            out
        };
        let whole = run(20000);
        assert!(run(64) == whole && run(1) == whole && run(777) == whole);
    }

    #[test]
    fn reverb_is_bit_identical_for_any_chunk_size() {
        let src = sine(330.0, 12000);
        let run = |chunk: usize| {
            let mut rv = Reverb::new(R);
            let mut out = Vec::new();
            for c in src.chunks(chunk * 2) {
                let mut b = c.to_vec();
                rv.process(&mut b);
                out.extend(b);
            }
            out
        };
        let whole = run(12000);
        assert!(run(1) == whole && run(333) == whole);
    }
}
