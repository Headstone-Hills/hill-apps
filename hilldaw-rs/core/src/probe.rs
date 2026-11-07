//! Finding out what a set of MIDI CCs does to the FM-1's sound, from its captured audio.
//!
//! The vendor documents the CCs but the document and the release note disagree, so the mapping is measured instead of
//! guessed. For each trial one note is played and three things are measured on the captured audio: how loud the held note is,
//! how bright it is (spectral centroid) and how much sound is left after the release (reverb/delay tails).
//!
//! Two passes, because an effect's parameter does nothing while its switch is off (a reverb switch with its mix at 0 is silent):
//! * **add one**: start with every probed CC at its low value, raise one CC at a time. Finds switches and anything that works alone.
//! * **one effect at a time**: the CCs are split into groups (the FM-1 guide has 4 per effect: switch, then three parameters). For each
//!   group the first CC (its switch) is on, the others sit at a middle value and every other group is off; then each CC is set to
//!   its low and to its high value in turn. Testing a group alone keeps one effect's extreme settings (a filter closed all the way)
//!   from hiding everything else, which an "everything at maximum" baseline does.
//!
//! Everything here is a pure function of audio; the hardware only supplies a `trial` closure that applies a CC state, plays a
//! note and returns the captured mono audio.

use crate::record::detect_onset;
use std::collections::BTreeMap;
use std::f64::consts::PI;

pub const FFT_N: usize = 16384;
const FLOOR_DB: f64 = -120.0;

/// Where in the captured audio to look, relative to the note's onset.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub rate: u32,
    /// how long the note is held before the note-off
    pub hold_s: f64,
}

/// What one trial sounded like.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Features {
    /// RMS of the held note, dBFS
    pub body_db: f64,
    /// RMS after the release, dBFS (reverb / delay tails; the dry FM-1 is exact digital silence here)
    pub tail_db: f64,
    /// spectral centroid of the held note, Hz (higher = brighter)
    pub centroid_hz: f64,
}

fn db(rms: f64) -> f64 {
    if rms <= 1e-6 { FLOOR_DB } else { (20.0 * rms.log10()).max(FLOOR_DB) }
}

fn rms(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

/// In-place radix-2 FFT; `re.len()` must be a power of two.
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for i in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let (a, b) = (i + k, i + k + len / 2);
                let (vr, vi) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - vr;
                im[b] = im[a] - vi;
                re[a] += vr;
                im[a] += vi;
                let n_cr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = n_cr;
            }
        }
        len <<= 1;
    }
}

/// Spectral centroid (Hz) of `x` between 50 Hz and 12 kHz, Hann-windowed.
pub fn centroid(x: &[f32], rate: u32) -> f64 {
    let n = x.len().min(FFT_N);
    if n < 256 {
        return 0.0;
    }
    let mut re = vec![0.0f64; FFT_N];
    let mut im = vec![0.0f64; FFT_N];
    for (i, s) in x.iter().take(n).enumerate() {
        let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / (n - 1) as f64).cos();
        re[i] = *s as f64 * w;
    }
    fft(&mut re, &mut im);
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for k in 1..FFT_N / 2 {
        let f = k as f64 * rate as f64 / FFT_N as f64;
        if !(50.0..=12000.0).contains(&f) {
            continue;
        }
        let m = (re[k] * re[k] + im[k] * im[k]).sqrt();
        num += f * m;
        den += m;
    }
    if den > 0.0 { num / den } else { 0.0 }
}

/// Measure one trial. `None` when the capture holds no sound at all.
pub fn analyze(x: &[f32], t: &Timing) -> Option<Features> {
    let r = t.rate as f64;
    let onset = detect_onset(x, 0.001)?;
    let body_start = onset + (0.05 * r) as usize;
    let body_end = (onset + (t.hold_s * r) as usize).min(x.len());
    if body_end <= body_start + 256 {
        return None;
    }
    let body = &x[body_start..body_end];
    let tail_start = onset + ((t.hold_s + 0.4) * r) as usize;
    let tail = if tail_start + (0.2 * r) as usize <= x.len() { &x[tail_start..] } else { &[][..] };
    Some(Features { body_db: db(rms(body)), tail_db: if tail.is_empty() { FLOOR_DB } else { db(rms(tail)) }, centroid_hz: centroid(body, t.rate) })
}

/// How much louder / brighter / longer-ringing trial `b` is than `a`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delta {
    pub level_db: f64,
    pub centroid_pct: f64,
    pub tail_db: f64,
}

pub fn compare(a: &Features, b: &Features) -> Delta {
    Delta {
        level_db: b.body_db - a.body_db,
        centroid_pct: if a.centroid_hz > 0.0 { (b.centroid_hz - a.centroid_hz) / a.centroid_hz * 100.0 } else { 0.0 },
        tail_db: b.tail_db - a.tail_db,
    }
}

/// How big a difference has to be before it counts, learned from how much repeated identical trials already differ.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    pub level_db: f64,
    pub centroid_pct: f64,
    pub tail_db: f64,
}

impl Thresholds {
    pub const MIN: Thresholds = Thresholds { level_db: 1.0, centroid_pct: 5.0, tail_db: 3.0 };

    /// At least the minimums, and at least 4x the spread seen among repeated baseline trials.
    pub fn from_baselines(b: &[Features]) -> Thresholds {
        let spread = |f: fn(&Features) -> f64| {
            let (lo, hi) = b.iter().fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(f(x)), hi.max(f(x))));
            if hi >= lo { hi - lo } else { 0.0 }
        };
        let mean_c = b.iter().map(|x| x.centroid_hz).sum::<f64>() / b.len().max(1) as f64;
        Thresholds {
            level_db: Self::MIN.level_db.max(4.0 * spread(|x| x.body_db)),
            centroid_pct: Self::MIN.centroid_pct.max(if mean_c > 0.0 { 4.0 * spread(|x| x.centroid_hz) / mean_c * 100.0 } else { 0.0 }),
            tail_db: Self::MIN.tail_db.max(4.0 * spread(|x| x.tail_db)),
        }
    }
}

impl Delta {
    pub fn changed(&self, t: &Thresholds) -> bool {
        self.level_db.abs() >= t.level_db || self.centroid_pct.abs() >= t.centroid_pct || self.tail_db.abs() >= t.tail_db
    }

    /// Words for the features that moved past the thresholds, e.g. "duller (-31%), tail +12 dB".
    pub fn describe(&self, t: &Thresholds) -> String {
        let mut v = Vec::new();
        if self.level_db.abs() >= t.level_db {
            v.push(format!("{} {:+.1} dB", if self.level_db > 0.0 { "louder" } else { "quieter" }, self.level_db));
        }
        if self.centroid_pct.abs() >= t.centroid_pct {
            v.push(format!("{} {:+.0}%", if self.centroid_pct > 0.0 { "brighter" } else { "duller" }, self.centroid_pct));
        }
        if self.tail_db.abs() >= t.tail_db {
            v.push(format!("tail {:+.0} dB", self.tail_db));
        }
        if v.is_empty() { "no change".into() } else { v.join(", ") }
    }
}

/// What a trial did relative to its baseline. A trial that captured no sound at all, when the baseline had sound,
/// is a change (the CC silenced the note, e.g. a filter closed all the way), not a missing measurement.
fn effect(base: &Features, thr: &Thresholds, audio: &[f32], t: &Timing) -> Effect {
    match analyze(audio, t) {
        Some(f) => {
            let d = compare(base, &f);
            Effect { delta: d, changed: d.changed(thr), text: d.describe(thr) }
        }
        None => Effect { delta: Delta { level_db: FLOOR_DB - base.body_db, centroid_pct: -100.0, tail_db: FLOOR_DB - base.tail_db }, changed: true, text: "silenced (no sound)".into() },
    }
}

/// A value for every probed CC.
pub type State = BTreeMap<u8, u8>;

pub struct Config {
    pub ccs: Vec<u8>,
    pub low: u8,
    pub high: u8,
    /// the resting value of a group's parameters while one of them is tested
    pub mid: u8,
    /// CCs per effect; the first of each group is treated as its on/off switch. 0 = all CCs in one group.
    pub group: usize,
    pub timing: Timing,
    /// identical baseline trials in pass 1, to learn the noise level
    pub baseline_reps: usize,
    /// identical baseline trials per group in pass 2
    pub group_reps: usize,
}

impl Config {
    fn groups(&self) -> Vec<&[u8]> {
        let g = if self.group == 0 { self.ccs.len().max(1) } else { self.group };
        self.ccs.chunks(g).collect()
    }

    /// How many trials `run` will make (for progress messages).
    pub fn trial_count(&self) -> usize {
        let reps1 = self.baseline_reps.max(2);
        let reps2 = self.group_reps.max(2);
        reps1 + self.ccs.len() + self.groups().iter().map(|g| reps2 + 1 + 2 * g.len().saturating_sub(1)).sum::<usize>()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub delta: Delta,
    pub changed: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub cc: u8,
    /// raising this CC alone from the all-low state
    pub add: Effect,
    /// pass 2, relative to its group's resting state (switch on, parameters at `mid`):
    /// this CC set to `low`, and set to `high` (`None` for a group's switch, which is already on)
    pub down: Effect,
    pub up: Option<Effect>,
}

#[derive(Debug)]
pub struct GroupBase {
    pub ccs: Vec<u8>,
    /// the resting value of the group's parameters that gave a sound (`mid`, else `low`); `None` = silent either way, group skipped
    pub rest: Option<u8>,
    pub base: Option<Features>,
    pub thr: Thresholds,
}

#[derive(Debug)]
pub struct Report {
    pub rows: Vec<Row>,
    pub base_low: Features,
    pub thr_low: Thresholds,
    pub groups: Vec<GroupBase>,
}

fn mean(f: &[Features]) -> Features {
    let n = f.len().max(1) as f64;
    Features { body_db: f.iter().map(|x| x.body_db).sum::<f64>() / n, tail_db: f.iter().map(|x| x.tail_db).sum::<f64>() / n, centroid_hz: f.iter().map(|x| x.centroid_hz).sum::<f64>() / n }
}

fn uniform(ccs: &[u8], v: u8) -> State {
    ccs.iter().map(|c| (*c, v)).collect()
}

fn baseline(state: &State, reps: usize, label: &str, cfg: &Config, trial: &mut dyn FnMut(&State) -> Vec<f32>, progress: &mut dyn FnMut(&str)) -> Result<(Features, Thresholds), String> {
    let reps = reps.max(2);
    let mut fs = Vec::new();
    for i in 0..reps {
        progress(&format!("{label} baseline {}/{reps}", i + 1));
        let f = analyze(&trial(state), &cfg.timing).ok_or_else(|| format!("no sound captured at the {label} baseline: is the FM-1 volume up, and is its sound muted?"))?;
        fs.push(f);
    }
    Ok((mean(&fs), Thresholds::from_baselines(&fs)))
}

/// Run both passes. `trial` applies the CC state, plays a note and returns the captured mono audio.
pub fn run(cfg: &Config, trial: &mut dyn FnMut(&State) -> Vec<f32>, progress: &mut dyn FnMut(&str)) -> Result<Report, String> {
    // pass 1: raise each CC alone from all-low
    let mut low = uniform(&cfg.ccs, cfg.low);
    let (base_low, thr_low) = baseline(&low, cfg.baseline_reps, "all-low", cfg, trial, progress)?;
    let mut adds: BTreeMap<u8, Effect> = BTreeMap::new();
    for &cc in &cfg.ccs {
        progress(&format!("CC {cc} up (alone)"));
        low.insert(cc, cfg.high);
        let e = effect(&base_low, &thr_low, &trial(&low), &cfg.timing);
        low.insert(cc, cfg.low);
        adds.insert(cc, e);
    }

    // pass 2: one effect at a time
    let mut rows = Vec::new();
    let mut groups = Vec::new();
    for g in cfg.groups() {
        let label = format!("group CC {}-{}", g[0], g[g.len() - 1]);
        let resting = |rest: u8| {
            let mut st = uniform(&cfg.ccs, cfg.low);
            for (k, &cc) in g.iter().enumerate() {
                st.insert(cc, if k == 0 { cfg.high } else { rest });
            }
            st
        };
        // parameters at mid; if that is silent (a filter closed by its own settings), try them at low; if still silent, skip the group
        let mut found = None;
        for rest in [cfg.mid, cfg.low] {
            let st = resting(rest);
            if let Ok((base, thr)) = baseline(&st, cfg.group_reps, &label, cfg, trial, progress) {
                found = Some((rest, st, base, thr));
                break;
            }
            if cfg.mid == cfg.low {
                break;
            }
        }
        let Some((rest, mut st, base, thr)) = found else {
            let skipped = Effect { delta: Delta { level_db: 0.0, centroid_pct: 0.0, tail_db: 0.0 }, changed: false, text: "skipped: silent at rest".into() };
            for &cc in g {
                rows.push(Row { cc, add: adds.remove(&cc).expect("every probed CC was added"), down: skipped.clone(), up: None });
            }
            groups.push(GroupBase { ccs: g.to_vec(), rest: None, base: None, thr: Thresholds::MIN });
            continue;
        };
        for (k, &cc) in g.iter().enumerate() {
            let at_rest = st[&cc];
            progress(&format!("CC {cc} down ({label})"));
            st.insert(cc, cfg.low);
            let down = effect(&base, &thr, &trial(&st), &cfg.timing);
            let up = if k == 0 {
                None
            } else {
                progress(&format!("CC {cc} up ({label})"));
                st.insert(cc, cfg.high);
                Some(effect(&base, &thr, &trial(&st), &cfg.timing))
            };
            st.insert(cc, at_rest);
            rows.push(Row { cc, add: adds.remove(&cc).expect("every probed CC was added"), down, up });
        }
        groups.push(GroupBase { ccs: g.to_vec(), rest: Some(rest), base: Some(base), thr });
    }
    Ok(Report { rows, base_low, thr_low, groups })
}

impl Report {
    /// Plain-text table for pasting back into a chat.
    pub fn render(&self) -> String {
        let mut s = String::new();
        let line = |name: &str, b: &Features, t: &Thresholds| format!("{name}: level {:.1} dB, centroid {:.0} Hz, tail {:.0} dB   (noise limits {:.1} dB / {:.0}% / {:.0} dB)\n", b.body_db, b.centroid_hz, b.tail_db, t.level_db, t.centroid_pct, t.tail_db);
        s += &line("all CCs low", &self.base_low, &self.thr_low);
        for g in &self.groups {
            let name = format!("group {}-{}", g.ccs[0], g.ccs[g.ccs.len() - 1]);
            match (&g.base, g.rest) {
                (Some(b), Some(r)) => s += &line(&format!("{name} resting (switch on, others at {r})"), b, &g.thr),
                _ => s += &format!("{name}: silent at rest (switch on, others at mid and at 0), skipped\n"),
            }
        }
        s += "\nADD = raise this CC alone from all-low.   DOWN / UP = set this CC to 0 / 127 while its effect group is on and the rest of the group is at mid.\n";
        s += &format!("{:<4} {:<38} {:<38} {}\n", "CC", "ADD", "DOWN (to 0)", "UP (to 127)");
        for r in &self.rows {
            s += &format!("{:<4} {:<38} {:<38} {}\n", r.cc, r.add.text, r.down.text, r.up.as_ref().map_or("-".to_string(), |e| e.text.clone()));
        }
        let ids = |f: &dyn Fn(&Row) -> bool| self.rows.iter().filter(|r| f(r)).map(|r| r.cc.to_string()).collect::<Vec<_>>().join(" ");
        s += &format!("\nchanged the sound when raised alone: {}\n", ids(&|r| r.add.changed));
        s += &format!("changed the sound within their group (down or up): {}\n", ids(&|r| r.down.changed || r.up.as_ref().is_some_and(|e| e.changed)));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = 44100;
    const T: Timing = Timing { rate: R, hold_s: 0.6 };

    /// A held note of harmonics of `f0` (amplitude 1/h) up to `max_hz`, then an optional ringing tail. 0.3 s of silence first.
    fn note(f0: f64, max_hz: f64, tail_amp: f64, total_s: f64) -> Vec<f32> {
        let n = (total_s * R as f64) as usize;
        let (on, off) = ((0.3 * R as f64) as usize, (0.9 * R as f64) as usize);
        let mut x = vec![0.0f32; n];
        for (i, s) in x.iter_mut().enumerate().skip(on) {
            let t = i as f64 / R as f64;
            let v = if i < off {
                let env = ((i - on) as f64 / (0.02 * R as f64)).min(1.0);
                (1..=30).filter(|h| f0 * *h as f64 <= max_hz).map(|h| (2.0 * PI * f0 * h as f64 * t).sin() / h as f64).sum::<f64>() * 0.3 * env
            } else {
                let since = (i - off) as f64 / R as f64;
                (2.0 * PI * 330.0 * t).sin() * tail_amp * (-since / 0.8).exp()
            };
            *s = v as f32;
        }
        x
    }

    #[test]
    fn fft_finds_a_pure_tone() {
        let mut re: Vec<f64> = (0..FFT_N).map(|i| (2.0 * PI * 1000.0 * i as f64 / R as f64).sin()).collect();
        let mut im = vec![0.0; FFT_N];
        fft(&mut re, &mut im);
        let peak = (1..FFT_N / 2).max_by(|a, b| (re[*a].powi(2) + im[*a].powi(2)).total_cmp(&(re[*b].powi(2) + im[*b].powi(2)))).unwrap();
        assert!((peak as f64 * R as f64 / FFT_N as f64 - 1000.0).abs() < 5.0);
    }

    #[test]
    fn a_duller_note_has_a_lower_centroid_and_a_tail_shows_up() {
        let bright = analyze(&note(220.0, 8000.0, 0.0, 2.4), &T).unwrap();
        let dull = analyze(&note(220.0, 500.0, 0.0, 2.4), &T).unwrap();
        assert!(dull.centroid_hz < bright.centroid_hz * 0.6, "{} vs {}", dull.centroid_hz, bright.centroid_hz);
        assert!(bright.tail_db <= FLOOR_DB, "no tail: digital silence");
        let wet = analyze(&note(220.0, 8000.0, 0.2, 2.4), &T).unwrap();
        assert!(wet.tail_db > -40.0, "tail {} dB", wet.tail_db);
        let d = compare(&bright, &dull);
        assert!(d.changed(&Thresholds::MIN) && d.describe(&Thresholds::MIN).contains("duller"), "{}", d.describe(&Thresholds::MIN));
        let louder = analyze(&note(220.0, 8000.0, 0.0, 2.4).iter().map(|s| s * 2.0).collect::<Vec<_>>(), &T).unwrap();
        assert!((compare(&bright, &louder).level_db - 6.02).abs() < 0.1);
    }

    #[test]
    fn identical_trials_differ_by_nothing_and_silence_is_reported() {
        let a = analyze(&note(220.0, 8000.0, 0.0, 2.4), &T).unwrap();
        assert!(!compare(&a, &a).changed(&Thresholds::MIN));
        assert_eq!(compare(&a, &a).describe(&Thresholds::MIN), "no change");
        assert!(analyze(&vec![0.0; 100_000], &T).is_none());
        assert!(analyze(&[], &T).is_none());
    }

    #[test]
    fn thresholds_grow_with_the_noise_seen_in_the_baselines() {
        let quiet = [Features { body_db: -20.0, tail_db: -120.0, centroid_hz: 1000.0 }; 3];
        assert_eq!(Thresholds::from_baselines(&quiet), Thresholds::MIN);
        let noisy = [Features { body_db: -20.0, tail_db: -50.0, centroid_hz: 1000.0 }, Features { body_db: -17.0, tail_db: -40.0, centroid_hz: 1200.0 }];
        let t = Thresholds::from_baselines(&noisy);
        assert!((t.level_db - 12.0).abs() < 1e-9 && (t.tail_db - 40.0).abs() < 1e-9 && t.centroid_pct > 50.0, "{t:?}");
    }

    /// A pretend FM-1 with a hidden mapping in two groups of four:
    /// filter 70 switch, 71 type (inert here), 72 cutoff (200 + 60 x value Hz, only while the switch is on), 73 inert;
    /// reverb 74 switch, 75 inert, 76 mix (tail amplitude value/127 x 0.3, only while the switch is on), 77 inert.
    /// A *global* all-high state would put this filter at 7.8 kHz and the reverb at full, so it also checks that testing one group at a time works.
    fn fake(state: &State) -> Vec<f32> {
        let g = |c: u8| *state.get(&c).unwrap_or(&0);
        let max_hz = if g(70) > 0 { 200.0 + 60.0 * g(72) as f64 } else { 8000.0 };
        let tail = if g(74) > 0 { g(76) as f64 / 127.0 * 0.3 } else { 0.0 };
        note(220.0, max_hz, tail, 2.4)
    }

    fn cfg() -> Config {
        Config { ccs: (70..=77).collect(), low: 0, high: 127, mid: 64, group: 4, timing: T, baseline_reps: 2, group_reps: 2 }
    }

    #[test]
    fn the_passes_find_a_hidden_mapping_including_dependent_parameters() {
        let c = cfg();
        let mut log = Vec::new();
        let rep = run(&c, &mut |s| fake(s), &mut |m| log.push(m.to_string())).unwrap();
        let row = |cc: u8| rep.rows.iter().find(|r| r.cc == cc).unwrap();
        // pass 1: only the filter switch does anything alone (and with the cutoff at 0 it silences the note entirely)
        let alone: Vec<u8> = rep.rows.iter().filter(|r| r.add.changed).map(|r| r.cc).collect();
        assert_eq!(alone, vec![70]);
        assert!(row(70).add.text.contains("silenced"), "{}", row(70).add.text);
        // pass 2, filter group: the switch matters (filter off = brighter than cutoff 4 kHz); cutoff matters both ways
        assert!(row(70).down.changed);
        assert!(row(72).down.changed && row(72).down.text.contains("silenced"), "{}", row(72).down.text);
        assert!(row(72).up.as_ref().unwrap().text.contains("brighter"), "{}", row(72).up.as_ref().unwrap().text);
        for inert in [71, 73, 75, 77] {
            assert!(!row(inert).down.changed && !row(inert).up.as_ref().unwrap().changed, "CC {inert} should do nothing");
        }
        // pass 2, reverb group: the switch and the mix matter (tail), and the mix both ways
        assert!(row(74).down.changed && row(74).down.text.contains("tail"));
        assert!(row(76).down.changed && row(76).up.as_ref().unwrap().changed);
        assert!(row(74).up.is_none(), "a group's switch is already on at rest");
        let text = rep.render();
        assert!(text.contains("raised alone: 70"), "{text}");
        assert!(text.contains("within their group (down or up): 70 72 74 76"), "{text}");
        assert_eq!(log.len(), c.trial_count(), "progress reports every trial: {log:?}");
    }

    #[test]
    fn trial_count_matches_a_real_run_for_the_fm1_layout() {
        let c = Config { ccs: (70..=93).collect(), group: 4, ..cfg() };
        // pass 1: 2 + 24; pass 2: 6 groups x (2 baselines + switch down + 3 x (down, up)) = 6 x 9
        assert_eq!(c.trial_count(), 26 + 54);
    }

    #[test]
    fn a_group_that_is_silent_at_mid_falls_back_to_low_and_a_silent_one_is_skipped() {
        let c = Config { ccs: (70..=73).collect(), ..cfg() };
        // Q (73) at mid kills the sound; at 0 the sound is fine
        let plain = || note(220.0, 8000.0, 0.0, 2.4);
        let rep = run(&c, &mut |s| if s.get(&73) == Some(&64) { vec![0.0; 105_000] } else { plain() }, &mut |_| {}).unwrap();
        assert_eq!(rep.groups[0].rest, Some(0));
        assert!(rep.render().contains("others at 0"));
        // silent whenever the group's switch (70) is on, whatever the rest value: skipped; the run still finishes and pass 1 still reports
        let rep = run(&c, &mut |s| if s.get(&70) == Some(&127) { vec![0.0; 105_000] } else { plain() }, &mut |_| {}).unwrap();
        assert_eq!(rep.groups[0].rest, None);
        assert!(rep.rows.iter().all(|r| r.down.text.contains("skipped")));
        assert!(rep.render().contains("skipped"));
    }

    #[test]
    fn no_sound_at_the_baseline_is_an_error_not_an_empty_table() {
        let c = Config { ccs: vec![70], ..cfg() };
        let err = run(&c, &mut |_| vec![0.0; 105_000], &mut |_| {}).unwrap_err();
        assert!(err.contains("no sound"), "{err}");
    }
}
