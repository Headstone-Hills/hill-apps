//! Small WAV (RIFF) decoder: PCM 8/16/24/32-bit and float 32/64, any channel
//! count (extra channels are dropped), including WAVE_FORMAT_EXTENSIBLE.

use std::sync::Arc;

/// How the audio is held. 16-bit files stay 16-bit, and mono when both channels
/// match (most of the converted library), which is 4x smaller than f32 stereo.
enum Store {
    /// Interleaved stereo f32.
    F32(Vec<f32>),
    /// Interleaved stereo i16.
    I16(Vec<i16>),
    /// i16, one channel (played on both sides).
    I16Mono(Vec<i16>),
}

/// Decoded audio. Read it with `frame(i)`.
pub struct Sample {
    store: Store,
    pub rate: u32,
}

const I16_SCALE: f32 = 1.0 / 32768.0;

impl Sample {
    /// From interleaved stereo f32.
    pub fn new(data: Vec<f32>, rate: u32) -> Self {
        Sample { store: Store::F32(data), rate }
    }
    pub fn frames(&self) -> usize {
        match &self.store {
            Store::F32(d) => d.len() / 2,
            Store::I16(d) => d.len() / 2,
            Store::I16Mono(d) => d.len(),
        }
    }
    /// The (left, right) pair at frame `i`; `i` must be below `frames()`.
    #[inline]
    pub fn frame(&self, i: usize) -> (f32, f32) {
        match &self.store {
            Store::F32(d) => (d[i * 2], d[i * 2 + 1]),
            Store::I16(d) => (d[i * 2] as f32 * I16_SCALE, d[i * 2 + 1] as f32 * I16_SCALE),
            Store::I16Mono(d) => {
                let v = d[i] as f32 * I16_SCALE;
                (v, v)
            }
        }
    }
    pub fn seconds(&self) -> f32 {
        self.frames() as f32 / self.rate.max(1) as f32
    }
    /// Memory held by the audio, in bytes.
    pub fn bytes(&self) -> usize {
        match &self.store {
            Store::F32(d) => d.len() * 4,
            Store::I16(d) => d.len() * 2,
            Store::I16Mono(d) => d.len() * 2,
        }
    }
    pub fn is_mono_stored(&self) -> bool {
        matches!(self.store, Store::I16Mono(_))
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
    if tag == 1 && bits == 16 {
        return Some(Arc::new(compact16(data, ch, n, rate)));
    }
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
    Some(Arc::new(Sample::new(out, rate)))
}

/// 16-bit PCM kept as i16. Two channels that differ by at most 2 LSB throughout
/// (dither on a mono source) are stored as one.
fn compact16(data: &[u8], ch: usize, n: usize, rate: u32) -> Sample {
    let s16 = |o: usize| i16::from_le_bytes([data[o], data[o + 1]]);
    if ch == 1 {
        let m: Vec<i16> = (0..n).map(|i| s16(i * 2)).collect();
        return Sample { store: Store::I16Mono(m), rate };
    }
    let mut st: Vec<i16> = Vec::with_capacity(n * 2);
    let mut max_diff = 0i32;
    for i in 0..n {
        let (l, r) = (s16(i * ch * 2), s16(i * ch * 2 + 2));
        max_diff = max_diff.max((l as i32 - r as i32).abs());
        st.push(l);
        st.push(r);
    }
    if max_diff <= 2 {
        let m: Vec<i16> = st.chunks_exact(2).map(|p| ((p[0] as i32 + p[1] as i32) / 2) as i16).collect();
        Sample { store: Store::I16Mono(m), rate }
    } else {
        Sample { store: Store::I16(st), rate }
    }
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
        assert!((s.frame(10).0 - src[10]).abs() < 1e-3);
        assert_eq!(s.frame(10).0, s.frame(10).1);
    }

    #[test]
    fn stereo_roundtrip() {
        let src = [0.5, -0.5, 0.25, -0.25];
        let s = parse(&encode16(&src, 2, 44100)).unwrap();
        assert_eq!(s.frames(), 2);
        assert!(s.frame(0).0 > 0.49 && s.frame(0).1 < -0.49);
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

    #[test]
    fn dual_mono_16bit_is_stored_mono_and_real_stereo_is_not() {
        // dithered dual-mono: channels differ by 1 LSB
        let tone: Vec<f32> = (0..2000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let mut dm = Vec::new();
        for (i, v) in tone.iter().enumerate() {
            dm.push(*v);
            dm.push(*v + if i % 2 == 0 { 1.0 / 32768.0 } else { 0.0 });
        }
        let s = parse(&encode16(&dm, 2, 44100)).unwrap();
        assert!(s.is_mono_stored());
        assert_eq!(s.bytes(), 2000 * 2);
        assert!((s.frame(500).0 - tone[500]).abs() < 2.0 / 32768.0);
        assert_eq!(s.frame(500).0, s.frame(500).1);
        // genuine stereo keeps both channels
        let st: Vec<f32> = (0..2000).flat_map(|i| [(i as f32 * 0.05).sin() * 0.5, (i as f32 * 0.09).sin() * 0.5]).collect();
        let s = parse(&encode16(&st, 2, 44100)).unwrap();
        assert!(!s.is_mono_stored());
        assert_eq!(s.bytes(), 2000 * 4);
        assert!((s.frame(300).1 - st[601]).abs() < 2.0 / 32768.0);
    }
}
