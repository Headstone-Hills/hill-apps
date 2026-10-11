//! Voice pool. Samples play through a linear-interpolating resampler, which
//! is how both pitch-shifting and loop varispeed work.

use crate::wav::Sample;
use std::sync::Arc;

const MAX_VOICES: usize = 40;

struct Voice {
    sample: Option<Arc<Sample>>,
    pos: f64,
    step: f64,
    gain: f32,
    key: u32,
    age: u64,
    env: f32,
    fade: f32, // per-frame decrement once released; 0 = holding
    rel: f32,  // per-frame decrement to use when this voice is released
    tail: bool, // longer than a beat: fades smoothly at its own end too (length = rel)
    played: u64, // frames rendered so far, for the maximum-length cap
}

impl Voice {
    fn idle() -> Self {
        Voice { sample: None, pos: 0.0, step: 1.0, gain: 0.0, key: 0, age: 0, env: 1.0, fade: 0.0, rel: 0.0, tail: false, played: 0 }
    }
}

pub struct Mixer {
    voices: Vec<Voice>,
    pub rate: f32,
    counter: u64,
    pub master: f32,
    /// Release time applied by `release`, in seconds.
    pub release_secs: f32,
    /// No voice sounds longer than this many seconds (apps set it to 4 measures).
    pub max_secs: Option<f32>,
    /// One beat in seconds. How long a sound plays, in beats, picks its fade (`fade_secs`).
    pub beat_secs: Option<f32>,
    /// Sequencer sounds longer than a beat follow this envelope (see `Envelope`); None = off.
    pub env: Option<EnvParams>,
}

/// Fade out by how long the sound plays (`beats` at the current tempo), however it
/// ends: its own end, release, the next note, pause or the cap.
///   up to 1 beat: none (a hit or pluck keeps its snap; an early release still gets
///                 `release_secs` so it does not click)
///   up to 4 beats: short   up to 8 beats: medium   over 8 beats: long
/// Returns (fade seconds, fades at its own end too).
fn fade_secs(beats: f64, release_secs: f32) -> (f32, bool) {
    if beats <= 1.0 {
        (release_secs, false)
    } else if beats <= 4.0 {
        (SHORT_FADE_SECS.max(release_secs), true)
    } else if beats <= 8.0 {
        (MEDIUM_FADE_SECS, true)
    } else {
        (LONG_FADE_SECS, true)
    }
}

/// Fade for "stop now" actions (pause, audition cut-off) on sounds with a longer tier fade.
const QUICK_FADE_SECS: f32 = 0.15;
const SHORT_FADE_SECS: f32 = 0.1;
const MEDIUM_FADE_SECS: f32 = 0.25;
const LONG_FADE_SECS: f32 = 0.6;

/// S-shaped gain for a fade position 1 (full) .. 0 (silent): gentle at both ends.
#[inline]
fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Settings for the envelope given to long sounds in the sequencer apps (see `Envelope`).
/// All times are in beats, so they follow the tempo.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct EnvParams {
    /// Attack: how long the sound stays at full level before it starts to decay.
    pub hold: f32,
    /// Decay: how long the fall from full level to the sustain level takes.
    pub decay: f32,
    /// Sustain level, 0..1 of the full level.
    pub sustain: f32,
    /// Release length scale: 1 is the standard length (see `Envelope`).
    pub release: f32,
}

impl Default for EnvParams {
    fn default() -> Self {
        EnvParams { hold: 0.5, decay: 0.5, sustain: 0.25, release: 1.0 }
    }
}

impl EnvParams {
    pub const HOLD: (f32, f32, f32) = (0.25, 4.0, 0.25); // min, max, step
    pub const DECAY: (f32, f32, f32) = (0.125, 4.0, 0.125);
    pub const SUSTAIN: (f32, f32, f32) = (0.0, 1.0, 0.05);
    pub const RELEASE: (f32, f32, f32) = (0.25, 4.0, 0.25);

    pub fn clamped(self) -> Self {
        let c = |v: f32, r: (f32, f32, f32)| v.clamp(r.0, r.1);
        EnvParams { hold: c(self.hold, Self::HOLD), decay: c(self.decay, Self::DECAY), sustain: c(self.sustain, Self::SUSTAIN), release: c(self.release, Self::RELEASE) }
    }
}

/// The shape given to long sounds in the sequencer apps, in frames.
///   attack  : full level for `hold` beats
///   decay   : from full to `sustain` over `decay` beats, steep at first then easing out (cubic)
///   sustain : held at the sustain level
///   release : fades the held level to silence at the end of the sound (or the cap)
/// The release is short (a quarter beat) for sounds up to two beats, and for longer ones grows with
/// the sound's length (0.5 + 0.2 x its beats, up to 3), all times `release`. Nothing steps:
/// every part joins the next at the same level.
pub struct Envelope {
    hold: f64,
    decay: f64,
    sustain: f32,
    release_start: f64,
    release: f64,
    base_at_release: f32,
}

impl Envelope {
    /// For a sound that plays `end` frames at `beat` frames per beat; None if it is no
    /// longer than a beat.
    pub fn new(beat: f64, end: f64, p: &EnvParams) -> Option<Envelope> {
        if end <= beat {
            return None;
        }
        let beats = end / beat;
        let release_beats = if beats <= 2.0 { 0.25 } else { (0.5 + 0.2 * beats).min(3.0) } * p.release as f64;
        let release = (release_beats * beat).min(end - beat).max(1.0);
        let release_start = end - release;
        let hold = p.hold as f64 * beat;
        let decay = (p.decay as f64 * beat).min((release_start - hold).max(0.0)).max(1.0);
        let mut e = Envelope { hold, decay, sustain: p.sustain, release_start, release, base_at_release: 1.0 };
        e.base_at_release = e.base(release_start);
        Some(e)
    }

    /// Level before the release: full for the hold, then sustain + (1 - sustain)(1 - x)^3.
    fn base(&self, t: f64) -> f32 {
        if t <= self.hold {
            return 1.0;
        }
        let left = (1.0 - (t - self.hold) / self.decay).clamp(0.0, 1.0) as f32;
        self.sustain + (1.0 - self.sustain) * left * left * left
    }

    pub fn gain(&self, t: f64) -> f32 {
        if t < self.release_start {
            self.base(t)
        } else {
            let y = ((t - self.release_start) / self.release).clamp(0.0, 1.0) as f32;
            self.base_at_release * smooth(1.0 - y)
        }
    }
}

impl Mixer {
    pub fn new(rate: f32) -> Self {
        Mixer {
            voices: (0..MAX_VOICES).map(|_| Voice::idle()).collect(),
            rate,
            counter: 0,
            master: 0.8,
            release_secs: 0.08,
            max_secs: None,
            beat_secs: None,
            env: None,
        }
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
    }

    pub fn active(&self) -> usize {
        self.voices.iter().filter(|v| v.sample.is_some()).count()
    }

    /// Start a voice. `key` groups voices for `release`/limiting; at most
    /// `limit` voices with the same key sound at once (oldest is cut).
    pub fn trigger(&mut self, s: &Arc<Sample>, ratio: f64, gain: f32, key: u32, limit: usize) {
        self.counter += 1;
        // Triggering a sound that is already sounding restarts it (a loop), not a second copy on top.
        let step = ratio * s.rate as f64 / self.rate as f64;
        for v in self.voices.iter_mut() {
            if v.key == key && v.fade == 0.0 && v.sample.as_ref().is_some_and(|o| Arc::ptr_eq(o, s)) && (v.step - step).abs() < 1e-9 {
                v.fade = v.rel;
            }
        }
        if limit > 0 {
            let mut same: Vec<usize> = (0..self.voices.len())
                .filter(|&i| self.voices[i].sample.is_some() && self.voices[i].key == key && self.voices[i].fade == 0.0)
                .collect();
            if same.len() >= limit {
                same.sort_by_key(|&i| self.voices[i].age);
                for &i in same.iter().take(same.len() + 1 - limit) {
                    self.voices[i].fade = self.voices[i].rel; // fade the oldest, don't chop it
                }
            }
        }
        // the fade depends on how many beats this sound plays for
        let (rel_secs, tail) = match self.beat_secs {
            Some(b) if b > 0.0 => fade_secs(s.frames() as f64 / step / (b * self.rate) as f64, self.release_secs),
            _ => (self.release_secs, false),
        };
        let rel = 1.0 / (rel_secs * self.rate).max(1.0);
        let idx = match self.voices.iter().position(|v| v.sample.is_none()) {
            Some(i) => i,
            None => self
                .voices
                .iter()
                .enumerate()
                .min_by_key(|(_, v)| v.age)
                .map(|(i, _)| i)
                .unwrap_or(0),
        };
        self.voices[idx] = Voice {
            sample: Some(s.clone()),
            pos: 0.0,
            step: ratio * s.rate as f64 / self.rate as f64,
            gain,
            key,
            age: self.counter,
            env: 1.0,
            fade: 0.0,
            rel,
            tail,
            played: 0,
        };
    }

    /// Begin fading out every held voice with this key.
    pub fn release(&mut self, key: u32) {
        for v in self.voices.iter_mut() {
            if v.sample.is_some() && v.key == key && v.fade == 0.0 {
                v.fade = v.rel;
            }
        }
    }

    /// Fade out every sounding voice (pause). Always quick, whatever the length.
    pub fn release_all(&mut self) {
        let quick = 1.0 / (QUICK_FADE_SECS * self.rate).max(1.0);
        for v in self.voices.iter_mut() {
            if v.sample.is_some() && v.fade == 0.0 {
                v.fade = v.rel.max(quick);
            }
        }
    }

    /// Like `release`, but never slower than `QUICK_FADE_SECS` (cutting an audition note).
    pub fn release_quick(&mut self, key: u32) {
        let quick = 1.0 / (QUICK_FADE_SECS * self.rate).max(1.0);
        for v in self.voices.iter_mut() {
            if v.sample.is_some() && v.key == key && v.fade == 0.0 {
                v.fade = v.rel.max(quick);
            }
        }
    }

    pub fn stop_all(&mut self) {
        for v in self.voices.iter_mut() {
            v.sample = None;
        }
    }

    /// Add this block's audio into `out` (interleaved stereo).
    pub fn render(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let master = self.master;
        let max_frames = self.max_secs.map(|s| (s * self.rate) as u64);
        let beat_frames = self.beat_secs.map(|b| (b * self.rate) as f64);
        for v in self.voices.iter_mut() {
            let Some(s) = v.sample.as_ref() else { continue };
            let n = s.frames();
            // the envelope for a sound longer than a beat (it ends at its own end or the cap)
            let env = match (&self.env, beat_frames) {
                (Some(p), Some(b)) => {
                    let len = n as f64 / v.step;
                    Envelope::new(b, max_frames.map_or(len, |m| len.min(m as f64)), p)
                }
                _ => None,
            };
            // a voice that sounds longer than a beat fades out at its own end, over the
            // same length as its release (the fade is in source frames: step per output frame)
            let tail = if env.is_none() && v.tail && v.rel > 0.0 { (1.0 / v.rel) as f64 * v.step } else { 0.0 };
            let mut pos = v.pos;
            let mut done = false;
            for f in 0..frames {
                let i = pos as usize;
                if i + 1 >= n {
                    done = true;
                    break;
                }
                let fr = (pos - i as f64) as f32;
                let (l0, r0) = s.frame(i);
                let (l1, r1) = s.frame(i + 1);
                let l = l0 + (l1 - l0) * fr;
                let r = r0 + (r1 - r0) * fr;
                let mut g = v.gain * smooth(v.env) * master;
                if let Some(e) = &env {
                    g *= e.gain(v.played as f64);
                }
                if tail > 0.0 {
                    let left = (n - 1) as f64 - pos;
                    if left < tail {
                        g *= smooth((left / tail) as f32);
                    }
                }
                out[f * 2] += l * g;
                out[f * 2 + 1] += r * g;
                pos += v.step;
                v.played += 1;
                if let Some(m) = max_frames {
                    if v.played >= m && v.fade == 0.0 {
                        v.fade = v.rel;
                    }
                }
                if v.fade > 0.0 {
                    v.env -= v.fade;
                    if v.env <= 0.0 {
                        done = true;
                        break;
                    }
                }
            }
            v.pos = pos;
            if done {
                v.sample = None;
            }
        }
        for x in out.iter_mut() {
            *x = x.clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Arc<Sample> {
        let mut data = Vec::new();
        for _ in 0..n {
            data.push(0.5);
            data.push(0.5);
        }
        Arc::new(Sample::new(data, 44100))
    }

    #[test]
    fn voices_are_capped_at_max_secs() {
        let mut m = Mixer::new(1000.0);
        m.max_secs = Some(1.0);
        m.release_secs = 0.01;
        let long = Arc::new(Sample::new(vec![0.5; 2 * 10_000], 1000)); // 10 s
        m.trigger(&long, 1.0, 1.0, 1, 0);
        let mut out = vec![0.0; 2 * 1500];
        m.render(&mut out);
        assert_eq!(m.active(), 0, "gone after 1 s plus the release");
        assert!(out[2 * 900] > 0.1 && out[2 * 1400].abs() < 1e-6);
    }

    #[test]
    fn fade_length_follows_sample_length_in_beats() {
        // mixer at 1 kHz, one beat = 0.5 s (120 bpm): tiers at 0.5, 2, 4 s
        let mk = |secs: f32| Arc::new(Sample::new(vec![0.5; 2 * (secs * 1000.0) as usize], 1000));
        // (length s, expected fade ms at the very end; 0 = none)
        for (secs, fade_ms) in [(0.4f32, 0), (1.5, 100), (3.5, 250), (6.0, 600)] {
            let mut m = Mixer::new(1000.0);
            m.beat_secs = Some(0.5);
            m.master = 1.0;
            m.trigger(&mk(secs), 1.0, 1.0, 1, 0);
            let frames = (secs * 1000.0) as usize;
            let mut out = vec![0.0; 2 * frames];
            m.render(&mut out);
            let at = |f: usize| out[2 * f];
            assert!((at(frames / 4) - 0.5).abs() < 1e-3, "{}s: body untouched", secs);
            if fade_ms == 0 {
                assert!(at(frames - 3) > 0.49, "{}s: no fade", secs);
            } else {
                let f = fade_ms as usize;
                assert!(at(frames - 3) < 0.01, "{}s: faded at the end ({})", secs, at(frames - 3));
                assert!(at(frames - f - 20) > 0.49, "{}s: fade starts {} ms out", secs, f);
                assert!((at(frames - f / 2) - 0.25).abs() < 0.04, "{}s: S-curve midpoint {}", secs, at(frames - f / 2));
            }
        }
    }

    /// Level (left channel) of a constant 0.5 sample played through the sequencer envelope,
    /// at 1 kHz with a 0.5 s beat (500 frames).
    fn env_run(secs: f32, cap: Option<f32>) -> Vec<f32> {
        env_run_at(1000, secs, cap)
    }

    fn env_run_at(rate: u32, secs: f32, cap: Option<f32>) -> Vec<f32> {
        let frames = (secs * rate as f32) as usize;
        let mut m = Mixer::new(rate as f32);
        m.beat_secs = Some(0.5);
        m.max_secs = cap;
        m.env = Some(EnvParams::default());
        m.master = 1.0;
        let s = Arc::new(Sample::new(vec![0.5; 2 * frames], rate));
        m.trigger(&s, 1.0, 1.0, 1, 0);
        let mut out = vec![0.0; 2 * frames];
        m.render(&mut out);
        out.chunks(2).map(|p| p[0]).collect()
    }

    #[test]
    fn long_sounds_attack_decay_to_a_low_sustain_then_release() {
        let a = env_run(4.0, None); // 8 beats
        // attack: full level for exactly the first half beat (250 frames)
        assert!(a[..=250].iter().all(|v| (v - 0.5).abs() < 1e-3), "full level for the attack hold");
        // decay: falls steeply right after it (already well down a fifth of the way in)...
        assert!(a[300] < 0.5 * 0.65 && a[300] > 0.5 * EnvParams::default().sustain, "steep early fall: {}", a[300]);
        // ...to a lower sustain (a quarter of the attack level) once the half-beat decay is over
        let sustain = 0.5 * EnvParams::default().sustain;
        assert!((a[500] - sustain).abs() < 0.002 && (a[2000] - sustain).abs() < 0.002, "sustain {}", a[2000]);
        // release: a long fade to silence at the end
        assert!(a[3990] < 0.003, "silent at the end: {}", a[3990]);
        // never rises, never steps
        assert!(a[250..].windows(2).all(|w| w[1] <= w[0] + 1e-6), "monotone");
        // (the 500-frame beat of this test makes the designed slope 0.0045 a frame at its steepest)
        let biggest = a[250..].windows(2).map(|w| (w[0] - w[1]).abs()).fold(0.0, f32::max);
        assert!(biggest < 0.0055, "no jumps in level: {}", biggest);
        // at the real 44.1 kHz rate (a 22050-frame beat) every step is tiny
        let real = env_run_at(44100, 4.0, None);
        let biggest = real[11025..].windows(2).map(|w| (w[0] - w[1]).abs()).fold(0.0, f32::max);
        assert!(biggest < 0.00015, "no jumps in level at 44.1 kHz: {}", biggest);
        // a sound of one beat or less is untouched
        let d = env_run(0.4, None);
        assert!((d[390] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn release_is_longer_for_samples_over_two_beats() {
        // frames before the end at which the level has left the sustain level
        let release_len = |v: &[f32]| v.len() - v.iter().rposition(|x| *x >= 0.5 * EnvParams::default().sustain * 0.99).unwrap();
        let two_beats = release_len(&env_run(1.0, None)); // exactly 2 beats: short release (0.25 beat)
        let six_beats = release_len(&env_run(3.0, None)); // 6 beats: 0.5 + 0.2 * 6 = 1.7 beats
        let sixteen_beats = release_len(&env_run(8.0, None)); // 16 beats: capped at 3 beats
        assert!(two_beats <= 140, "2 beats: {}", two_beats);
        // (the S-shaped release starts flat, so the audible part is a little under its nominal length)
        assert!((750..900).contains(&six_beats), "6 beats: {}", six_beats);
        assert!((1350..1550).contains(&sixteen_beats), "16 beats: {}", sixteen_beats);
        assert!(two_beats < six_beats && six_beats < sixteen_beats, "longer sample, longer release");
    }

    #[test]
    fn envelope_ends_at_the_cap() {
        // a 10 s sound with a 4 s cap releases to silence by the cap, not by its own end
        let c = env_run(10.0, Some(4.0));
        assert!(c[3990] < 0.003 && c[200] > 0.49 && c[2000] > 0.1, "{} {} {}", c[3990], c[200], c[2000]);
    }

    #[test]
    fn retrigger_restarts_instead_of_stacking() {
        let mut m = Mixer::new(1000.0);
        m.release_secs = 0.01;
        let s = Arc::new(Sample::new(vec![0.5; 2 * 10_000], 1000));
        m.trigger(&s, 1.0, 1.0, 1, 0);
        m.trigger(&s, 1.0, 1.0, 1, 0);
        m.trigger(&s, 1.0, 1.0, 1, 0);
        let mut out = vec![0.0; 2 * 50];
        m.render(&mut out);
        assert_eq!(m.active(), 1, "earlier copies faded out");
        // a different pitch is a different voice
        m.trigger(&s, 2.0, 1.0, 1, 0);
        assert_eq!(m.active(), 2);
    }

    #[test]
    fn plays_and_finishes() {
        let mut m = Mixer::new(44100.0);
        m.trigger(&ramp(100), 1.0, 1.0, 1, 0);
        assert_eq!(m.active(), 1);
        let mut out = vec![0.0; 400];
        m.render(&mut out);
        assert!(out[0] > 0.3);
        assert_eq!(m.active(), 0);
        assert_eq!(out[250], 0.0);
    }

    #[test]
    fn double_rate_halves_length() {
        let mut m = Mixer::new(44100.0);
        m.trigger(&ramp(1000), 2.0, 1.0, 1, 0);
        let mut out = vec![0.0; 2 * 400];
        m.render(&mut out);
        assert_eq!(m.active(), 1);
        let mut out = vec![0.0; 2 * 400];
        m.render(&mut out);
        assert_eq!(m.active(), 0);
    }

    #[test]
    fn key_limit_cuts_oldest() {
        let mut m = Mixer::new(44100.0);
        for _ in 0..5 {
            m.trigger(&ramp(10000), 1.0, 1.0, 7, 3);
        }
        // the two oldest are fading, not chopped; once the 80 ms release is over 3 remain
        let mut out = vec![0.0; 2 * 6615];
        m.render(&mut out);
        assert_eq!(m.active(), 3);
    }

    #[test]
    fn release_fades_to_silence() {
        let mut m = Mixer::new(44100.0);
        m.trigger(&ramp(100000), 1.0, 1.0, 9, 0);
        m.release(9);
        let mut out = vec![0.0; 2 * 8000];
        m.render(&mut out);
        assert_eq!(m.active(), 0);
    }

    #[test]
    fn output_is_clamped() {
        let mut m = Mixer::new(44100.0);
        m.master = 4.0;
        m.trigger(&ramp(100), 1.0, 1.0, 1, 0);
        let mut out = vec![0.0; 20];
        m.render(&mut out);
        assert!(out.iter().all(|x| x.abs() <= 1.0));
    }
}
