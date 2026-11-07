//! Tempo tools: tap tempo, BPM detection from audio, and sample-to-tempo fitting by resampling (varispeed;
//! time-stretching is out of scope, so fitting changes pitch along with speed).

use crate::wav::{resample_linear, Audio};

/// Tap tempo: BPM from the average of the last taps. Resets after a pause, like the Hill apps (6 taps, 3 minimum, 2 s gap).
#[derive(Default)]
pub struct TapTempo {
    taps: Vec<f64>,
}

impl TapTempo {
    pub const MAX_TAPS: usize = 6;
    pub const MIN_TAPS: usize = 3;
    pub const RESET_GAP_S: f64 = 2.0;

    /// Register a tap at time `t` (seconds). Returns the BPM once at least `MIN_TAPS` taps are in.
    pub fn tap(&mut self, t: f64) -> Option<f64> {
        if self.taps.last().is_some_and(|l| t - l > Self::RESET_GAP_S || t < *l) {
            self.taps.clear();
        }
        self.taps.push(t);
        if self.taps.len() > Self::MAX_TAPS {
            self.taps.remove(0);
        }
        if self.taps.len() < Self::MIN_TAPS {
            return None;
        }
        let span = self.taps[self.taps.len() - 1] - self.taps[0];
        Some((60.0 * (self.taps.len() - 1) as f64 / span).clamp(20.0, 300.0))
    }
}

/// Tempo score for every candidate BPM in `[min_bpm, max_bpm]` (0.1 BPM steps), plus the zero-lag energy
/// used to normalise. A steady pulse repeats at 2x and 4x its period, so those lags add to the score.
pub fn tempo_curve(audio: &[f32], rate: u32, min_bpm: f64, max_bpm: f64) -> Option<(Vec<(f64, f64)>, f64)> {
    const HOP: usize = 128;
    let frames = audio.len() / 2;
    if frames < rate as usize * 3 {
        return None; // too short to say anything
    }
    // short-time energy -> log -> half-wave-rectified difference = onset strength
    let win = HOP * 2;
    let n = (frames - win) / HOP;
    let mut env = Vec::with_capacity(n);
    for k in 0..n {
        let mut e = 0.0f64;
        for i in k * HOP..k * HOP + win {
            let m = ((audio[2 * i] + audio[2 * i + 1]) * 0.5) as f64;
            e += m * m;
        }
        env.push((1.0 + 1000.0 * e / win as f64).ln());
    }
    let mut onset: Vec<f64> = env.windows(2).map(|w| (w[1] - w[0]).max(0.0)).collect();
    let mean = onset.iter().sum::<f64>() / onset.len() as f64;
    for o in onset.iter_mut() {
        *o -= mean; // remove DC so the autocorrelation measures periodicity, not loudness
    }
    let fps = rate as f64 / HOP as f64;
    let ac = |lag: f64| -> f64 {
        let (i0, frac) = (lag.floor() as usize, lag - lag.floor());
        let m = onset.len().saturating_sub(i0 + 2);
        let mut s = 0.0;
        for i in 0..m {
            s += onset[i] * (onset[i + i0] * (1.0 - frac) + onset[i + i0 + 1] * frac);
        }
        s / m.max(1) as f64
    };
    let zero = ac(0.0).max(1e-12);
    let mut curve = Vec::new();
    let mut bpm = min_bpm;
    while bpm <= max_bpm {
        let lag = 60.0 / bpm * fps;
        curve.push((bpm, ac(lag) + 0.5 * ac(2.0 * lag) + 0.25 * ac(4.0 * lag)));
        bpm += 0.1;
    }
    Some((curve, zero))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub bpm: f64,
    /// 0..1, how strongly the audio repeats at this tempo (before the tempo prior)
    pub confidence: f64,
}

/// Mild perceptual prior: people tap near 115 BPM, so half/double-time ties break towards it.
fn prior(bpm: f64) -> f64 {
    let o = (bpm / 115.0).log2() / 0.9;
    (-0.5 * o * o).exp()
}

/// The `max_n` best tempo candidates, best first, at least 3 BPM apart. Octave alternatives (half / double
/// time) usually appear in the list: audio alone cannot always tell them apart, so a UI should offer them.
pub fn detect_candidates(audio: &[f32], rate: u32, min_bpm: f64, max_bpm: f64, max_n: usize) -> Vec<Candidate> {
    let Some((curve, zero)) = tempo_curve(audio, rate, min_bpm, max_bpm) else { return Vec::new() };
    let w = 15; // +/- 1.5 BPM neighbourhood for local maxima
    let mut peaks: Vec<(f64, f64, f64)> = Vec::new(); // (bpm, raw score, ranked score)
    for i in 0..curve.len() {
        let (lo, hi) = (i.saturating_sub(w), (i + w + 1).min(curve.len()));
        if curve[i].1 > 0.0 && curve[lo..hi].iter().all(|c| c.1 <= curve[i].1) {
            peaks.push((curve[i].0, curve[i].1, curve[i].1 * prior(curve[i].0)));
        }
    }
    peaks.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    let mut out: Vec<Candidate> = Vec::new();
    for (bpm, raw, _) in peaks {
        if out.iter().all(|c| (c.bpm - bpm).abs() >= 3.0) {
            out.push(Candidate { bpm, confidence: (raw / (1.75 * zero)).clamp(0.0, 1.0) });
        }
        if out.len() >= max_n {
            break;
        }
    }
    out
}

/// The single best guess (see [`detect_candidates`] for the octave caveat).
pub fn detect_bpm(audio: &[f32], rate: u32, min_bpm: f64, max_bpm: f64) -> Option<(f64, f64)> {
    detect_candidates(audio, rate, min_bpm, max_bpm, 1).first().map(|c| (c.bpm, c.confidence))
}

/// Half and double time of `bpm` that fall inside `[min_bpm, max_bpm]` (for the UI's x2 / /2 buttons).
pub fn octave_alternatives(bpm: f64, min_bpm: f64, max_bpm: f64) -> Vec<f64> {
    [bpm / 2.0, bpm * 2.0].into_iter().filter(|b| *b >= min_bpm && *b <= max_bpm).collect()
}

/// Playback-speed ratio that makes a loop recorded at `detected_bpm` run at `target_bpm`.
pub fn fit_ratio(detected_bpm: f64, target_bpm: f64) -> f64 {
    target_bpm / detected_bpm
}

/// Speed audio up (ratio > 1) or down by `ratio` keeping its declared sample rate (pitch follows speed).
pub fn varispeed(a: &Audio, ratio: f64) -> Audio {
    let r = resample_linear(a, ((a.rate as f64) / ratio).round() as u32);
    Audio { rate: a.rate, data: r.data }
}

/// Ratio that makes `frames` last a whole number of bars at `bpm` (the nearest whole number, at least 1).
pub fn fit_to_bars_ratio(frames: u64, rate: u32, bpm: f64, beats_per_bar: u32) -> (u32, f64) {
    let bar = rate as f64 * 60.0 / bpm * beats_per_bar as f64;
    let bars = ((frames as f64 / bar).round() as u32).max(1);
    (bars, frames as f64 / (bars as f64 * bar))
}

#[cfg(test)]
mod tests {
    use super::*;
    const R: u32 = 44100;

    /// a click on every beat (short decaying noise-ish burst) plus an off-beat hat at half amplitude
    fn drum_loop(bpm: f64, secs: f64) -> Vec<f32> {
        let n = (secs * R as f64) as usize;
        let mut a = vec![0.0f32; n * 2];
        let spb = R as f64 * 60.0 / bpm;
        let mut seed = 12345u32;
        let mut put = |at: usize, amp: f32| {
            for k in 0..400 {
                if at + k >= n {
                    break;
                }
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (seed >> 9) as f32 / 4194304.0 - 1.0;
                let v = amp * noise * (-(k as f32) / 90.0).exp();
                a[2 * (at + k)] += v;
                a[2 * (at + k) + 1] += v;
            }
        };
        let mut b = 0usize;
        while ((b as f64) * spb) < n as f64 {
            put((b as f64 * spb) as usize, 0.8);
            put(((b as f64 + 0.5) * spb) as usize, 0.3);
            b += 1;
        }
        a
    }

    #[test]
    fn the_true_tempo_is_always_a_top_candidate_and_the_pick_is_within_an_octave() {
        for bpm in [76.0, 90.0, 100.0, 120.0, 128.0, 140.0, 150.0, 170.0] {
            let audio = drum_loop(bpm, 12.0);
            let cands = detect_candidates(&audio, R, 70.0, 180.0, 3);
            assert!(cands.iter().any(|c| (c.bpm - bpm).abs() < 1.0), "{bpm}: candidates {cands:?}");
            let (pick, conf) = detect_bpm(&audio, R, 70.0, 180.0).unwrap();
            let octave = (pick - bpm).abs() < 1.0 || (pick - bpm * 2.0).abs() < 2.0 || (pick - bpm / 2.0).abs() < 1.0;
            assert!(octave, "{bpm}: picked {pick}");
            assert!(conf > 0.05);
        }
    }

    #[test]
    fn tempos_near_the_sweet_spot_are_picked_exactly() {
        for bpm in [100.0, 120.0, 128.0, 140.0] {
            let (pick, _) = detect_bpm(&drum_loop(bpm, 12.0), R, 70.0, 180.0).unwrap();
            assert!((pick - bpm).abs() < 1.0, "{bpm}: picked {pick}");
        }
    }

    #[test]
    fn octave_alternatives_respect_the_range() {
        assert_eq!(octave_alternatives(90.0, 70.0, 180.0), vec![180.0]);
        assert!(octave_alternatives(120.0, 70.0, 180.0).is_empty(), "60 and 240 are both out of range");
        assert_eq!(octave_alternatives(150.0, 70.0, 180.0), vec![75.0]);
    }

    #[test]
    fn silence_and_short_audio_give_no_answer() {
        assert!(detect_bpm(&vec![0.0; 2 * R as usize * 6], R, 70.0, 180.0).is_none());
        assert!(detect_bpm(&vec![0.5; 2 * 1000], R, 70.0, 180.0).is_none());
    }

    #[test]
    fn tap_tempo_averages_resets_and_clamps() {
        let mut t = TapTempo::default();
        assert_eq!(t.tap(0.0), None);
        assert_eq!(t.tap(0.5), None);
        let bpm = t.tap(1.0).unwrap();
        assert!((bpm - 120.0).abs() < 1e-9);
        // a wobbly tap barely moves a 6-tap average
        let b = t.tap(1.52).unwrap();
        assert!((b - 60.0 * 3.0 / 1.52).abs() < 1e-9);
        // 3 s of silence starts a fresh count
        assert_eq!(t.tap(4.5), None);
        t.tap(5.0);
        assert!((t.tap(5.4).unwrap() - 60.0 * 2.0 / 0.9).abs() < 1e-9); // 3 taps spanning 0.9 s
        // only the last 6 taps count
        let mut t = TapTempo::default();
        for i in 0..20 {
            t.tap(i as f64 * 0.4);
        }
        assert!((t.tap(8.0).unwrap() - 150.0).abs() < 1e-9);
    }

    #[test]
    fn fitting_changes_length_by_the_right_ratio() {
        let a = Audio { rate: R, data: vec![0.1; 2 * 44100] };
        let r = fit_ratio(100.0, 120.0);
        assert!((r - 1.2).abs() < 1e-12);
        let f = varispeed(&a, r);
        assert_eq!(f.rate, R);
        assert!((f.frames() as f64 - 44100.0 / 1.2).abs() < 2.0);
        let (bars, ratio) = fit_to_bars_ratio(44100 * 4, R, 120.0, 4);
        assert_eq!(bars, 2);
        assert!((ratio - 1.0).abs() < 1e-9);
        // a 4.3 s loop at 120 BPM (2 s per bar) is nearest to 2 bars: it must shrink by 4.3/4.0
        let (bars, ratio) = fit_to_bars_ratio((44100.0 * 4.3) as u64, R, 120.0, 4);
        assert_eq!(bars, 2);
        assert!((ratio - 1.075).abs() < 1e-4);
        assert_eq!(fit_to_bars_ratio(100, R, 120.0, 4).0, 1, "never fewer than one bar");
    }

    #[test]
    fn detected_tempo_then_fit_lands_on_the_target_tempo() {
        let looped = drum_loop(100.0, 12.0);
        let (bpm, _) = detect_bpm(&looped, R, 70.0, 180.0).unwrap();
        let fitted = varispeed(&Audio { rate: R, data: looped }, fit_ratio(bpm, 125.0));
        let (after, _) = detect_bpm(&fitted.data, R, 70.0, 180.0).unwrap();
        assert!((after - 125.0).abs() < 1.5, "{after}");
    }
}
