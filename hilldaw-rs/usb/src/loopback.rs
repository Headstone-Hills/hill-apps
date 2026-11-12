//! Measuring how much of what we play into a device comes back in what we capture from it (the FM-1+VA mixes its USB input into its USB
//! output, so a recording made while monitoring through it also holds the other tracks).
//!
//! The model is `captured[n] = gain * sent[n + lag] + rest`: a delay and a gain, found by cross-correlation over a window.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loopback {
    /// frames: `captured[n]` matches `sent[n + lag]`
    pub lag: i64,
    /// how much of the sent signal is in the capture (1.0 = at full level)
    pub gain: f32,
    /// correlation at that lag, 0..1 (about 0 = no loopback)
    pub corr: f32,
    /// the capture's energy left after taking the loopback out, in dB relative to before (0 = nothing removed, -60 = all but gone)
    pub residual_db: f32,
}

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

/// Find the lag in `lags` (frames) at which `sent` best explains the window `captured[start..start + len]`. `None` if there is
/// not enough data or either side is silent.
pub fn estimate(sent: &[f32], captured: &[f32], start: usize, len: usize, lags: std::ops::RangeInclusive<i64>) -> Option<Loopback> {
    if start + len > captured.len() {
        return None;
    }
    let y = &captured[start..start + len];
    let yy = dot(y, y);
    if yy < 1e-9 {
        return None;
    }
    let mut best: Option<(f64, i64, f64, f64)> = None; // (|corr|, lag, xy, xx)
    for lag in lags {
        let from = start as i64 + lag;
        if from < 0 || from as usize + len > sent.len() {
            continue;
        }
        let x = &sent[from as usize..from as usize + len];
        let (xy, xx) = (dot(x, y), dot(x, x));
        if xx < 1e-9 {
            continue;
        }
        let c = xy.abs() / (xx * yy).sqrt();
        if best.is_none_or(|b| c > b.0) {
            best = Some((c, lag, xy, xx));
        }
    }
    let (corr, lag, xy, xx) = best?;
    let gain = xy / xx;
    // energy of y - gain * x
    let residual = (yy - xy * xy / xx).max(0.0);
    Some(Loopback { lag, gain: gain as f32, corr: corr as f32, residual_db: 10.0 * ((residual / yy).max(1e-12)).log10() as f32 })
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

    #[test]
    fn finds_the_delay_and_gain_of_a_leak_mixed_with_other_sound() {
        let sent = noise(30_000, 1);
        let own = noise(30_000, 2); // what is played live on the unit: not in `sent`
        // captured[n] = 0.5 * sent[n + 1234] + 0.3 * own[n]
        let captured: Vec<f32> = (0..20_000).map(|n| 0.5 * sent[n + 1234] + 0.3 * own[n]).collect();
        let r = estimate(&sent, &captured, 2000, 8000, -500..=3000).unwrap();
        assert_eq!(r.lag, 1234);
        assert!((r.gain - 0.5).abs() < 0.03, "gain {}", r.gain);
        assert!(r.corr > 0.8);
        // the live part is about 0.09 / (0.25 + 0.09) of the energy: -5.8 dB left when the leak is gone
        assert!((r.residual_db + 5.8).abs() < 1.0, "residual {}", r.residual_db);
    }

    #[test]
    fn no_leak_means_no_correlation() {
        let sent = noise(30_000, 1);
        let captured = noise(20_000, 9);
        let r = estimate(&sent, &captured, 2000, 8000, 0..=3000).unwrap();
        assert!(r.corr < 0.1, "corr {}", r.corr);
        assert!(r.residual_db > -0.5);
    }

    #[test]
    fn silence_or_missing_data_is_none() {
        assert!(estimate(&noise(100, 1), &vec![0.0; 1000], 0, 500, 0..=10).is_none());
        assert!(estimate(&noise(100, 1), &noise(100, 2), 50, 500, 0..=10).is_none());
    }
}
