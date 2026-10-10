//! Small WAV (RIFF) decoder: PCM 8/16/24/32-bit and float 32/64, any channel
//! count (extra channels are dropped), including WAVE_FORMAT_EXTENSIBLE.

use std::sync::Arc;

/// Decoded audio: interleaved stereo f32.
pub struct Sample {
    pub data: Vec<f32>,
    pub rate: u32,
}

impl Sample {
    pub fn frames(&self) -> usize {
        self.data.len() / 2
    }
    pub fn seconds(&self) -> f32 {
        self.frames() as f32 / self.rate.max(1) as f32
    }
}

fn u16le(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32le(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

pub fn parse(bytes: &[u8]) -> Option<Arc<Sample>> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // tag, channels, rate, bits
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32le(bytes, pos + 4)? as usize;
        let body = pos + 8;
        let end = body.saturating_add(size).min(bytes.len());
        if id == b"fmt " {
            let mut tag = u16le(bytes, body)?;
            let ch = u16le(bytes, body + 2)?;
            let rate = u32le(bytes, body + 4)?;
            let bits = u16le(bytes, body + 14)?;
            if tag == 0xFFFE && size >= 26 {
                tag = u16le(bytes, body + 24)?;
            }
            fmt = Some((tag, ch, rate, bits));
        } else if id == b"data" {
            data = Some(&bytes[body..end]);
            if fmt.is_some() {
                break;
            }
        }
        pos = body.saturating_add(size).saturating_add(size & 1);
    }
    let (tag, ch, rate, bits) = fmt?;
    let data = data?;
    if ch == 0 || rate == 0 {
        return None;
    }
    let ch = ch as usize;
    let bps = (bits as usize) / 8;
    if bps == 0 {
        return None;
    }
    let frame_bytes = bps * ch;
    let n = data.len() / frame_bytes;
    let mut out = Vec::with_capacity(n * 2);
    let read = |o: usize| -> Option<f32> {
        let b = &data[o..o + bps];
        Some(match (tag, bits) {
            (1, 8) => (b[0] as f32 - 128.0) / 128.0,
            (1, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
            (1, 24) => {
                let v = (b[0] as i32) | (b[1] as i32) << 8 | (b[2] as i32) << 16;
                ((v << 8) >> 8) as f32 / 8388608.0
            }
            (1, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2147483648.0,
            (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            (3, 64) => f64::from_le_bytes(b.try_into().ok()?) as f32,
            _ => return None,
        })
    };
    for i in 0..n {
        let base = i * frame_bytes;
        let l = read(base)?;
        let r = if ch > 1 { read(base + bps)? } else { l };
        out.push(l);
        out.push(r);
    }
    Some(Arc::new(Sample { data: out, rate }))
}

/// Encode mono/stereo f32 as 16-bit PCM WAV (used by tests and tools).
pub fn encode16(data: &[f32], channels: u16, rate: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + data.len() * 2);
    let bytes = (data.len() * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for &s in data {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_roundtrip() {
        let src: Vec<f32> = (0..100).map(|i| (i as f32 / 100.0) - 0.5).collect();
        let w = encode16(&src, 1, 22050);
        let s = parse(&w).unwrap();
        assert_eq!(s.rate, 22050);
        assert_eq!(s.frames(), 100);
        assert!((s.data[20] - src[10]).abs() < 1e-3);
        assert_eq!(s.data[20], s.data[21]);
    }

    #[test]
    fn stereo_roundtrip() {
        let src = [0.5, -0.5, 0.25, -0.25];
        let s = parse(&encode16(&src, 2, 44100)).unwrap();
        assert_eq!(s.frames(), 2);
        assert!(s.data[0] > 0.49 && s.data[1] < -0.49);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"hello world, not a wav file").is_none());
        assert!(parse(&[]).is_none());
    }

    #[test]
    fn truncated_data_chunk_is_clamped() {
        let mut w = encode16(&[0.1; 64], 1, 8000);
        w.truncate(w.len() - 10);
        let s = parse(&w).unwrap();
        assert!(s.frames() >= 58 && s.frames() < 64);
    }
}
