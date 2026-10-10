//! Built-in sounds, so every app makes noise with no sample files at all
//! (and the browser build works before any files are loaded).

use crate::instrument::Instrument;
use crate::wav::Sample;
use std::f32::consts::TAU;
use std::sync::Arc;

const SR: u32 = 44100;

/// (name, pitched)
pub const BUILTINS: [(&str, bool); 8] = [
    ("Kick", false),
    ("Snare", false),
    ("Hat", false),
    ("Clap", false),
    ("Tom", false),
    ("Pluck", true),
    ("Bass", true),
    ("Bell", true),
];

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        ((self.0 >> 8) as f32 / 8388608.0) - 1.0
    }
}

fn mono(v: Vec<f32>) -> Arc<Sample> {
    let mut data = Vec::with_capacity(v.len() * 2);
    for s in v {
        let s = s.clamp(-1.0, 1.0);
        data.push(s);
        data.push(s);
    }
    Arc::new(Sample { data, rate: SR })
}

fn frames(secs: f32) -> usize {
    (secs * SR as f32) as usize
}

fn kick() -> Vec<f32> {
    let n = frames(0.35);
    let mut ph = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let f = 42.0 + 120.0 * (-t * 28.0).exp();
            ph += TAU * f / SR as f32;
            ph.sin() * (-t * 9.0).exp() * 0.95
        })
        .collect()
}

fn snare() -> Vec<f32> {
    let n = frames(0.25);
    let mut r = Rng(7);
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let tone = (TAU * 185.0 * t).sin() * (-t * 30.0).exp() * 0.5;
            let noise = r.next() * (-t * 17.0).exp() * 0.7;
            tone + noise
        })
        .collect()
}

fn hat() -> Vec<f32> {
    let n = frames(0.07);
    let mut r = Rng(99);
    let mut prev = 0.0;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let x = r.next();
            let hp = x - prev; // crude high-pass
            prev = x;
            hp * (-t * 70.0).exp() * 0.55
        })
        .collect()
}

fn clap() -> Vec<f32> {
    let n = frames(0.22);
    let mut r = Rng(1234);
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let burst = |o: f32| if t >= o { (-(t - o) * 90.0).exp() } else { 0.0 };
            let env = burst(0.0) * 0.8 + burst(0.012) * 0.8 + burst(0.024) * 0.8 + (-t * 14.0).exp() * 0.3;
            r.next() * env * 0.6
        })
        .collect()
}

fn tom() -> Vec<f32> {
    let n = frames(0.4);
    let mut ph = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let f = 95.0 + 80.0 * (-t * 14.0).exp();
            ph += TAU * f / SR as f32;
            ph.sin() * (-t * 8.0).exp() * 0.9
        })
        .collect()
}

/// Pitched tones are rendered at C4 (midi 60, 261.63 Hz).
const C4: f32 = 261.6256;

fn pluck() -> Vec<f32> {
    let n = frames(1.4);
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let mut s = 0.0;
            for h in 1..=8 {
                let hf = h as f32;
                s += (TAU * C4 * hf * t).sin() / hf * (-t * (3.0 + hf * 2.2)).exp();
            }
            s * 0.45
        })
        .collect()
}

fn bass() -> Vec<f32> {
    let n = frames(1.2);
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let env = (-t * 2.8).exp() * (1.0 - (-t * 200.0).exp());
            ((TAU * C4 * t).sin() + 0.35 * (TAU * C4 * 2.0 * t).sin() + 0.15 * (TAU * C4 * 3.0 * t).sin()) * env * 0.6
        })
        .collect()
}

fn bell() -> Vec<f32> {
    let n = frames(2.0);
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let m = (TAU * C4 * 3.5 * t).sin() * 2.0 * (-t * 5.0).exp();
            ((TAU * C4 * t + m).sin() * (-t * 2.2).exp()) * 0.5
        })
        .collect()
}

pub fn builtin(id: usize) -> Option<Instrument> {
    let (name, _) = *BUILTINS.get(id)?;
    let s = mono(match id {
        0 => kick(),
        1 => snare(),
        2 => hat(),
        3 => clap(),
        4 => tom(),
        5 => pluck(),
        6 => bass(),
        _ => bell(),
    });
    Some(Instrument::single(name, 60, s))
}

pub fn builtin_by_name(name: &str) -> Option<Instrument> {
    BUILTINS.iter().position(|(n, _)| *n == name).and_then(builtin)
}

/// Short metronome click.
pub fn click(accent: bool) -> Arc<Sample> {
    let n = frames(0.03);
    let f = if accent { 1600.0 } else { 1000.0 };
    mono(
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                (TAU * f * t).sin() * (-t * 150.0).exp() * 0.8
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtins_are_audible_and_finite() {
        for id in 0..BUILTINS.len() {
            let i = builtin(id).unwrap();
            let s = i.first().unwrap();
            let peak = s.data.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 0.1 && peak <= 1.0, "{} peak {}", i.name, peak);
            assert!(s.data.iter().all(|x| x.is_finite()));
        }
    }
}
