//! Taking an audio device's own echo out of a recording.
//!
//! Some devices (the FM-1+VA) mix what they are *given* to play back into what they *send*: record a new part through one while
//! it plays the other tracks into your earbuds, and the recording holds those tracks too. We know exactly what was played into the
//! device (the "reference", counted in the device's own frames), so the echo can be found and subtracted:
//!
//! ```text
//! captured[c] = wanted[c] + gain * reference[c + lag]
//! ```
//!
//! The lag (the device's round trip) is found by cross-correlation, coarsely on a decimated copy and then exactly; because the two
//! streams are only a few ppm apart it is then tracked segment by segment (a drift of one frame in 20 s would otherwise spoil the highs)
//! and subtracted with a fractional delay.

/// What was found and done.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EchoReport {
    /// the echo's level relative to the reference (1.0 = as loud as what was played)
    pub gain: f32,
    /// the round trip in device frames (positive: the reference is ahead of the capture by this much)
    pub lag: f32,
    /// correlation of the best match, 0..1
    pub corr: f32,
    /// the capture's energy after / before, in dB (0 = nothing taken out)
    pub removed_db: f32,
}

const DEC: usize = 8;
/// frames per tracking segment (about 0.75 s)
const SEG: usize = 32_768;
/// the most the lag may move from one segment to the next, in frames
const TRACK: i64 = 6;
/// fractional delay table: 1/FRAC_STEPS of a frame, 8 taps
const FRAC_STEPS: usize = 32;
const TAPS: usize = 8;

fn dot(a: &[f32], b: &[f32]) -> f64 {
    let mut acc = [0.0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    acc.iter().map(|v| *v as f64).sum::<f64>() + ra.iter().zip(rb).map(|(x, y)| (*x as f64) * (*y as f64)).sum::<f64>()
}

fn decimate(v: &[f32]) -> Vec<f32> {
    v.chunks_exact(DEC).map(|c| c.iter().sum::<f32>() / DEC as f32).collect()
}

/// Correlation of `y[a..b]` with `x` shifted by `lag` (`x[a + lag..b + lag]`): (x.y, x.x, y.y), `None` if the shifted range is not inside `x`.
fn corr_at(y: &[f32], x: &[f32], a: usize, b: usize, lag: i64) -> Option<(f64, f64, f64)> {
    let (xa, xb) = (a as i64 + lag, b as i64 + lag);
    if xa < 0 || xb as usize > x.len() || b > y.len() || a >= b {
        return None;
    }
    let (xs, ys) = (&x[xa as usize..xb as usize], &y[a..b]);
    Some((dot(xs, ys), dot(xs, xs), dot(ys, ys)))
}

/// The best integer lag in `lags` for the stretches `spans` of `y` taken together (several stretches at different times keep a sustained
/// note, which matches itself at many lags, from fooling the search), with a fractional refinement and the figures at that lag:
/// (lag, gain, corr, x.x).
fn best_lag(y: &[f32], x: &[f32], spans: &[(usize, usize)], lags: std::ops::RangeInclusive<i64>) -> Option<(f64, f64, f64, f64)> {
    let mut scores: Vec<(i64, f64, f64, f64)> = Vec::new(); // (lag, |corr|, x.y, x.x)
    for lag in lags {
        let (mut sxy, mut sxx, mut syy) = (0.0f64, 0.0f64, 0.0f64);
        let mut any = false;
        for &(a, b) in spans {
            if let Some((xy, xx, yy)) = corr_at(y, x, a, b, lag) {
                (sxy, sxx, syy) = (sxy + xy, sxx + xx, syy + yy);
                any = true;
            }
        }
        if any && sxx > 1e-9 && syy > 1e-9 {
            scores.push((lag, sxy.abs() / (sxx * syy).sqrt(), sxy, sxx));
        }
    }
    let k = (0..scores.len()).max_by(|&i, &j| scores[i].1.total_cmp(&scores[j].1))?;
    let (lag, c, xy, xx) = scores[k];
    // a parabola through the peak and its two neighbours gives the lag to a fraction of a frame
    let frac = if k > 0 && k + 1 < scores.len() {
        let (l, r) = (scores[k - 1].1, scores[k + 1].1);
        let d = l - 2.0 * c + r;
        if d < -1e-12 { (0.5 * (l - r) / d).clamp(-0.5, 0.5) } else { 0.0 }
    } else {
        0.0
    };
    Some((lag as f64 + frac, xy / xx, c, xx))
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) }
}

/// 8-tap Lanczos weights for every 1/32 of a frame: row `f` gives the weights for taps at offsets -3..=4 around `floor(pos)`.
fn frac_table() -> Vec<[f32; TAPS]> {
    (0..=FRAC_STEPS)
        .map(|f| {
            let frac = f as f64 / FRAC_STEPS as f64;
            let mut w = [0.0f32; TAPS];
            for (k, wk) in w.iter_mut().enumerate() {
                let d = (k as f64 - 3.0) - frac; // tap offset minus the fractional position
                *wk = (sinc(d) * sinc(d / 4.0)) as f32;
            }
            let sum: f32 = w.iter().sum();
            w.iter_mut().for_each(|v| *v /= sum);
            w
        })
        .collect()
}

/// Remove the echo of `reference` from `captured` in place. Both are mono at the same rate. `search` is how far, in frames, the lag may be
/// from zero in either direction (the two streams are not started at the same instant). `None` (and `captured` untouched) if no echo is found.
pub fn cancel(captured: &mut [f32], reference: &[f32], search: usize) -> Option<EchoReport> {
    let (yd, xd) = (decimate(captured), decimate(reference));
    let w = 4096.min(yd.len() / 2);
    if w < 1024 || xd.len() < w {
        return None;
    }
    // the stretches of the capture where the reference is loudest, spread out in time: the most to go on
    let mut cands: Vec<(f64, usize)> = Vec::new();
    let mut s = 0;
    while s + w <= yd.len().min(xd.len()) {
        cands.push((dot(&xd[s..s + w], &xd[s..s + w]), s));
        s += w / 2;
    }
    cands.sort_by(|p, q| q.0.total_cmp(&p.0));
    let mut starts: Vec<usize> = Vec::new();
    for (e, s) in cands {
        if e > 1e-6 && starts.len() < 4 && starts.iter().all(|t| t.abs_diff(s) >= w) {
            starts.push(s);
        }
    }
    if starts.is_empty() {
        return None;
    }
    let sd = (search / DEC) as i64;
    let coarse_spans: Vec<(usize, usize)> = starts.iter().map(|&s| (s, s + w)).collect();
    let (coarse, _, c0, _) = best_lag(&yd, &xd, &coarse_spans, -sd..=sd)?;
    if c0 < 0.06 {
        return None;
    }
    let spans: Vec<(usize, usize)> = starts.iter().map(|&s| (s * DEC, ((s + w) * DEC).min(captured.len()))).collect();
    let centre = (coarse * DEC as f64).round() as i64;
    let (lag0, _, corr, _) = best_lag(captured, reference, &spans, centre - DEC as i64 - 4..=centre + DEC as i64 + 4)?;

    // follow the lag and gain segment by segment
    let n = captured.len();
    let nseg = n.div_ceil(SEG);
    let mut pts: Vec<(f64, f64, f64)> = Vec::with_capacity(nseg); // (segment centre, lag, gain)
    let (mut lag, mut gain) = (lag0, 0.0f64);
    for k in 0..nseg {
        let (sa, sb) = (k * SEG, ((k + 1) * SEG).min(n));
        let c = lag.round() as i64;
        if let Some((l, g, cc, xx)) = best_lag(captured, reference, &[(sa, sb)], c - TRACK..=c + TRACK) {
            // a quiet reference or a weak match says nothing new: keep what the last segment found
            if xx > 1e-3 * (sb - sa) as f64 * 1e-3 && cc > 0.02 {
                (lag, gain) = (l, g.clamp(-1.5, 1.5));
            }
        }
        pts.push(((sa + sb) as f64 / 2.0, lag, gain));
    }
    // a first segment that found nothing takes the figures of the first one that did
    let first_good = pts.iter().find(|p| p.2 != 0.0).copied()?;
    for p in pts.iter_mut().take_while(|p| p.2 == 0.0) {
        p.1 = first_good.1;
        p.2 = first_good.2;
    }

    let before: f64 = dot(captured, captured);
    let table = frac_table();
    let mut seg = 0usize;
    for (c, out) in captured.iter_mut().enumerate() {
        let t = c as f64;
        while seg + 1 < pts.len() && t > pts[seg + 1].0 {
            seg += 1;
        }
        let (l, g) = if seg + 1 < pts.len() && t >= pts[seg].0 {
            let (p, q) = (pts[seg], pts[seg + 1]);
            let u = (t - p.0) / (q.0 - p.0);
            (p.1 + (q.1 - p.1) * u, p.2 + (q.2 - p.2) * u)
        } else {
            (pts[seg].1, pts[seg].2)
        };
        let pos = c as f64 + l;
        let i0 = pos.floor() as i64;
        let f = ((pos - i0 as f64) * FRAC_STEPS as f64).round() as usize;
        let wts = &table[f.min(FRAC_STEPS)];
        let mut v = 0.0f32;
        for (k, wk) in wts.iter().enumerate() {
            let i = i0 + k as i64 - 3;
            if i >= 0 && (i as usize) < reference.len() {
                v += reference[i as usize] * wk;
            }
        }
        *out = (*out as f64 - g * v as f64) as f32;
    }
    let after: f64 = dot(captured, captured);
    let mid = pts[pts.len() / 2];
    Some(EchoReport { gain: mid.2 as f32, lag: mid.1 as f32, corr: corr as f32, removed_db: 10.0 * (after.max(1e-14) / before.max(1e-14)).log10() as f32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 8) as f32 / (1 << 23) as f32 - 1.0
            })
            .collect()
    }

    /// something music-like: tones that change every half second plus noise bursts
    fn music(n: usize, seed: u32) -> Vec<f32> {
        let nz = noise(n, seed);
        (0..n)
            .map(|i| {
                let note = 220.0 * 2f32.powf(((i / 22_050) * 7 % 12) as f32 / 12.0);
                let t = i as f32 / 44_100.0;
                0.4 * (std::f32::consts::TAU * note * t).sin() + 0.2 * (std::f32::consts::TAU * note * 2.01 * t).sin() + 0.15 * nz[i] * (((i / 5_512) % 2) as f32)
            })
            .collect()
    }

    fn energy(v: &[f32]) -> f64 {
        v.iter().map(|x| (*x as f64).powi(2)).sum()
    }

    /// the reference sample at a fractional position (for building a test echo with a lag that is not a whole number of frames)
    fn at(x: &[f32], pos: f64) -> f32 {
        let i = pos.floor() as i64;
        let f = (pos - i as f64) as f32;
        let g = |k: i64| if k >= 0 && (k as usize) < x.len() { x[k as usize] } else { 0.0 };
        // cubic Hermite
        let (p0, p1, p2, p3) = (g(i - 1), g(i), g(i + 1), g(i + 2));
        p1 + 0.5 * f * (p2 - p0 + f * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + f * (3.0 * (p1 - p2) + p3 - p0)))
    }

    #[test]
    fn removes_a_delayed_echo_and_keeps_the_wanted_part() {
        let n = 441_000; // 10 s
        let reference = music(n + 20_000, 7);
        let wanted: Vec<f32> = noise(n, 3).iter().enumerate().map(|(i, v)| 0.2 * v + 0.1 * (i as f32 * 0.05).sin()).collect();
        // the echo arrives 4321.4 frames late and drifts a frame over the take, at 0.75 of the level
        let mut y: Vec<f32> = (0..n).map(|c| wanted[c] + 0.75 * at(&reference, c as f64 + 4321.4 + c as f64 / 441_000.0)).collect();
        let before = energy(&y);
        let r = cancel(&mut y, &reference, 30_000).expect("the echo is found");
        assert!((r.lag - 4321.9).abs() < 1.0, "lag {}", r.lag);
        assert!((r.gain - 0.75).abs() < 0.05, "gain {}", r.gain);
        let err: Vec<f32> = y.iter().zip(&wanted).map(|(a, b)| a - b).collect();
        let before_err = energy(&wanted) * 0.0 + (before - energy(&wanted));
        let rejection = 10.0 * (energy(&err) / before_err.max(1.0)).log10();
        assert!(rejection < -20.0, "the echo came down by only {rejection:.1} dB");
        assert!(r.removed_db < -1.0);
    }

    #[test]
    fn a_wanted_part_with_no_echo_is_left_alone() {
        let n = 200_000;
        let reference = music(n, 7);
        let wanted = noise(n, 3);
        let mut y = wanted.clone();
        assert!(cancel(&mut y, &reference, 30_000).is_none());
        assert_eq!(y, wanted, "untouched");
    }

    #[test]
    fn silence_in_the_reference_or_a_short_take_does_nothing() {
        let mut y = noise(200_000, 3);
        let orig = y.clone();
        assert!(cancel(&mut y, &vec![0.0; 200_000], 30_000).is_none());
        let mut short = noise(5_000, 3);
        assert!(cancel(&mut short, &music(5_000, 1), 30_000).is_none());
        assert_eq!(y, orig);
    }

    #[test]
    fn the_echo_can_lead_or_trail_and_a_reference_that_ends_early_is_fine() {
        let n = 200_000;
        for lag in [-2500i64, 0, 7000] {
            let reference = music(n - 30_000, 5); // shorter than the capture
            let wanted = noise(n, 4);
            let mut y: Vec<f32> = (0..n).map(|c| wanted[c] * 0.3 + 0.8 * at(&reference, (c as i64 + lag) as f64)).collect();
            let r = cancel(&mut y, &reference, 30_000).unwrap_or_else(|| panic!("lag {lag}: not found"));
            assert!((r.lag - lag as f32).abs() < 1.0, "lag {lag}: found {}", r.lag);
            // where the reference exists the echo is gone
            let e: f64 = y[..n - 40_000].iter().zip(&wanted).map(|(a, b)| (*a as f64 - 0.3 * *b as f64).powi(2)).sum();
            let w: f64 = wanted[..n - 40_000].iter().map(|b| (0.3 * *b as f64).powi(2)).sum();
            assert!(10.0 * (e / w).log10() < -20.0, "lag {lag}");
        }
    }
}
