//! WAV output and numeric analysis of captured PCM (we cannot listen, so we measure).

use std::fmt;
use std::io::Write;

/// Write a canonical 44-byte-header PCM WAV.
pub fn write_wav<W: Write>(mut w: W, channels: u16, bytes_per_sample: u16, rate: u32, pcm: &[u8]) -> std::io::Result<()> {
    let frame = (channels * bytes_per_sample) as usize;
    let data = &pcm[..pcm.len() - pcm.len() % frame];
    let len = data.len() as u32;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + len).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&rate.to_le_bytes())?;
    w.write_all(&(rate * frame as u32).to_le_bytes())?;
    w.write_all(&(frame as u16).to_le_bytes())?;
    w.write_all(&(bytes_per_sample * 8).to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(data)
}

/// Parse a PCM WAV -> (channels, bytes_per_sample, rate, data).
pub fn read_wav(b: &[u8]) -> Result<(u16, u16, u32, &[u8]), String> {
    if b.len() < 44 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let (mut i, mut fmt) = (12, None);
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let n = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let body = &b[i + 8..(i + 8 + n).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            fmt = Some((
                u16::from_le_bytes([body[2], body[3]]),
                u16::from_le_bytes([body[14], body[15]]) / 8,
                u32::from_le_bytes([body[4], body[5], body[6], body[7]]),
            ));
        } else if id == b"data" {
            let (ch, bps, rate) = fmt.ok_or("data before fmt")?;
            return Ok((ch, bps, rate, body));
        }
        i += 8 + n + (n & 1);
    }
    Err("no data chunk".into())
}

#[derive(Debug, Clone)]
pub struct Report {
    pub bytes: usize,
    pub stray_bytes: usize,
    pub channels: usize,
    pub frames: usize,
    pub seconds: f64,
    pub peak_db: Vec<f64>,
    pub rms_db: Vec<f64>,
    pub dc: Vec<f64>,
    pub silent_fraction: f64,
    pub silent: bool,
    pub lr_max_diff: Option<i64>,
}

fn db(x: f64, full: f64) -> f64 {
    if x <= 0.0 { -999.0 } else { 20.0 * (x / full).log10() }
}

fn sample(raw: &[u8], bps: usize) -> i64 {
    match bps {
        2 => i16::from_le_bytes([raw[0], raw[1]]) as i64,
        3 => (((raw[2] as i8) as i64) << 16) | ((raw[1] as i64) << 8) | raw[0] as i64,
        _ => i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as i64,
    }
}

/// Peak / RMS / DC per channel, fraction of frames below -70 dBFS, and the exact max left-right difference.
pub fn analyze(raw: &[u8], channels: usize, bps: usize, rate: u32) -> Report {
    assert!((2..=4).contains(&bps) && channels >= 1);
    let frame = channels * bps;
    let usable = raw.len() - raw.len() % frame;
    let frames = usable / frame;
    let full = (1i64 << (bps * 8 - 1)) as f64;
    let (mut peak, mut sumsq, mut sum) = (vec![0i64; channels], vec![0f64; channels], vec![0f64; channels]);
    let (mut silent_frames, mut lr) = (0usize, 0i64);
    let thr = full * 10f64.powf(-70.0 / 20.0);
    for f in 0..frames {
        let base = f * frame;
        let mut first = 0i64;
        for c in 0..channels {
            let v = sample(&raw[base + c * bps..], bps);
            peak[c] = peak[c].max(v.abs());
            sumsq[c] += (v as f64) * (v as f64);
            sum[c] += v as f64;
            if c == 0 {
                first = v;
                if (v.abs() as f64) < thr {
                    silent_frames += 1;
                }
            } else if c == 1 {
                lr = lr.max((first - v).abs());
            }
        }
    }
    let n = frames.max(1) as f64;
    let peak_db: Vec<f64> = peak.iter().map(|p| db(*p as f64, full)).collect();
    Report {
        bytes: raw.len(),
        stray_bytes: raw.len() - usable,
        channels,
        frames,
        seconds: frames as f64 / rate as f64,
        silent: frames == 0 || peak_db.iter().cloned().fold(-999.0, f64::max) < -70.0,
        peak_db,
        rms_db: sumsq.iter().map(|s| db((s / n).sqrt(), full)).collect(),
        dc: sum.iter().map(|s| s / n / full).collect(),
        silent_fraction: if frames == 0 { 1.0 } else { silent_frames as f64 / frames as f64 },
        lr_max_diff: if channels >= 2 { Some(lr) } else { None },
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "captured {:.2} s  ({} frames, {} bytes", self.seconds, self.frames, self.bytes)?;
        if self.stray_bytes > 0 {
            write!(f, ", {} stray bytes", self.stray_bytes)?;
        }
        writeln!(f, ")")?;
        for c in 0..self.channels {
            writeln!(f, "  ch{}  peak {:7.1} dBFS   rms {:7.1} dBFS   dc {:+.5}", c, self.peak_db[c], self.rms_db[c], self.dc[c])?;
        }
        writeln!(f, "  silent frames: {:.1}%", 100.0 * self.silent_fraction)?;
        if let Some(d) = self.lr_max_diff {
            writeln!(f, "  left/right max difference: {} LSB{}", d, if d == 0 { "  (identical: mono downmix is lossless)" } else { "" })?;
        }
        write!(f, "  verdict: {}", if self.silent { "SILENT (no signal above -70 dBFS)" } else { "SIGNAL PRESENT" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, amp: f64, ch: usize) -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..frames {
            let s = (amp * 8388607.0 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 44100.0).sin()) as i32;
            for _ in 0..ch {
                v.extend_from_slice(&s.to_le_bytes()[..3]);
            }
        }
        v
    }

    #[test]
    fn tone_levels_and_identical_channels() {
        let r = analyze(&tone(44100, 0.5, 2), 2, 3, 44100);
        assert!((r.seconds - 1.0).abs() < 1e-9);
        assert!((r.peak_db[0] - -6.02).abs() < 0.05, "{}", r.peak_db[0]);
        assert!((r.rms_db[0] - -9.03).abs() < 0.1, "{}", r.rms_db[0]);
        assert_eq!(r.lr_max_diff, Some(0));
        assert!(!r.silent);
    }

    #[test]
    fn silence_stray_bytes_and_empty() {
        let r = analyze(&vec![0u8; 6 * 100 + 5], 2, 3, 44100);
        assert!(r.silent);
        assert_eq!((r.frames, r.stray_bytes, r.silent_fraction), (100, 5, 1.0));
        assert!(analyze(&[], 2, 3, 44100).silent);
    }

    #[test]
    fn full_scale_negative_and_lr_difference() {
        // 24-bit -8388608 on the left, +100 on the right
        let mut raw = Vec::new();
        raw.extend_from_slice(&[0x00, 0x00, 0x80]);
        raw.extend_from_slice(&[100, 0, 0]);
        let r = analyze(&raw, 2, 3, 44100);
        assert_eq!(r.lr_max_diff, Some(8388608 + 100));
        assert!((r.peak_db[0] - 0.0).abs() < 1e-9);
    }

    #[test]
    fn wav_roundtrip() {
        let pcm = tone(1000, 0.3, 2);
        let mut out = Vec::new();
        write_wav(&mut out, 2, 3, 44100, &pcm).unwrap();
        assert_eq!(out.len(), 44 + pcm.len());
        let (ch, bps, rate, data) = read_wav(&out).unwrap();
        assert_eq!((ch, bps, rate), (2, 3, 44100));
        assert_eq!(data, &pcm[..]);
        assert!(read_wav(b"nope").is_err());
    }
}
