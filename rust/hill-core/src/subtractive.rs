//! The HillSynth engine: eight voices, each with two band-limited oscillators, a
//! resonant state-variable low-pass filter (with an LFO), and an ADSR envelope.
//!
//! No effects (the Hill apps leave them out). Everything is mono; the app copies it to both
//! channels. `render_note` renders one finished note offline for the multisample export.

use std::f64::consts::PI;

pub const VOICES: usize = 8;
pub const WAVES: [&str; 5] = ["sine", "saw", "square", "triangle", "pulse25"];

/// How a parameter is stored and stepped.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// Index into `WAVES`.
    Wave,
    Lin { min: f32, max: f32, step: f32 },
    /// Multiplied or divided by `ratio` per step (cutoff, times).
    Log { min: f32, max: f32, ratio: f32 },
    Int { min: i32, max: i32 },
}

pub struct ParamDef {
    pub name: &'static str,
    pub key: &'static str,
    pub kind: Kind,
    pub default: f32,
}

pub const W1: usize = 0;
pub const V1: usize = 1;
pub const D1: usize = 2;
pub const W2: usize = 3;
pub const V2: usize = 4;
pub const D2: usize = 5;
pub const O2: usize = 6;
pub const CUT: usize = 7;
pub const RES: usize = 8;
pub const LFR: usize = 9;
pub const LFD: usize = 10;
pub const ATK: usize = 11;
pub const DEC: usize = 12;
pub const SUS: usize = 13;
pub const REL: usize = 14;
pub const VOL: usize = 15;
pub const N_PARAMS: usize = 16;

const fn lin(min: f32, max: f32, step: f32) -> Kind {
    Kind::Lin { min, max, step }
}
const fn log(min: f32, max: f32, ratio: f32) -> Kind {
    Kind::Log { min, max, ratio }
}

pub const PARAMS: [ParamDef; N_PARAMS] = [
    ParamDef { name: "Wave", key: "wave1", kind: Kind::Wave, default: 1.0 },
    ParamDef { name: "Level", key: "vol1", kind: lin(0.0, 1.0, 0.05), default: 0.8 },
    ParamDef { name: "Detune", key: "detune1", kind: lin(-50.0, 50.0, 1.0), default: 0.0 },
    ParamDef { name: "Wave", key: "wave2", kind: Kind::Wave, default: 2.0 },
    ParamDef { name: "Level", key: "vol2", kind: lin(0.0, 1.0, 0.05), default: 0.4 },
    ParamDef { name: "Detune", key: "detune2", kind: lin(-50.0, 50.0, 1.0), default: 7.0 },
    ParamDef { name: "Octave", key: "oct2", kind: Kind::Int { min: -2, max: 2 }, default: 0.0 },
    ParamDef { name: "Cutoff", key: "cutoff", kind: log(20.0, 20000.0, 1.1225), default: 4000.0 },
    ParamDef { name: "Resonance", key: "res", kind: lin(0.0, 0.95, 0.05), default: 0.3 },
    ParamDef { name: "LFO rate", key: "lfo_rate", kind: log(0.1, 20.0, 1.2), default: 2.0 },
    ParamDef { name: "LFO depth", key: "lfo_depth", kind: lin(0.0, 1.0, 0.05), default: 0.0 },
    ParamDef { name: "Attack", key: "attack", kind: log(0.001, 4.0, 1.25), default: 0.01 },
    ParamDef { name: "Decay", key: "decay", kind: log(0.001, 4.0, 1.25), default: 0.2 },
    ParamDef { name: "Sustain", key: "sustain", kind: lin(0.0, 1.0, 0.05), default: 0.7 },
    ParamDef { name: "Release", key: "release", kind: log(0.001, 4.0, 1.25), default: 0.4 },
    ParamDef { name: "Volume", key: "volume", kind: lin(0.0, 1.0, 0.05), default: 0.8 },
];

/// The edit screen's pages: a title and the parameters on it.
pub const PAGES: [(&str, &[usize]); 5] =
    [("OSC 1", &[W1, V1, D1]), ("OSC 2", &[W2, V2, D2, O2]), ("FILTER", &[CUT, RES, LFR, LFD]), ("ENV", &[ATK, DEC, SUS, REL]), ("OUT", &[VOL])];

/// A complete sound.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Patch {
    pub v: [f32; N_PARAMS],
}

impl Default for Patch {
    fn default() -> Self {
        let mut v = [0.0; N_PARAMS];
        for (i, p) in PARAMS.iter().enumerate() {
            v[i] = p.default;
        }
        Patch { v }
    }
}

impl Patch {
    pub fn get(&self, i: usize) -> f32 {
        self.v[i]
    }

    pub fn set(&mut self, i: usize, x: f32) {
        self.v[i] = match PARAMS[i].kind {
            Kind::Wave => x.round().clamp(0.0, (WAVES.len() - 1) as f32),
            Kind::Lin { min, max, .. } => x.clamp(min, max),
            Kind::Log { min, max, .. } => x.clamp(min, max),
            Kind::Int { min, max } => x.round().clamp(min as f32, max as f32),
        };
    }

    /// Move parameter `i` one step up (`dir` > 0) or down; `coarse` takes bigger steps.
    pub fn step(&mut self, i: usize, dir: i32, coarse: bool) {
        let d = dir.signum() as f32;
        let x = self.v[i];
        let n = if coarse { 5.0 } else { 1.0 };
        let next = match PARAMS[i].kind {
            Kind::Wave => (x as i32 + dir.signum()).rem_euclid(WAVES.len() as i32) as f32,
            Kind::Lin { step, .. } => ((x + d * step * n) / step).round() * step,
            Kind::Log { ratio, .. } => x * ratio.powf(d * if coarse { 4.0 } else { 1.0 }),
            Kind::Int { .. } => x + d,
        };
        self.set(i, next);
    }

    /// 0..1 position of the value within its range (for bars).
    pub fn fraction(&self, i: usize) -> f32 {
        let x = self.v[i];
        match PARAMS[i].kind {
            Kind::Wave => x / (WAVES.len() - 1) as f32,
            Kind::Lin { min, max, .. } => (x - min) / (max - min),
            Kind::Log { min, max, .. } => (x / min).ln() / (max / min).ln(),
            Kind::Int { min, max } => (x - min as f32) / (max - min) as f32,
        }
        .clamp(0.0, 1.0)
    }

    pub fn display(&self, i: usize) -> String {
        let x = self.v[i];
        match i {
            W1 | W2 => WAVES[x as usize].to_string(),
            D1 | D2 => format!("{:+.0} ct", x),
            O2 => format!("{:+}", x as i32),
            CUT => {
                if x >= 1000.0 {
                    format!("{:.1} kHz", x / 1000.0)
                } else {
                    format!("{:.0} Hz", x)
                }
            }
            LFR => format!("{:.1} Hz", x),
            ATK | DEC | REL => {
                if x < 0.1 {
                    format!("{:.0} ms", x * 1000.0)
                } else {
                    format!("{:.2} s", x)
                }
            }
            _ => format!("{:.0}%", x * 100.0),
        }
    }

    pub fn to_csv(&self) -> String {
        self.v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
    }

    pub fn from_csv(s: &str) -> Option<Patch> {
        let nums: Vec<f32> = s.split(',').filter_map(|t| t.trim().parse().ok()).collect();
        if nums.len() != N_PARAMS {
            return None;
        }
        let mut p = Patch::default();
        for (i, x) in nums.into_iter().enumerate() {
            p.set(i, x);
        }
        Some(p)
    }
}

/// The built-in presets.
pub fn factory_presets() -> Vec<(&'static str, Patch)> {
    // (name, wave1 vol1 det1, wave2 vol2 det2 oct2, cutoff res, lfo rate depth, a d s r, volume)
    let mk = |v: [f32; N_PARAMS]| Patch { v };
    vec![
        ("Warm Pad", mk([1.0, 0.8, -7.0, 1.0, 0.7, 7.0, 0.0, 1800.0, 0.2, 0.3, 0.1, 0.7, 0.6, 0.7, 1.4, 0.8])),
        ("Soft Keys", mk([3.0, 0.8, 0.0, 0.0, 0.5, 3.0, 1.0, 5200.0, 0.1, 2.0, 0.0, 0.005, 0.7, 0.25, 0.5, 0.8])),
        ("Pluck", mk([1.0, 0.8, 0.0, 2.0, 0.4, 5.0, 0.0, 2600.0, 0.4, 2.0, 0.0, 0.001, 0.28, 0.0, 0.25, 0.8])),
        ("Sub Bass", mk([0.0, 0.9, 0.0, 1.0, 0.45, 0.0, -1.0, 700.0, 0.25, 2.0, 0.0, 0.004, 0.2, 0.8, 0.15, 0.85])),
        ("Bright Lead", mk([1.0, 0.8, -9.0, 1.0, 0.6, 9.0, 0.0, 7000.0, 0.35, 5.5, 0.15, 0.01, 0.2, 0.75, 0.3, 0.75])),
        ("Strings", mk([1.0, 0.7, -10.0, 1.0, 0.7, 10.0, 0.0, 3200.0, 0.15, 4.5, 0.08, 0.4, 0.5, 0.8, 0.9, 0.8])),
        ("Bell", mk([0.0, 0.8, 0.0, 0.0, 0.45, 6.0, 2.0, 12000.0, 0.0, 2.0, 0.0, 0.001, 1.6, 0.0, 1.5, 0.8])),
        ("Hollow Reed", mk([2.0, 0.7, 0.0, 4.0, 0.6, 4.0, 0.0, 1500.0, 0.55, 3.0, 0.2, 0.03, 0.3, 0.6, 0.35, 0.8])),
    ]
}

#[inline]
fn poly_blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// One oscillator sample at phase `t` (0..1) advancing `dt` per sample.
fn osc(wave: u8, t: f64, dt: f64) -> f64 {
    match wave {
        0 => (2.0 * PI * t).sin(),
        1 => 2.0 * t - 1.0 - poly_blep(t, dt),
        2 => {
            let naive = if t < 0.5 { 1.0 } else { -1.0 };
            naive + poly_blep(t, dt) - poly_blep((t + 0.5).fract(), dt)
        }
        3 => 4.0 * (t - 0.5).abs() - 1.0,
        _ => {
            let w = 0.25;
            let naive = if t < w { 1.0 } else { -1.0 };
            naive + poly_blep(t, dt) - poly_blep((t + 1.0 - w).fract(), dt)
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone)]
struct Voice {
    on: bool,
    note: u8,
    freq: f64,
    ph1: f64,
    ph2: f64,
    lfo: f64,
    env: f32,
    stage: Stage,
    gate: bool,
    /// Released while the pedal was down: sustained until it comes up.
    pedal_hold: bool,
    ic1: f32,
    ic2: f32,
    age: u64,
}

impl Voice {
    fn idle() -> Self {
        Voice { on: false, note: 0, freq: 440.0, ph1: 0.0, ph2: 0.0, lfo: 0.0, env: 0.0, stage: Stage::Release, gate: false, pedal_hold: false, ic1: 0.0, ic2: 0.0, age: 0 }
    }
}

pub struct Synth {
    pub patch: Patch,
    voices: Vec<Voice>,
    rate: f32,
    counter: u64,
    pedal: bool,
}

const BLOCK: usize = 16;

impl Synth {
    pub fn new(rate: f32) -> Self {
        Synth { patch: Patch::default(), voices: vec![Voice::idle(); VOICES], rate, counter: 0, pedal: false }
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
    }

    /// Voices that are making sound (held or releasing).
    pub fn active(&self) -> usize {
        self.voices.iter().filter(|v| v.on).count()
    }

    pub fn note_on(&mut self, note: u8) {
        self.counter += 1;
        let idx = self
            .voices
            .iter()
            .position(|v| v.on && v.note == note) // the same note again restarts that voice
            .or_else(|| self.voices.iter().position(|v| !v.on))
            .or_else(|| {
                // steal: the quietest releasing voice, else the oldest
                self.voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| !v.gate)
                    .min_by(|a, b| a.1.env.partial_cmp(&b.1.env).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(i, _)| i)
            })
            .unwrap_or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
        let v = &mut self.voices[idx];
        if !v.on {
            *v = Voice::idle(); // a fresh voice starts from silence
        }
        // a restarted or stolen voice keeps its level and phase, so there is no click
        v.on = true;
        v.note = note;
        v.freq = 440.0 * 2f64.powf((note as f64 - 69.0) / 12.0);
        v.gate = true;
        v.pedal_hold = false;
        v.stage = Stage::Attack;
        v.age = self.counter;
    }

    pub fn note_off(&mut self, note: u8) {
        let pedal = self.pedal;
        for v in self.voices.iter_mut().filter(|v| v.on && v.note == note && v.gate) {
            v.gate = false;
            if pedal {
                v.pedal_hold = true;
            } else {
                v.stage = Stage::Release;
            }
        }
    }

    /// Sustain pedal: notes released while it is down keep sounding until it comes up.
    pub fn set_pedal(&mut self, down: bool) {
        self.pedal = down;
        if !down {
            for v in self.voices.iter_mut().filter(|v| v.pedal_hold) {
                v.pedal_hold = false;
                v.stage = Stage::Release;
            }
        }
    }

    pub fn all_off(&mut self) {
        for v in self.voices.iter_mut() {
            *v = Voice::idle();
        }
        self.pedal = false;
    }

    /// Add `out.len()` mono samples.
    pub fn render(&mut self, out: &mut [f32]) {
        let p = self.patch;
        let rate = self.rate as f64;
        let atk_inc = 1.0 / (p.v[ATK].max(0.001) * self.rate);
        let dec_c = 1.0 - (-4.6 / (p.v[DEC].max(0.001) * self.rate)).exp();
        let rel_r = (-4.6 / (p.v[REL].max(0.001) * self.rate)).exp();
        let sus = p.v[SUS];
        let (w1, w2) = (p.v[W1] as u8, p.v[W2] as u8);
        let (l1, l2) = (p.v[V1] * 0.5, p.v[V2] * 0.5);
        let ratio2 = 2f64.powf(p.v[D2] as f64 / 1200.0 + p.v[O2] as f64);
        let ratio1 = 2f64.powf(p.v[D1] as f64 / 1200.0);
        let k = 2.0 - 1.94 * p.v[RES];
        let lfo_inc = p.v[LFR] as f64 * BLOCK as f64 / rate;
        let max_cut = (0.45 * rate) as f32;

        for v in self.voices.iter_mut().filter(|v| v.on) {
            let (dt1, dt2) = (v.freq * ratio1 / rate, v.freq * ratio2 / rate);
            let (mut a1, mut a2, mut a3) = (0.0f32, 0.0f32, 0.0f32);
            for (n, o) in out.iter_mut().enumerate() {
                if n % BLOCK == 0 {
                    // filter coefficients, updated at control rate (the LFO moves the cutoff)
                    let lfo = (2.0 * PI * v.lfo).sin() as f32;
                    v.lfo = (v.lfo + lfo_inc).fract();
                    let fc = (p.v[CUT] * 2f32.powf(p.v[LFD] * lfo)).clamp(20.0, max_cut);
                    let g = (PI * fc as f64 / rate).tan() as f32;
                    a1 = 1.0 / (1.0 + g * (g + k));
                    a2 = g * a1;
                    a3 = g * a2;
                }
                // envelope
                match v.stage {
                    Stage::Attack => {
                        v.env += atk_inc;
                        if v.env >= 1.0 {
                            v.env = 1.0;
                            v.stage = Stage::Decay;
                        }
                    }
                    Stage::Decay => {
                        v.env += (sus - v.env) * dec_c;
                        if (v.env - sus).abs() < 0.001 {
                            v.stage = Stage::Sustain;
                        }
                    }
                    Stage::Sustain => v.env = sus,
                    Stage::Release => {
                        v.env *= rel_r;
                        if v.env < 1e-4 {
                            v.on = false;
                            break;
                        }
                    }
                }
                if v.stage == Stage::Sustain && sus < 0.001 && v.env < 1e-4 && !v.pedal_hold {
                    v.on = false; // a plucked sound that has died away
                    break;
                }
                // oscillators
                let s = osc(w1, v.ph1, dt1) as f32 * l1 + osc(w2, v.ph2, dt2) as f32 * l2;
                v.ph1 = (v.ph1 + dt1).fract();
                v.ph2 = (v.ph2 + dt2).fract();
                // trapezoidal state-variable low-pass
                let v3 = s - v.ic2;
                let v1 = a1 * v.ic1 + a2 * v3;
                let v2 = v.ic2 + a2 * v.ic1 + a3 * v3;
                v.ic1 = 2.0 * v1 - v.ic1;
                v.ic2 = 2.0 * v2 - v.ic2;
                *o += v2 * v.env;
            }
        }
        let vol = p.v[VOL];
        for o in out.iter_mut() {
            *o = soft_clip(*o * vol);
        }
    }
}

/// Linear below 0.7, then a smooth knee to 1: loud chords and resonance do not clip hard.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.7 {
        x
    } else {
        let over = (a - 0.7) / 0.3;
        x.signum() * (0.7 + 0.3 * (over / (1.0 + over)).min(1.0))
    }
}

/// One finished note, rendered offline: held for `hold` seconds, then released for `tail`.
pub fn render_note(patch: &Patch, midi: u8, rate: f32, hold: f32, tail: f32) -> Vec<f32> {
    let mut s = Synth::new(rate);
    s.patch = *patch;
    s.note_on(midi);
    let mut out = vec![0.0; ((hold + tail) * rate) as usize];
    let split = (hold * rate) as usize;
    s.render(&mut out[..split]);
    s.note_off(midi);
    s.render(&mut out[split..]);
    // end the file at silence, with no click
    let fade = (0.02 * rate) as usize;
    let n = out.len();
    for i in 0..fade.min(n) {
        out[n - 1 - i] *= i as f32 / fade as f32;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    fn run(s: &mut Synth, secs: f32) -> Vec<f32> {
        let mut out = vec![0.0; (secs * SR) as usize];
        s.render(&mut out);
        out
    }

    fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0.0, |m, x| m.max(x.abs()))
    }

    fn sine_patch() -> Patch {
        let mut p = Patch::default();
        p.set(W1, 0.0);
        p.set(V1, 1.0);
        p.set(V2, 0.0);
        p.set(CUT, 20000.0);
        p.set(RES, 0.0);
        p.set(ATK, 0.001);
        p.set(SUS, 1.0);
        p
    }

    #[test]
    fn silent_until_a_note_plays_then_audible_and_finite() {
        let mut s = Synth::new(SR);
        assert_eq!(peak(&run(&mut s, 0.1)), 0.0);
        s.note_on(60);
        let out = run(&mut s, 0.3);
        assert!(peak(&out) > 0.1 && out.iter().all(|x| x.is_finite()));
        assert!(peak(&out) <= 1.0);
    }

    #[test]
    fn pitch_is_right() {
        let mut s = Synth::new(SR);
        s.patch = sine_patch();
        s.note_on(69); // A4 = 440 Hz
        run(&mut s, 0.05);
        let out = run(&mut s, 1.0);
        let rising = out.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((rising as i32 - 440).abs() <= 2, "{} cycles in a second", rising);
    }

    #[test]
    fn adsr_reaches_the_sustain_level_and_releases_to_silence() {
        let mut s = Synth::new(SR);
        s.patch = sine_patch();
        s.patch.set(SUS, 0.5);
        s.patch.set(DEC, 0.05);
        s.patch.set(REL, 0.1);
        s.note_on(60);
        run(&mut s, 0.5);
        let held = peak(&run(&mut s, 0.1));
        s.note_off(60);
        run(&mut s, 0.2);
        let quiet = peak(&run(&mut s, 0.5));
        assert!((held - 0.8 * 0.5 * 0.5).abs() < 0.02, "sustain level {}", held); // volume .8 * level .5 * sustain .5
        assert!(quiet < 0.001 && s.active() == 0, "released to silence {}", quiet);
    }

    #[test]
    fn a_lower_cutoff_is_darker() {
        let bright = |cut: f32| {
            let mut s = Synth::new(SR);
            s.patch.set(W1, 1.0);
            s.patch.set(CUT, cut);
            s.note_on(48);
            run(&mut s, 0.1);
            let out = run(&mut s, 0.3);
            // energy in the sample-to-sample differences is mostly high-frequency content
            out.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>() / out.iter().map(|x| x * x).sum::<f32>()
        };
        assert!(bright(500.0) < bright(8000.0) * 0.2, "{} vs {}", bright(500.0), bright(8000.0));
    }

    #[test]
    fn stable_at_full_resonance_and_cutoff() {
        let mut s = Synth::new(SR);
        s.patch.set(RES, 0.95);
        s.patch.set(CUT, 20000.0);
        s.patch.set(LFD, 1.0);
        for n in [24, 48, 72, 96] {
            s.note_on(n);
        }
        let out = run(&mut s, 1.0);
        assert!(out.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
    }

    #[test]
    fn eight_voices_then_stealing_and_retrigger_does_not_stack() {
        let mut s = Synth::new(SR);
        for n in 60..68 {
            s.note_on(n);
        }
        assert_eq!(s.active(), 8);
        s.note_on(70); // a ninth note steals one
        assert_eq!(s.active(), 8);
        let mut t = Synth::new(SR);
        for _ in 0..5 {
            t.note_on(60);
        }
        assert_eq!(t.active(), 1, "the same note again restarts its voice");
    }

    #[test]
    fn the_pedal_holds_released_notes() {
        let mut s = Synth::new(SR);
        s.patch.set(REL, 0.05);
        s.set_pedal(true);
        s.note_on(60);
        run(&mut s, 0.3);
        s.note_off(60);
        assert!(peak(&run(&mut s, 0.5)) > 0.05, "still sounding while the pedal is down");
        s.set_pedal(false);
        run(&mut s, 0.3);
        assert!(peak(&run(&mut s, 0.3)) < 0.001, "releases when the pedal comes up");
    }

    #[test]
    fn parameters_step_clamp_and_round_trip() {
        let mut p = Patch::default();
        p.step(V1, 1, false);
        assert!((p.get(V1) - 0.85).abs() < 1e-5 || p.get(V1) == 0.85);
        for _ in 0..100 {
            p.step(V1, 1, true);
        }
        assert_eq!(p.get(V1), 1.0);
        let c0 = p.get(CUT);
        p.step(CUT, 1, false);
        assert!(p.get(CUT) > c0);
        p.step(W1, 1, false);
        assert_eq!(p.get(W1) as usize, 2); // saw -> square
        for _ in 0..4 {
            p.step(W1, 1, false);
        }
        assert_eq!(p.get(W1) as usize, 1, "waves wrap round");
        let again = Patch::from_csv(&p.to_csv()).unwrap();
        assert_eq!(again, p);
        assert!(Patch::from_csv("1,2,3").is_none());
        for i in 0..N_PARAMS {
            assert!((0.0..=1.0).contains(&p.fraction(i)));
            assert!(!p.display(i).is_empty());
        }
    }

    #[test]
    fn every_factory_preset_sounds_and_stays_in_range() {
        for (name, patch) in factory_presets() {
            let mut s = Synth::new(SR);
            s.patch = patch;
            s.note_on(48);
            let out = run(&mut s, 0.5);
            assert!(peak(&out) > 0.02 && out.iter().all(|x| x.is_finite() && x.abs() <= 1.0), "{}", name);
            let clamped = Patch::from_csv(&patch.to_csv()).unwrap();
            assert_eq!(clamped, patch, "{} has values outside the ranges", name);
        }
    }

    #[test]
    fn rendered_notes_end_in_silence() {
        let n = render_note(&Patch::default(), 60, SR, 0.5, 0.5);
        assert_eq!(n.len(), SR as usize);
        assert!(peak(&n) > 0.1 && n[n.len() - 1].abs() < 1e-3);
    }
}
