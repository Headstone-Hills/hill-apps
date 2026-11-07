//! Measuring how fast an audio clock really runs, against the system clock.
//!
//! The FM-1 captures at its own crystal's idea of 44100 Hz and the SP plays at its sound card's idea of 44100 Hz. If the two
//! differ by even 50 ppm, a take recorded against the drums slides 3 ms per minute (180 ms in an hour). To know whether that
//! matters, each clock is compared with the system's monotonic clock: every so often the caller records
//! `(seconds since start, total frames so far)`, and a least-squares line through those points gives the real rate. A fit over
//! many jittery samples is far more precise than the jitter of any one of them (a callback can be late by 20 ms; the slope still
//! comes out to a fraction of a ppm over a few minutes).
//!
//! Pure arithmetic: the caller supplies the timestamps.

/// A least-squares fit of frames against seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// measured frames per second
    pub rate_hz: f64,
    /// difference from the nominal rate in parts per million (positive = the clock runs fast)
    pub ppm: f64,
    pub samples: usize,
    /// seconds between the first and last sample
    pub span_s: f64,
    /// RMS distance of the samples from the fitted line, in milliseconds (how jittery the timestamps were)
    pub jitter_ms: f64,
}

pub struct DriftEstimator {
    nominal_hz: f64,
    samples: Vec<(f64, f64)>,
    cap: usize,
    /// every `stride`-th pushed sample is kept
    stride: u64,
    pushed: u64,
}

/// Samples kept at most; beyond this the density is halved (the span is preserved), so a day-long run uses bounded memory
/// and never reallocates inside an audio callback.
const MAX_SAMPLES: usize = 4096;

impl DriftEstimator {
    pub fn new(nominal_hz: f64) -> Self {
        DriftEstimator { nominal_hz, samples: Vec::with_capacity(MAX_SAMPLES), cap: MAX_SAMPLES, stride: 1, pushed: 0 }
    }

    /// Record that `frames` frames had been delivered in total `t_s` seconds after the start.
    pub fn push(&mut self, t_s: f64, frames: u64) {
        self.pushed += 1;
        if !(self.pushed - 1).is_multiple_of(self.stride) {
            return;
        }
        if self.samples.len() == self.cap {
            // drop every second sample and keep half as many from now on
            let mut i = 0;
            self.samples.retain(|_| {
                i += 1;
                i % 2 == 1
            });
            self.stride *= 2;
            if !(self.pushed - 1).is_multiple_of(self.stride) {
                return;
            }
        }
        self.samples.push((t_s, frames as f64));
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The fit, once there are enough samples over enough time to mean something.
    pub fn fit(&self, min_span_s: f64) -> Option<Fit> {
        let n = self.samples.len();
        if n < 10 {
            return None;
        }
        let span = self.samples[n - 1].0 - self.samples[0].0;
        if span < min_span_s {
            return None;
        }
        let nf = n as f64;
        // centre the data: the plain formulas lose digits when t is small and frames are in the millions
        let (mt, mf) = (self.samples.iter().map(|s| s.0).sum::<f64>() / nf, self.samples.iter().map(|s| s.1).sum::<f64>() / nf);
        let (mut sxx, mut sxy) = (0.0, 0.0);
        for (t, f) in &self.samples {
            sxx += (t - mt) * (t - mt);
            sxy += (t - mt) * (f - mf);
        }
        if sxx <= 0.0 {
            return None;
        }
        let rate = sxy / sxx;
        let resid = (self.samples.iter().map(|(t, f)| (f - (mf + rate * (t - mt))).powi(2)).sum::<f64>() / nf).sqrt();
        Some(Fit { rate_hz: rate, ppm: (rate / self.nominal_hz - 1.0) * 1e6, samples: n, span_s: span, jitter_ms: resid / rate * 1000.0 })
    }
}

/// How much faster the capture clock runs than the output clock, in ppm. Positive: a take recorded against the drums is
/// longer than the drums that played it, so it slides later by this many microseconds every second.
pub fn relative_ppm(capture: &Fit, output: &Fit) -> f64 {
    (capture.rate_hz / output.rate_hz - 1.0) * 1e6
}

/// What a relative drift means in practice.
pub fn describe(ppm: f64) -> String {
    let per_min_ms = ppm.abs() * 60.0 / 1000.0;
    let per_10_min_ms = per_min_ms * 10.0;
    let verdict = if ppm.abs() < 20.0 {
        "negligible for songs of a few minutes"
    } else if ppm.abs() < 100.0 {
        "audible in a long take; worth correcting on long recordings"
    } else {
        "needs correction"
    };
    format!("{ppm:+.1} ppm = {per_min_ms:.1} ms per minute, {per_10_min_ms:.0} ms per 10 minutes ({verdict})")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake clock: true rate `rate`, timestamps late by a repeatable pseudo-random jitter of up to +-`jitter_s`.
    fn run(rate: f64, secs: f64, step_s: f64, jitter_s: f64) -> DriftEstimator {
        let mut e = DriftEstimator::new(44100.0);
        let mut seed = 12345u64;
        let mut t = 0.0;
        while t <= secs {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = ((seed >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 2.0 * jitter_s;
            e.push(t + j, (t * rate).round() as u64);
            t += step_s;
        }
        e
    }

    #[test]
    fn recovers_a_known_clock_error_through_heavy_timestamp_jitter() {
        // 50 ppm fast, every sample's timestamp wrong by up to 20 ms
        let e = run(44100.0 * (1.0 + 50e-6), 300.0, 0.1, 0.020);
        let f = e.fit(30.0).unwrap();
        assert!((f.ppm - 50.0).abs() < 2.0, "got {:.2} ppm", f.ppm);
        assert!(f.jitter_ms > 5.0 && f.jitter_ms < 20.0, "jitter {}", f.jitter_ms);
        assert_eq!(f.samples, 3001);
        assert!((f.span_s - 300.0).abs() < 0.1);
    }

    #[test]
    fn a_perfect_clock_reads_zero_and_a_slow_one_reads_negative() {
        assert!(run(44100.0, 120.0, 0.05, 0.0).fit(30.0).unwrap().ppm.abs() < 0.05);
        let slow = run(44100.0 * (1.0 - 120e-6), 200.0, 0.05, 0.005).fit(30.0).unwrap();
        assert!((slow.ppm + 120.0).abs() < 2.0, "{}", slow.ppm);
    }

    #[test]
    fn too_little_data_or_too_short_a_span_gives_no_answer() {
        let mut e = DriftEstimator::new(44100.0);
        assert!(e.fit(1.0).is_none());
        for i in 0..5 {
            e.push(i as f64, i * 44100);
        }
        assert!(e.fit(1.0).is_none(), "fewer than 10 samples");
        let short = run(44100.0, 5.0, 0.1, 0.0);
        assert!(short.fit(30.0).is_none(), "a 5 s span is not enough");
        assert!(short.fit(1.0).is_some());
    }

    #[test]
    fn a_long_run_keeps_bounded_memory_and_the_same_answer() {
        // 30 minutes sampled every 10 ms = 180000 pushes
        let e = run(44100.0 * (1.0 + 30e-6), 1800.0, 0.01, 0.010);
        assert!(e.len() <= MAX_SAMPLES, "{} samples", e.len());
        let f = e.fit(30.0).unwrap();
        assert!((f.ppm - 30.0).abs() < 1.0, "{}", f.ppm);
        assert!(f.span_s > 1700.0, "the whole span is still covered: {}", f.span_s);
    }

    #[test]
    fn relative_drift_between_two_clocks_and_its_plain_language() {
        let cap = run(44100.0 * (1.0 + 40e-6), 300.0, 0.1, 0.01).fit(30.0).unwrap();
        let out = run(44100.0 * (1.0 - 10e-6), 300.0, 0.1, 0.01).fit(30.0).unwrap();
        let r = relative_ppm(&cap, &out);
        assert!((r - 50.0).abs() < 2.0, "{r}");
        let d = describe(50.0);
        assert!(d.contains("3.0 ms per minute") && d.contains("30 ms per 10 minutes") && d.contains("worth correcting"), "{d}");
        assert!(describe(5.0).contains("negligible") && describe(-300.0).contains("needs correction"));
    }
}
