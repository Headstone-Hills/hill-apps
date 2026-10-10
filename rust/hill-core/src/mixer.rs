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
}

impl Voice {
    fn idle() -> Self {
        Voice { sample: None, pos: 0.0, step: 1.0, gain: 0.0, key: 0, age: 0, env: 1.0, fade: 0.0 }
    }
}

pub struct Mixer {
    voices: Vec<Voice>,
    pub rate: f32,
    counter: u64,
    pub master: f32,
    /// Release time applied by `release`, in seconds.
    pub release_secs: f32,
}

impl Mixer {
    pub fn new(rate: f32) -> Self {
        Mixer {
            voices: (0..MAX_VOICES).map(|_| Voice::idle()).collect(),
            rate,
            counter: 0,
            master: 0.8,
            release_secs: 0.08,
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

    pub fn stop_all(&mut self) {
        for v in self.voices.iter_mut() {
            v.sample = None;
        }
    }

    /// Add this block's audio into `out` (interleaved stereo).
    pub fn render(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let master = self.master;
        for v in self.voices.iter_mut() {
            let Some(s) = v.sample.as_ref() else { continue };
            let n = s.frames();
            let data = &s.data;
            let mut pos = v.pos;
            let mut done = false;
            for f in 0..frames {
                let i = pos as usize;
                if i + 1 >= n {
                    done = true;
                    break;
                }
                let fr = (pos - i as f64) as f32;
                let l = data[i * 2] + (data[i * 2 + 2] - data[i * 2]) * fr;
                let r = data[i * 2 + 1] + (data[i * 2 + 3] - data[i * 2 + 1]) * fr;
                let g = v.gain * v.env * master;
                out[f * 2] += l * g;
                out[f * 2 + 1] += r * g;
                pos += v.step;
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
        Arc::new(Sample { data, rate: 44100 })
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
