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
    played: u64, // frames rendered so far, for the maximum-length cap
}

impl Voice {
    fn idle() -> Self {
        Voice { sample: None, pos: 0.0, step: 1.0, gain: 0.0, key: 0, age: 0, env: 1.0, fade: 0.0, played: 0 }
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
    /// One beat in seconds. A voice that sounds longer than this fades out over its
    /// last `END_FADE_SECS` instead of stopping abruptly; shorter ones are left alone.
    pub beat_secs: Option<f32>,
}

/// Length of the fade at the natural end of a long sound.
const END_FADE_SECS: f32 = 0.06;

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
        let fade = 1.0 / (self.release_secs * self.rate).max(1.0);
        for v in self.voices.iter_mut() {
            if v.key == key && v.fade == 0.0 && v.sample.as_ref().is_some_and(|o| Arc::ptr_eq(o, s)) && (v.step - step).abs() < 1e-9 {
                v.fade = fade;
            }
        }
        if limit > 0 {
            let mut same: Vec<usize> = (0..self.voices.len())
                .filter(|&i| self.voices[i].sample.is_some() && self.voices[i].key == key)
                .collect();
            if same.len() >= limit {
                same.sort_by_key(|&i| self.voices[i].age);
                for &i in same.iter().take(same.len() + 1 - limit) {
                    self.voices[i].sample = None;
                }
            }
        }
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
            played: 0,
        };
    }

    /// Begin fading out every held voice with this key.
    pub fn release(&mut self, key: u32) {
        let fade = 1.0 / (self.release_secs * self.rate).max(1.0);
        for v in self.voices.iter_mut() {
            if v.sample.is_some() && v.key == key && v.fade == 0.0 {
                v.fade = fade;
            }
        }
    }

    /// Fade out every sounding voice (pause, preview cut-off).
    pub fn release_all(&mut self) {
        let fade = 1.0 / (self.release_secs * self.rate).max(1.0);
        for v in self.voices.iter_mut() {
            if v.sample.is_some() && v.fade == 0.0 {
                v.fade = fade;
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
        let cap_fade = 1.0 / (self.release_secs * self.rate).max(1.0);
        let beat_frames = self.beat_secs.map(|b| b * self.rate);
        let end_fade_frames = END_FADE_SECS * self.rate;
        for v in self.voices.iter_mut() {
            let Some(s) = v.sample.as_ref() else { continue };
            let n = s.frames();
            // output frames this voice will sound in total; long ones get an end fade
            let tail = match beat_frames {
                Some(b) if n as f64 / v.step > b as f64 => (end_fade_frames as f64 * v.step).max(1.0),
                _ => 0.0,
            };
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
                let mut g = v.gain * v.env * master;
                if tail > 0.0 {
                    let left = (n - 1) as f64 - pos;
                    if left < tail {
                        g *= (left / tail).max(0.0) as f32;
                    }
                }
                out[f * 2] += l * g;
                out[f * 2 + 1] += r * g;
                pos += v.step;
                v.played += 1;
                if let Some(m) = max_frames {
                    if v.played >= m && v.fade == 0.0 {
                        v.fade = cap_fade;
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
    fn long_sounds_fade_at_their_end_and_short_ones_do_not() {
        let mk = |secs: f32| Arc::new(Sample::new(vec![0.5; 2 * (secs * 1000.0) as usize], 1000));
        for (secs, faded) in [(2.0f32, true), (0.2, false)] {
            let mut m = Mixer::new(1000.0);
            m.beat_secs = Some(0.5);
            m.master = 1.0;
            m.trigger(&mk(secs), 1.0, 1.0, 1, 0);
            let frames = (secs * 1000.0) as usize;
            let mut out = vec![0.0; 2 * frames];
            m.render(&mut out);
            let at = |f: usize| out[2 * f];
            assert!((at(frames / 2) - 0.5).abs() < 1e-3, "body untouched");
            if faded {
                assert!(at(frames - 3) < 0.15, "tail is faded: {}", at(frames - 3));
                assert!(at(frames - 70) > 0.49, "fade is short (60 ms)");
                assert!(at(frames - 30) < at(frames - 50), "monotone");
            } else {
                assert!(at(frames - 3) > 0.49, "short sounds keep their snap");
            }
        }
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
