//! Two synthesised 8-bit drum kits, built in code (no sample files): "8-Bit NES" (triangle bass and toms, stepped noise snare and hats)
//! and "8-Bit GB" (pulse-wave bass and toms, shorter and brighter noise). Every sound is made the way the chips did it: a 15-bit LFSR
//! for noise (the short 93-step mode for hats and cymbals sounds metallic), square/triangle oscillators with a pitch sweep, and a
//! 4-bit amplitude (16 levels), which is where the crunch comes from.
//!
//! The file names match what HillBeat's kit matcher looks for ("Kick", "Snare", "Hat Closed", "Tom Hi" ...), so the owner's
//! `Preset::plan` lays a pattern onto these kits exactly as it does for a folder of samples.

/// Sample rate of the generated sounds.
pub const RATE: u32 = 44_100;

/// Kit folder names, as shown in the browser.
pub const NES: &str = "8-Bit NES";
pub const GB: &str = "8-Bit GB";

#[derive(Clone, Copy)]
enum Wave {
    Tri,
    Pulse(f32),
}

fn frames(secs: f32) -> usize {
    (secs * RATE as f32) as usize
}

/// Round to `levels` steps per unit: the chips' 4-bit output.
fn quant(x: f32, levels: f32) -> f32 {
    (x * levels).round() / levels
}

fn wave(w: Wave, ph: f32) -> f32 {
    match w {
        Wave::Tri => 4.0 * (ph - 0.5).abs() - 1.0,
        Wave::Pulse(duty) => {
            if ph < duty {
                1.0
            } else {
                -1.0
            }
        }
    }
}

/// An oscillator whose pitch falls from `f0` to `f1` (rate `sweep` per second) under a decaying 4-bit envelope.
fn tone(dur: f32, f0: f32, f1: f32, sweep: f32, w: Wave, decay: f32) -> Vec<f32> {
    let mut ph = 0.0f32;
    (0..frames(dur))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let f = f1 + (f0 - f1) * (-t * sweep).exp();
            ph = (ph + f / RATE as f32).fract();
            let env = quant((-t * decay).exp(), 15.0) * (t * 800.0).min(1.0);
            quant(wave(w, ph) * env, 15.0) * 0.9
        })
        .collect()
}

/// The chips' noise channel: a 15-bit LFSR clocked at `clock` Hz; `short` taps bit 6 instead of bit 1 (period 93, a metallic pitch).
fn noise(dur: f32, clock: f32, short: bool, decay: f32) -> Vec<f32> {
    let mut lfsr: u16 = 1;
    let mut acc = 0.0f32;
    let mut out = 1.0f32;
    (0..frames(dur))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            acc += clock / RATE as f32;
            while acc >= 1.0 {
                acc -= 1.0;
                let fb = (lfsr ^ (lfsr >> if short { 6 } else { 1 })) & 1;
                lfsr = (lfsr >> 1) | (fb << 14);
                out = if lfsr & 1 == 0 { 1.0 } else { -1.0 };
            }
            let env = quant((-t * decay).exp(), 15.0) * (t * 800.0).min(1.0);
            quant(out * env, 15.0) * 0.8
        })
        .collect()
}

/// `b` mixed into `a` starting at `at` seconds, `a` growing if needed.
fn mix(mut a: Vec<f32>, b: &[f32], at: f32, gain: f32) -> Vec<f32> {
    let off = frames(at);
    if a.len() < off + b.len() {
        a.resize(off + b.len(), 0.0);
    }
    for (i, v) in b.iter().enumerate() {
        a[off + i] += v * gain;
    }
    a
}

/// Scale so the loudest sample is `peak` (the kit's sounds are then level with each other).
fn normalised(mut v: Vec<f32>, peak: f32) -> Vec<f32> {
    let m = v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if m > 0.0 {
        v.iter_mut().for_each(|x| *x *= peak / m);
    }
    v
}

/// One kit's files: (file stem, mono samples).
pub type KitSounds = Vec<(&'static str, Vec<f32>)>;

fn nes() -> KitSounds {
    let clap = {
        let burst = noise(0.012, 12_000.0, false, 90.0);
        let mut c = Vec::new();
        for k in 0..3 {
            c = mix(c, &burst, k as f32 * 0.011, 1.0);
        }
        mix(c, &noise(0.14, 8_000.0, false, 24.0), 0.033, 0.8)
    };
    let snare = mix(noise(0.2, 9_000.0, false, 17.0), &tone(0.09, 200.0, 150.0, 30.0, Wave::Tri, 30.0), 0.0, 0.8);
    vec![
        ("Kick", tone(0.24, 135.0, 38.0, 26.0, Wave::Tri, 9.0)),
        ("Snare", snare),
        ("Hat Closed", noise(0.05, 30_000.0, true, 75.0)),
        ("Hat Open", noise(0.3, 30_000.0, true, 10.0)),
        ("Clap", clap),
        ("Rim", tone(0.035, 1_800.0, 1_650.0, 40.0, Wave::Pulse(0.5), 110.0)),
        ("Tom Hi", tone(0.2, 270.0, 190.0, 14.0, Wave::Tri, 11.0)),
        ("Tom Mid", tone(0.22, 200.0, 140.0, 14.0, Wave::Tri, 10.0)),
        ("Tom Lo", tone(0.26, 150.0, 98.0, 14.0, Wave::Tri, 9.0)),
        ("Crash", noise(0.8, 42_000.0, true, 4.0)),
        ("Cowbell", mix(tone(0.2, 587.0, 587.0, 1.0, Wave::Pulse(0.5), 14.0), &tone(0.2, 845.0, 845.0, 1.0, Wave::Pulse(0.5), 14.0), 0.0, 1.0)),
    ]
}

fn gb() -> KitSounds {
    let clap = {
        let burst = noise(0.01, 20_000.0, false, 100.0);
        let mut c = Vec::new();
        for k in 0..4 {
            c = mix(c, &burst, k as f32 * 0.009, 1.0);
        }
        mix(c, &noise(0.11, 14_000.0, false, 30.0), 0.036, 0.7)
    };
    let snare = mix(noise(0.16, 14_000.0, false, 22.0), &tone(0.06, 260.0, 190.0, 40.0, Wave::Pulse(0.25), 38.0), 0.0, 0.7);
    vec![
        ("Kick", tone(0.2, 190.0, 46.0, 34.0, Wave::Pulse(0.5), 11.0)),
        ("Snare", snare),
        ("Hat Closed", noise(0.04, 60_000.0, true, 90.0)),
        ("Hat Open", noise(0.24, 60_000.0, true, 13.0)),
        ("Clap", clap),
        ("Rim", tone(0.03, 2_400.0, 2_100.0, 50.0, Wave::Pulse(0.125), 130.0)),
        ("Tom Hi", tone(0.16, 330.0, 240.0, 16.0, Wave::Pulse(0.25), 14.0)),
        ("Tom Mid", tone(0.18, 240.0, 170.0, 16.0, Wave::Pulse(0.25), 12.0)),
        ("Tom Lo", tone(0.22, 175.0, 115.0, 16.0, Wave::Pulse(0.25), 11.0)),
        ("Crash", noise(0.6, 70_000.0, true, 5.0)),
        ("Cowbell", mix(tone(0.16, 560.0, 560.0, 1.0, Wave::Pulse(0.25), 18.0), &tone(0.16, 810.0, 810.0, 1.0, Wave::Pulse(0.25), 18.0), 0.0, 1.0)),
    ]
}

/// The built-in kits: (kit name, [(file stem, mono samples at [`RATE`])]).
pub fn kits() -> Vec<(&'static str, KitSounds)> {
    [(NES, nes()), (GB, gb())]
        .into_iter()
        .map(|(name, files)| (name, files.into_iter().map(|(stem, v)| (stem, normalised(v, 0.85))).collect()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_audible_finite_short_and_level() {
        for (kit, files) in kits() {
            assert_eq!(files.len(), 11, "{kit}");
            for (stem, v) in files {
                assert!(v.len() > frames(0.02) && v.len() < frames(1.0), "{kit}/{stem}: {} frames", v.len());
                assert!(v.iter().all(|x| x.is_finite()), "{kit}/{stem}");
                let peak = v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                assert!((peak - 0.85).abs() < 1e-3, "{kit}/{stem}: peak {peak}");
                assert!(v.iter().filter(|x| x.abs() > 0.05).count() > 100, "{kit}/{stem} is mostly silent");
            }
        }
    }

    #[test]
    fn the_sounds_are_four_bit() {
        // before normalising, the oscillator and noise outputs sit on a 16-step grid
        let v = noise(0.05, 20_000.0, false, 10.0);
        assert!(v.iter().all(|x| ((x / 0.8) * 15.0 - ((x / 0.8) * 15.0).round()).abs() < 1e-3));
    }

    #[test]
    fn generation_is_deterministic_and_the_two_kits_differ() {
        let (a, b) = (kits(), kits());
        assert_eq!(a[0].1[1].1, b[0].1[1].1, "same noise every time");
        assert_ne!(a[0].1[0].1, a[1].1[0].1, "NES and GB kicks are different sounds");
    }
}
