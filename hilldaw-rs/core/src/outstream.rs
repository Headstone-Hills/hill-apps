//! Playing to an audio device that runs on its own clock: a streaming resampler whose pace can be nudged, and the encoder that turns
//! float audio into the device's sample format.
//!
//! HillDAW renders in blocks driven by the sound card's clock, but a USB device consumes audio on the USB bus clock. Two clocks never
//! agree exactly (the SP's own sound card is about 42 ppm fast against its system clock, measured), so a buffer between them slowly fills
//! or empties. The resampler therefore reads its input at a rate that is adjusted a hair (at most 0.5%, inaudible) according to how full
//! the buffer is: the same mechanism also converts 44.1 kHz to a 48 kHz device.

/// Input frames consumed per output frame, adjusted so a buffer stays near `target` frames: a fuller buffer is read a little faster.
/// `base` is the nominal ratio (input rate / output rate). The correction is proportional and limited to +-0.5%.
pub fn adaptive_step(base: f64, fill: usize, target: usize) -> f64 {
    let target = target.max(1) as f64;
    let err = (fill as f64 - target) / target; // +1 = twice as full as wanted
    base * (1.0 + (err * 0.005).clamp(-0.005, 0.005))
}

/// A pull-based 4-point Hermite resampler for interleaved stereo. Frames come from a closure so the caller can feed it from a ring buffer;
/// when the closure has nothing, [`StreamResampler::pull`] returns `None` and picks up where it was once more input arrives.
#[derive(Default)]
pub struct StreamResampler {
    y: [[f32; 2]; 4],
    filled: usize,
    t: f64,
}

fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, f: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * f + c2) * f + c1) * f + y1
}

impl StreamResampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next output frame. `step` = input frames per output frame (1.0 = same rate).
    pub fn pull(&mut self, step: f64, src: &mut dyn FnMut() -> Option<[f32; 2]>) -> Option<[f32; 2]> {
        while self.filled < 4 {
            self.y[self.filled] = src()?;
            self.filled += 1;
        }
        while self.t >= 1.0 {
            let f = src()?;
            self.y = [self.y[1], self.y[2], self.y[3], f];
            self.t -= 1.0;
        }
        let f = self.t as f32;
        let out = [hermite(self.y[0][0], self.y[1][0], self.y[2][0], self.y[3][0], f), hermite(self.y[0][1], self.y[1][1], self.y[2][1], self.y[3][1], f)];
        self.t += step;
        Some(out)
    }
}

/// Write one frame in a device's format: `channels` samples of `subframe` bytes (2, 3 or 4, signed little-endian). Mono gets the average of
/// left and right, stereo gets left and right, more channels get left and right and silence. Values are clamped to full scale.
/// `out` must be `channels * subframe` bytes.
pub fn encode_frame(l: f32, r: f32, channels: u8, subframe: u8, out: &mut [u8]) {
    let sub = subframe as usize;
    for c in 0..channels as usize {
        let v = match (channels, c) {
            (1, _) => (l + r) * 0.5,
            (_, 0) => l,
            (_, 1) => r,
            _ => 0.0,
        }
        .clamp(-1.0, 1.0);
        let dst = &mut out[c * sub..(c + 1) * sub];
        match sub {
            2 => dst.copy_from_slice(&((v * 32767.0).round() as i16).to_le_bytes()),
            3 => dst.copy_from_slice(&((v * 8388607.0).round() as i32).to_le_bytes()[..3]),
            _ => dst.copy_from_slice(&((v as f64 * 2147483647.0).round() as i32).to_le_bytes()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(v: Vec<[f32; 2]>) -> impl FnMut() -> Option<[f32; 2]> {
        let mut it = v.into_iter();
        move || it.next()
    }

    fn tone(hz: f64, rate: f64, n: usize) -> Vec<[f32; 2]> {
        (0..n).map(|i| {
            let v = (2.0 * std::f64::consts::PI * hz * i as f64 / rate).sin() as f32 * 0.8;
            [v, -v]
        }).collect()
    }

    #[test]
    fn at_the_same_rate_the_output_is_the_input_one_frame_later() {
        let input: Vec<[f32; 2]> = (0..200).map(|i| [i as f32 * 0.003, -(i as f32) * 0.002]).collect();
        let mut rs = StreamResampler::new();
        let mut src = feed(input.clone());
        let out: Vec<[f32; 2]> = std::iter::from_fn(|| rs.pull(1.0, &mut src)).take(150).collect();
        for (k, o) in out.iter().enumerate() {
            assert!((o[0] - input[k + 1][0]).abs() < 1e-5 && (o[1] - input[k + 1][1]).abs() < 1e-5, "frame {k}");
        }
    }

    #[test]
    fn converting_44_1_to_48_khz_keeps_the_pitch_and_the_level() {
        let input = tone(1000.0, 44_100.0, 44_100);
        let mut rs = StreamResampler::new();
        let mut src = feed(input);
        let step = 44_100.0 / 48_000.0;
        let out: Vec<[f32; 2]> = std::iter::from_fn(|| rs.pull(step, &mut src)).collect();
        assert!((47_990..=48_000).contains(&out.len()), "one second of input gives about one second of output: {}", out.len());
        // the reference: the same tone sampled where the resampler reads it (input position k * step + 1: its fixed one-frame delay)
        let want: Vec<f32> = (0..out.len()).map(|k| ((2.0 * std::f64::consts::PI * 1000.0 * (k as f64 * step + 1.0) / 44_100.0).sin() * 0.8) as f32).collect();
        let worst = out.iter().zip(&want).skip(8).fold(0.0f32, |m, (a, b)| m.max((a[0] - b).abs()));
        assert!(worst < 0.012, "the 1 kHz tone is intact: {worst}");
        assert!((out[10_000][0] + out[10_000][1]).abs() < 1e-6, "left and right stay separate (they were opposite)");
    }

    #[test]
    fn an_empty_buffer_pauses_the_stream_and_resuming_gives_the_same_audio() {
        let input = tone(440.0, 44_100.0, 5000);
        let step = 0.97;
        let whole = {
            let mut rs = StreamResampler::new();
            let mut src = feed(input.clone());
            std::iter::from_fn(|| rs.pull(step, &mut src)).collect::<Vec<_>>()
        };
        // the same input arrives in three bursts with underruns in between
        let mut rs = StreamResampler::new();
        let mut got = Vec::new();
        let mut at = 0;
        for burst in [1000usize, 1, 3999] {
            let mut src = feed(input[at..at + burst].to_vec());
            at += burst;
            while let Some(f) = rs.pull(step, &mut src) {
                got.push(f);
            }
        }
        assert_eq!(got.len(), whole.len());
        assert!(got == whole, "an underrun in the middle does not change what comes out");
    }

    #[test]
    fn the_step_speeds_up_when_the_buffer_is_full_slows_when_it_is_empty_and_is_limited() {
        let base = 44_100.0 / 48_000.0;
        assert_eq!(adaptive_step(base, 1500, 1500), base, "on target: the nominal ratio");
        assert!(adaptive_step(base, 3000, 1500) > base, "too full: read faster to drain it");
        assert!(adaptive_step(base, 300, 1500) < base, "too empty: read slower");
        let max = adaptive_step(base, 1_000_000, 1500);
        assert!((max / base - 1.005).abs() < 1e-9, "never more than 0.5% off: {}", max / base);
        assert!((adaptive_step(base, 0, 1500) / base - 0.995).abs() < 1e-9);
    }

    #[test]
    fn a_clock_that_runs_fast_is_followed_by_the_buffer_staying_level() {
        // a producer that is 100 ppm fast against the consumer: without correction the buffer grows by 4.4 frames a second; with the
        // adaptive step the fill settles near the target
        let target = 1500usize;
        let mut ring: std::collections::VecDeque<[f32; 2]> = std::collections::VecDeque::new();
        ring.extend(std::iter::repeat_n([0.1, 0.1], target));
        let mut rs = StreamResampler::new();
        let (mut prod_acc, mut max_dev) = (0.0f64, 0usize);
        for sec in 0..120 {
            for _ in 0..44_100 {
                prod_acc += 1.0001; // frames the producer makes per consumed output frame
                while prod_acc >= 1.0 {
                    ring.push_back([0.1, 0.1]);
                    prod_acc -= 1.0;
                }
                let step = adaptive_step(1.0, ring.len(), target);
                let mut src = || ring.pop_front();
                assert!(rs.pull(step, &mut src).is_some(), "underrun at {sec} s");
            }
            max_dev = max_dev.max(ring.len().abs_diff(target));
        }
        assert!(max_dev < 600, "after 2 minutes the buffer never strayed more than {max_dev} frames from the target");
    }

    #[test]
    fn frames_are_encoded_for_any_device_format() {
        let mut b = [0u8; 12];
        encode_frame(0.5, -0.25, 2, 3, &mut b[..6]);
        assert_eq!(&b[..3], &(4194304i32).to_le_bytes()[..3]);
        assert_eq!(&b[3..6], &(-2097152i32).to_le_bytes()[..3]);
        encode_frame(1.0, -1.0, 2, 2, &mut b[..4]);
        assert_eq!((i16::from_le_bytes([b[0], b[1]]), i16::from_le_bytes([b[2], b[3]])), (32767, -32767));
        encode_frame(0.5, 0.0, 1, 2, &mut b[..2]);
        assert_eq!(i16::from_le_bytes([b[0], b[1]]), (0.25f32 * 32767.0).round() as i16, "mono is the average");
        encode_frame(0.5, 0.5, 2, 4, &mut b[..8]);
        assert_eq!(i32::from_le_bytes([b[0], b[1], b[2], b[3]]), 1_073_741_824);
        encode_frame(0.3, 0.4, 4, 2, &mut b[..8]);
        assert_eq!((i16::from_le_bytes([b[4], b[5]]), i16::from_le_bytes([b[6], b[7]])), (0, 0), "extra channels are silent");
        encode_frame(7.0, -7.0, 2, 3, &mut b[..6]);
        assert_eq!(&b[..3], &(8388607i32).to_le_bytes()[..3], "clamped to full scale, not wrapped");
        assert_eq!(&b[3..6], &(-8388607i32).to_le_bytes()[..3]);
    }
}
