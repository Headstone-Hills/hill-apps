//! Mixdown and stem export, the pure parts: render a mixer offline, convert float audio to 16/24-bit PCM with optional dither,
//! measure the peak and normalize. No files and no threads here: the app feeds the rendered chunks to a writer on a background
//! thread (`hilldaw-app`'s `export` module).
//!
//! The mixer renders from absolute frame numbers, so an offline render is bit-identical to what the audio callback produces for the
//! same frames, whatever chunk size is used (tested).

use crate::mixer::Mixer;

/// 16-bit dither: triangular (TPDF) noise of +-1 LSB, from a small deterministic generator (the same file every time).
pub struct Dither {
    state: u64,
}

impl Dither {
    pub fn new(seed: u64) -> Self {
        Dither { state: seed.max(1) }
    }

    fn next_u32(&mut self) -> u32 {
        // xorshift64*
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        (x.wrapping_mul(0x2545F4914F6CDD1D) >> 32) as u32
    }

    /// One dither value in LSBs, in (-1, 1), triangular.
    pub fn next_lsb(&mut self) -> f32 {
        let a = self.next_u32() as f32 / 4_294_967_296.0;
        let b = self.next_u32() as f32 / 4_294_967_296.0;
        a - b
    }
}

/// What a block of samples looked like after gain: its peak and how many samples went past full scale (and so were clipped).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Levels {
    pub peak: f32,
    pub clipped: u64,
}

impl Levels {
    pub fn merge(&mut self, o: Levels) {
        self.peak = self.peak.max(o.peak);
        self.clipped += o.clipped;
    }
}

/// Peak of interleaved samples.
pub fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, s| m.max(s.abs()))
}

/// Gain that brings a signal with the given peak to `target_db` dBFS (1.0 for silence).
pub fn normalize_gain(peak: f32, target_db: f32) -> f32 {
    if peak <= 1e-9 { 1.0 } else { 10f32.powf(target_db / 20.0) / peak }
}

pub fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// Append `samples * gain` as little-endian PCM (`bits` = 16 or 24). Dither applies to 16-bit only (a 24-bit quantisation step is
/// below any real noise floor). Out-of-range values are clamped and counted.
pub fn encode_pcm(samples: &[f32], bits: u16, gain: f32, mut dither: Option<&mut Dither>, out: &mut Vec<u8>) -> Levels {
    assert!(bits == 16 || bits == 24, "16 or 24 bits");
    let mut lv = Levels::default();
    for s in samples {
        let x = s * gain;
        lv.peak = lv.peak.max(x.abs());
        if x.abs() > 1.0 {
            lv.clipped += 1;
        }
        if bits == 16 {
            let d = dither.as_mut().map_or(0.0, |d| d.next_lsb());
            let v = (x.clamp(-1.0, 1.0) * 32767.0 + d).round().clamp(-32768.0, 32767.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        } else {
            let v = (x.clamp(-1.0, 1.0) * 8388607.0).round() as i32;
            out.extend_from_slice(&v.to_le_bytes()[..3]);
        }
    }
    lv
}

/// Render `frames` frames of `mixer` from frame 0 in chunks of `chunk` frames, handing each interleaved stereo chunk to `on_chunk`.
/// Returns false if `on_chunk` asked to stop (cancel) before the end.
pub fn render_offline(mixer: &mut Mixer, frames: u64, chunk: usize, mut on_chunk: impl FnMut(&[f32]) -> bool) -> bool {
    let chunk = chunk.max(1);
    let mut buf = vec![0.0f32; chunk * 2];
    let mut at = 0u64;
    while at < frames {
        let n = chunk.min((frames - at) as usize);
        mixer.render(at, &mut buf[..n * 2]);
        if !on_chunk(&buf[..n * 2]) {
            return false;
        }
        at += n as u64;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixer::{Click, Tone, Track};

    fn mixer_with_sound() -> Mixer {
        let mut m = Mixer::new(44100, 4096);
        let mut t = Track::new("T");
        t.sources.push(Box::new(Tone { freq: 440.0, amp: 0.4 }));
        t.sources.push(Box::new(Click { bpm: 120.0, beats_per_bar: 4, amp: 0.5, enabled: true }));
        t.pan = -0.3;
        m.tracks.push(t);
        m
    }

    fn all(m: &mut Mixer, frames: u64, chunk: usize) -> Vec<f32> {
        let mut v = Vec::new();
        assert!(render_offline(m, frames, chunk, |c| {
            v.extend_from_slice(c);
            true
        }));
        v
    }

    #[test]
    fn an_offline_render_is_identical_for_any_chunk_size() {
        let whole = all(&mut mixer_with_sound(), 50_000, 50_000);
        assert!(peak(&whole) > 0.1);
        for chunk in [1usize, 333, 1024, 4096] {
            assert!(all(&mut mixer_with_sound(), 50_000, chunk) == whole, "chunk {chunk}");
        }
    }

    #[test]
    fn rendering_can_be_cancelled_part_way() {
        let mut n = 0;
        let done = render_offline(&mut mixer_with_sound(), 100_000, 1000, |_| {
            n += 1;
            n < 10
        });
        assert!(!done && n == 10);
        assert_eq!(all(&mut mixer_with_sound(), 0, 1000).len(), 0, "zero frames renders nothing");
    }

    #[test]
    fn pcm_conversion_is_exact_at_known_values_and_counts_clipping() {
        let mut out = Vec::new();
        let lv = encode_pcm(&[0.0, 0.5, -0.5, 1.0, -1.0, 1.5], 16, 1.0, None, &mut out);
        let v: Vec<i16> = out.chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(v, vec![0, 16384, -16384, 32767, -32767, 32767], "0.5 * 32767 rounds to 16384; 1.5 clamps");
        assert_eq!((lv.peak, lv.clipped), (1.5, 1));
        let mut out = Vec::new();
        encode_pcm(&[0.25, -0.25, 1.0], 24, 2.0, None, &mut out);
        let v: Vec<i32> = out.chunks(3).map(|b| (((b[2] as i8) as i32) << 16) | ((b[1] as i32) << 8) | b[0] as i32).collect();
        assert_eq!(v, vec![4194304, -4194304, 8388607], "gain 2 doubles, full scale clamps");
    }

    #[test]
    fn a_24_bit_export_reads_back_as_the_same_audio() {
        let a: Vec<f32> = (0..2000).map(|i| (i as f32 * 0.013).sin() * 0.7).collect();
        let mut out = Vec::new();
        encode_pcm(&a, 24, 1.0, None, &mut out);
        let back: Vec<f32> = out.chunks(3).map(|b| ((((b[2] as i8) as i32) << 16) | ((b[1] as i32) << 8) | b[0] as i32) as f32 / 8388608.0).collect();
        let worst = a.iter().zip(&back).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
        assert!(worst < 2e-7, "{worst}");
    }

    #[test]
    fn dither_is_triangular_zero_mean_and_deterministic() {
        let mut d = Dither::new(7);
        let v: Vec<f32> = (0..200_000).map(|_| d.next_lsb()).collect();
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        assert!(mean.abs() < 0.01, "mean {mean}");
        assert!(v.iter().all(|x| x.abs() < 1.0), "within one LSB");
        // triangular: variance of the difference of two uniforms is 1/6
        let var = v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32;
        assert!((var - 1.0 / 6.0).abs() < 0.01, "variance {var}");
        let (mut a, mut b) = (Dither::new(7), Dither::new(7));
        assert!((0..100).all(|_| a.next_lsb() == b.next_lsb()));
    }

    #[test]
    fn dither_keeps_a_signal_below_one_lsb_alive_on_average() {
        // a constant 0.3 LSB: without dither it rounds to 0 every time; with dither the average comes back
        let x = vec![0.3 / 32767.0; 100_000];
        let mean = |dither: Option<&mut Dither>| {
            let mut out = Vec::new();
            encode_pcm(&x, 16, 1.0, dither, &mut out);
            out.chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32).sum::<f32>() / x.len() as f32
        };
        assert_eq!(mean(None), 0.0);
        let m = mean(Some(&mut Dither::new(3)));
        assert!((m - 0.3).abs() < 0.03, "dithered average {m}");
    }

    #[test]
    fn normalization_brings_the_peak_to_the_target() {
        assert!((db(normalize_gain(0.25, -1.0) * 0.25) + 1.0).abs() < 1e-4);
        assert_eq!(normalize_gain(0.0, -1.0), 1.0, "silence is left alone");
        let mut out = Vec::new();
        let g = normalize_gain(0.2, -1.0);
        let lv = encode_pcm(&[0.2, -0.1], 24, g, None, &mut out);
        assert!((db(lv.peak) + 1.0).abs() < 1e-4 && lv.clipped == 0);
    }
}
