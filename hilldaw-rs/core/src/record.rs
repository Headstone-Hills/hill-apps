//! Recording pieces (no device, no threads): mono downmix, a crash-tolerant streaming WAV writer,
//! onset detection for latency calibration, and take placement maths.
//!
//! The FM-1 delivers 44.1 kHz 24-bit *stereo* with an engine that is mono; we keep one mono 24-bit file
//! per take (half the storage) by averaging L and R. Captured audio is never modified on disk.

use std::io::{Seek, SeekFrom, Write};

/// 24-bit little-endian stereo bytes -> 24-bit little-endian mono bytes, (L+R)/2.
/// Bytes that do not complete a frame are carried to the next call.
#[derive(Default)]
pub struct Downmixer {
    carry: Vec<u8>,
}

impl Downmixer {
    pub fn push(&mut self, input: &[u8], out: &mut Vec<u8>) {
        let mut buf;
        let data: &[u8] = if self.carry.is_empty() {
            input
        } else {
            buf = std::mem::take(&mut self.carry);
            buf.extend_from_slice(input);
            &buf
        };
        let whole = data.len() - data.len() % 6;
        for f in data[..whole].chunks_exact(6) {
            let l = (((f[2] as i8) as i32) << 16) | ((f[1] as i32) << 8) | f[0] as i32;
            let r = (((f[5] as i8) as i32) << 16) | ((f[4] as i32) << 8) | f[3] as i32;
            let m = (l + r) >> 1;
            out.extend_from_slice(&m.to_le_bytes()[..3]);
        }
        self.carry = data[whole..].to_vec();
    }
}

/// Any PCM capture format -> 24-bit little-endian mono, the average of all channels. Handles 16-, 24- and 32-bit signed samples
/// (32-bit keeps its top 24 bits) and carries bytes that do not complete a frame to the next call. For 2 channels of 24-bit it gives
/// exactly what [`Downmixer`] does.
pub struct MonoMixer {
    channels: usize,
    bytes: usize,
    carry: Vec<u8>,
}

impl MonoMixer {
    /// `bytes_per_sample` must be 2, 3 or 4 and `channels` at least 1.
    pub fn new(channels: u8, bytes_per_sample: u8) -> Self {
        assert!(matches!(bytes_per_sample, 2..=4) && channels >= 1, "unsupported capture format");
        MonoMixer { channels: channels as usize, bytes: bytes_per_sample as usize, carry: Vec::new() }
    }

    pub fn frame_bytes(&self) -> usize {
        self.channels * self.bytes
    }

    fn sample(&self, b: &[u8]) -> i32 {
        match self.bytes {
            2 => (i16::from_le_bytes([b[0], b[1]]) as i32) << 8,
            3 => (((b[2] as i8) as i32) << 16) | ((b[1] as i32) << 8) | b[0] as i32,
            _ => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) >> 8,
        }
    }

    pub fn push(&mut self, input: &[u8], out: &mut Vec<u8>) {
        let mut buf;
        let data: &[u8] = if self.carry.is_empty() {
            input
        } else {
            buf = std::mem::take(&mut self.carry);
            buf.extend_from_slice(input);
            &buf
        };
        let fb = self.frame_bytes();
        let whole = data.len() - data.len() % fb;
        for f in data[..whole].chunks_exact(fb) {
            let sum: i64 = f.chunks_exact(self.bytes).map(|s| self.sample(s) as i64).sum();
            let m = sum.div_euclid(self.channels as i64) as i32; // floor, like `>> 1` for two channels
            out.extend_from_slice(&m.to_le_bytes()[..3]);
        }
        self.carry = data[whole..].to_vec();
    }
}

/// 24-bit mono bytes -> float (for analysis and onset detection).
pub fn mono24_to_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(3).map(|p| ((((p[2] as i8) as i32) << 16) | ((p[1] as i32) << 8) | p[0] as i32) as f32 / 8388608.0).collect()
}

/// WAV writer for streaming a recording to disk. The header is written up front with zero sizes and
/// rewritten by [`StreamingWav::flush_header`]; call that about once a second so a crash or power loss
/// leaves a valid file that is at most that much shorter than the audio captured.
pub struct StreamingWav<W: Write + Seek> {
    w: W,
    data_len: u64,
    pub header_flushes: u32,
}

impl<W: Write + Seek> StreamingWav<W> {
    pub fn new(mut w: W, channels: u16, bytes_per_sample: u16, rate: u32) -> std::io::Result<Self> {
        let frame = channels * bytes_per_sample;
        w.write_all(b"RIFF")?;
        w.write_all(&36u32.to_le_bytes())?;
        w.write_all(b"WAVEfmt ")?;
        w.write_all(&16u32.to_le_bytes())?;
        w.write_all(&1u16.to_le_bytes())?;
        w.write_all(&channels.to_le_bytes())?;
        w.write_all(&rate.to_le_bytes())?;
        w.write_all(&(rate * frame as u32).to_le_bytes())?;
        w.write_all(&frame.to_le_bytes())?;
        w.write_all(&(bytes_per_sample * 8).to_le_bytes())?;
        w.write_all(b"data")?;
        w.write_all(&0u32.to_le_bytes())?;
        Ok(StreamingWav { w, data_len: 0, header_flushes: 0 })
    }

    pub fn write(&mut self, pcm: &[u8]) -> std::io::Result<()> {
        self.w.write_all(pcm)?;
        self.data_len += pcm.len() as u64;
        Ok(())
    }

    pub fn data_len(&self) -> u64 {
        self.data_len
    }

    /// Make the file valid as it stands (sizes = bytes written so far).
    pub fn flush_header(&mut self) -> std::io::Result<()> {
        let n = self.data_len.min(u32::MAX as u64 - 36) as u32;
        self.w.flush()?;
        self.w.seek(SeekFrom::Start(4))?;
        self.w.write_all(&(36 + n).to_le_bytes())?;
        self.w.seek(SeekFrom::Start(40))?;
        self.w.write_all(&n.to_le_bytes())?;
        self.w.seek(SeekFrom::End(0))?;
        self.w.flush()?;
        self.header_flushes += 1;
        Ok(())
    }

    pub fn finish(mut self) -> std::io::Result<W> {
        self.flush_header()?;
        Ok(self.w)
    }
}

/// First sample whose magnitude exceeds `threshold` (linear, 0..1). Used to find when a note became audible.
pub fn detect_onset(x: &[f32], threshold: f32) -> Option<usize> {
    x.iter().position(|s| s.abs() > threshold)
}

/// Where a take belongs on the timeline. `latency_offset_frames` is how late captured audio arrives
/// (positive = late), so the take is placed that much earlier. Never before frame 0.
pub fn placed_start(record_start_frame: u64, latency_offset_frames: i64) -> u64 {
    (record_start_frame as i64 - latency_offset_frames).max(0) as u64
}

/// Clock-drift correction for a take. The FM-1 (USB) and the SP (sound card) run on separate clocks.
/// `captured / elapsed` is the playback-rate ratio that makes a take of `captured` frames last exactly
/// `elapsed` transport frames. A ratio of 1.0 means the clocks agreed.
pub fn drift_ratio(captured_frames: u64, elapsed_transport_frames: u64) -> f64 {
    if elapsed_transport_frames == 0 { 1.0 } else { captured_frames as f64 / elapsed_transport_frames as f64 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav;
    use std::io::Cursor;

    fn stereo24(l: i32, r: i32) -> Vec<u8> {
        let mut v = l.to_le_bytes()[..3].to_vec();
        v.extend_from_slice(&r.to_le_bytes()[..3]);
        v
    }

    #[test]
    fn mono_mixer_matches_the_fm1_downmixer_and_handles_other_formats() {
        // 2ch 24-bit: identical to Downmixer for any input
        let mut bytes = Vec::new();
        for i in 0..500i32 {
            bytes.extend(stereo24((i * 7919 % 8_000_000) - 4_000_000, -(i * 104_729 % 8_000_000) + 4_000_000));
        }
        let (mut a, mut b) = (Vec::new(), Vec::new());
        Downmixer::default().push(&bytes, &mut a);
        MonoMixer::new(2, 3).push(&bytes, &mut b);
        assert!(a == b, "same 24-bit stereo result");

        // mono 16-bit: the value moved up into 24 bits
        let mut m = Vec::new();
        MonoMixer::new(1, 2).push(&[0x34, 0x12, 0x00, 0x80], &mut m); // 0x1234, -32768
        let v: Vec<i32> = m.chunks(3).map(|c| (((c[2] as i8) as i32) << 16) | ((c[1] as i32) << 8) | c[0] as i32).collect();
        assert_eq!(v, vec![0x1234 << 8, -32768 << 8]);

        // stereo 16-bit averages; 32-bit keeps the top 24 bits; 4 channels average all of them
        let mut m = Vec::new();
        MonoMixer::new(2, 2).push(&[0x00, 0x10, 0x00, 0x30], &mut m); // 0x1000 and 0x3000 -> 0x2000
        assert_eq!(m, (0x2000i32 << 8).to_le_bytes()[..3].to_vec());
        let mut m = Vec::new();
        MonoMixer::new(1, 4).push(&0x7FFF_FF00i32.to_le_bytes(), &mut m);
        assert_eq!(m, vec![0xFF, 0xFF, 0x7F], "top 24 bits of a 32-bit sample");
        let mut m = Vec::new();
        let mut four = Vec::new();
        for v in [100i32, 200, 300, 400] {
            four.extend_from_slice(&v.to_le_bytes()[..3]);
        }
        MonoMixer::new(4, 3).push(&four, &mut m);
        assert_eq!(m, 250i32.to_le_bytes()[..3].to_vec());
    }

    #[test]
    fn mono_mixer_carries_a_partial_frame_between_packets_for_any_split() {
        let mut bytes = Vec::new();
        for i in 0..200i16 {
            bytes.extend_from_slice(&(i * 150).to_le_bytes());
            bytes.extend_from_slice(&(-i * 90).to_le_bytes());
        }
        let mut whole = Vec::new();
        MonoMixer::new(2, 2).push(&bytes, &mut whole);
        for split in [1usize, 3, 5, 7, 99] {
            let mut mx = MonoMixer::new(2, 2);
            let mut got = Vec::new();
            for c in bytes.chunks(split) {
                mx.push(c, &mut got);
            }
            assert!(got == whole, "split {split}");
        }
        assert_eq!(whole.len(), 200 * 3);
    }

    #[test]
    fn downmix_averages_and_handles_negative_values() {
        let mut d = Downmixer::default();
        let mut out = Vec::new();
        let mut inp = stereo24(1000, 3000);
        inp.extend(stereo24(-8388608, -8388608));
        inp.extend(stereo24(-100, 50));
        d.push(&inp, &mut out);
        let m = mono24_to_f32(&out);
        assert_eq!(m.len(), 3);
        assert!((m[0] - 2000.0 / 8388608.0).abs() < 1e-9);
        assert_eq!(m[1], -1.0);
        assert!((m[2] - (-25.0 / 8388608.0)).abs() < 1e-7); // (-100+50)>>1 = -25
    }

    #[test]
    fn downmix_is_independent_of_how_the_stream_is_split() {
        let mut inp = Vec::new();
        for i in 0..1000 {
            inp.extend(stereo24(i * 37 - 9000, 5000 - i * 11));
        }
        let mut whole = Vec::new();
        Downmixer::default().push(&inp, &mut whole);
        for split in [1usize, 5, 6, 7, 264, 270, 999] {
            let (mut d, mut out) = (Downmixer::default(), Vec::new());
            for c in inp.chunks(split) {
                d.push(c, &mut out);
            }
            assert_eq!(out, whole, "split {split}");
        }
        assert_eq!(whole.len(), 3000);
    }

    #[test]
    fn streaming_wav_is_valid_after_every_flush_and_after_a_crash() {
        let mut s = StreamingWav::new(Cursor::new(Vec::new()), 1, 3, 44100).unwrap();
        s.write(&[1, 2, 3, 4, 5, 6]).unwrap();
        s.flush_header().unwrap();
        s.write(&[7, 8, 9]).unwrap(); // written after the last flush: lost in a crash
        let crashed = s.w.get_ref().clone();
        let a = wav::decode(&crashed).unwrap();
        assert_eq!(a.frames(), 2, "a crash keeps everything up to the last header flush");
        let done = s.finish().unwrap().into_inner();
        let b = wav::decode(&done).unwrap();
        assert_eq!((b.frames(), b.rate), (3, 44100));
        assert_eq!(u32::from_le_bytes(done[4..8].try_into().unwrap()) as usize, done.len() - 8);
    }

    #[test]
    fn onset_and_placement_and_drift() {
        let mut x = vec![0.0f32; 1000];
        x[600] = 0.02;
        x[700] = 0.5;
        assert_eq!(detect_onset(&x, 0.001), Some(600));
        assert_eq!(detect_onset(&x, 0.1), Some(700));
        assert_eq!(detect_onset(&[0.0; 10], 0.001), None);
        assert_eq!(placed_start(10_000, 512), 9_488);
        assert_eq!(placed_start(100, 512), 0);
        assert_eq!(placed_start(100, -50), 150);
        assert_eq!(drift_ratio(2_646_000, 2_646_000), 1.0);
        assert!((drift_ratio(2_646_132, 2_646_000) - (1.0 + 132.0 / 2_646_000.0)).abs() < 1e-12); // ~50 ppm fast: 132 frames over 60 s
        assert_eq!(drift_ratio(5, 0), 1.0);
    }
}
