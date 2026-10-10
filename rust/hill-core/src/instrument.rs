use crate::wav::Sample;
use std::sync::Arc;

/// One or more samples, each tagged with the MIDI note it was recorded at.
#[derive(Clone)]
pub struct Instrument {
    pub name: String,
    pub samples: Vec<(u8, Arc<Sample>)>,
    /// Holds a steady level (pads, organs, looped sustains) instead of decaying.
    /// Sequencers gate these, or every step would start a multi-second drone.
    pub sustained: bool,
}

fn rms(d: &[f32]) -> f32 {
    if d.is_empty() {
        return 0.0;
    }
    (d.iter().map(|x| x * x).sum::<f32>() / d.len() as f32).sqrt()
}

/// Mid-sample level vs the start: a sound that is still more than half as loud
/// at 40-60% of its length as in its first fifth is sustained.
fn is_sustained(s: &Sample) -> Option<bool> {
    if s.seconds() < 0.5 {
        return None;
    }
    let n = s.frames();
    let early = rms(&s.data[..n / 5 * 2]);
    let mid = rms(&s.data[n * 2 / 5 * 2..n * 3 / 5 * 2]);
    Some(early > 1e-4 && mid > 0.5 * early)
}

impl Instrument {
    pub fn new(name: &str, samples: Vec<(u8, Arc<Sample>)>) -> Self {
        let votes: Vec<bool> = samples.iter().filter_map(|(_, s)| is_sustained(s)).collect();
        let sustained = !votes.is_empty() && votes.iter().filter(|v| **v).count() * 2 > votes.len();
        Instrument { name: name.to_string(), samples, sustained }
    }

    pub fn single(name: &str, root: u8, s: Arc<Sample>) -> Self {
        Self::new(name, vec![(root, s)])
    }

    /// Pick the sample nearest to `midi`; returns it with the playback-rate
    /// ratio needed to reach that pitch.
    pub fn pick(&self, midi: u8) -> Option<(&Arc<Sample>, f64)> {
        let (root, s) = self
            .samples
            .iter()
            .min_by_key(|(r, _)| (*r as i32 - midi as i32).abs())?;
        Some((s, 2f64.powf((midi as f64 - *root as f64) / 12.0)))
    }

    /// The first sample, unshifted (drum use).
    pub fn first(&self) -> Option<&Arc<Sample>> {
        self.samples.first().map(|(_, s)| s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Arc<Sample> {
        Arc::new(Sample { data: vec![0.0; 8], rate: 44100 })
    }

    #[test]
    fn sustained_detection() {
        let tone = |f: &dyn Fn(usize) -> f32| {
            let d: Vec<f32> = (0..44100 * 2).flat_map(|i| [f(i), f(i)]).collect();
            Arc::new(Sample { data: d, rate: 44100 })
        };
        let pad = Instrument::single("pad", 60, tone(&|i| (i as f32 * 0.05).sin() * 0.5));
        let pluck = Instrument::single("pluck", 60, tone(&|i| (i as f32 * 0.05).sin() * (-(i as f32) / 6000.0).exp()));
        assert!(pad.sustained);
        assert!(!pluck.sustained);
        assert!(!Instrument::single("hit", 60, s()).sustained, "short samples are never sustained");
    }

    #[test]
    fn nearest_and_ratio() {
        let i = Instrument::new("x", vec![(48, s()), (60, s()), (72, s())]);
        let (_, r) = i.pick(60).unwrap();
        assert!((r - 1.0).abs() < 1e-9);
        let (_, r) = i.pick(66).unwrap(); // tie 60/72 -> first min is 60
        assert!((r - 2f64.powf(0.5)).abs() < 1e-6);
        let (_, r) = i.pick(84).unwrap();
        assert!((r - 2.0).abs() < 1e-9);
    }
}
