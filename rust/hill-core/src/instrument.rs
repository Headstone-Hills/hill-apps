use crate::wav::Sample;
use std::sync::Arc;

/// One or more samples, each tagged with the MIDI note it was recorded at.
#[derive(Clone)]
pub struct Instrument {
    pub name: String,
    pub samples: Vec<(u8, Arc<Sample>)>,
}

impl Instrument {
    pub fn single(name: &str, root: u8, s: Arc<Sample>) -> Self {
        Instrument { name: name.to_string(), samples: vec![(root, s)] }
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
    fn nearest_and_ratio() {
        let i = Instrument {
            name: "x".into(),
            samples: vec![(48, s()), (60, s()), (72, s())],
        };
        let (_, r) = i.pick(60).unwrap();
        assert!((r - 1.0).abs() < 1e-9);
        let (_, r) = i.pick(66).unwrap(); // tie 60/72 -> first min is 60
        assert!((r - 2f64.powf(0.5)).abs() < 1e-6);
        let (_, r) = i.pick(84).unwrap();
        assert!((r - 2.0).abs() < 1e-9);
    }
}
