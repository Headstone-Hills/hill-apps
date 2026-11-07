//! WAV decode to interleaved stereo f32, linear resampling (WAV import, feature 14), and 16/24-bit encode.

pub struct Audio {
    pub rate: u32,
    /// interleaved L,R
    pub data: Vec<f32>,
}

impl Audio {
    pub fn frames(&self) -> usize {
        self.data.len() / 2
    }
}

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

struct Parsed<'a> {
    tag: u16,
    ch: usize,
    rate: u32,
    bps: usize,
    data: &'a [u8],
}

fn parse(b: &[u8]) -> Result<Parsed<'_>, String> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let (mut i, mut fmt, mut data) = (12usize, None, None);
    while i + 8 <= b.len() {
        let n = u32le(b, i + 4) as usize;
        let body = &b[i + 8..(i + 8 + n).min(b.len())];
        match &b[i..i + 4] {
            b"fmt " if body.len() >= 16 => {
                let mut tag = u16le(body, 0);
                if tag == 0xFFFE && body.len() >= 26 {
                    tag = u16le(body, 24); // WAVE_FORMAT_EXTENSIBLE: real format in the sub-format GUID
                }
                fmt = Some((tag, u16le(body, 2) as usize, u32le(body, 4), u16le(body, 14) as usize));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        i += 8 + n + (n & 1);
    }
    let ((tag, ch, rate, bits), data) = (fmt.ok_or("no fmt chunk")?, data.ok_or("no data chunk")?);
    if ch == 0 || rate == 0 {
        return Err("bad fmt".into());
    }
    let bps = bits / 8;
    if !(tag == 1 && (1..=4).contains(&bps) || tag == 3 && bps == 4) {
        return Err(format!("unsupported WAV format tag {tag}, {bits} bits"));
    }
    Ok(Parsed { tag, ch, rate, bps, data })
}

fn read_sample(p: &Parsed, byte: usize) -> f32 {
    let q = &p.data[byte..];
    match (p.tag, p.bps) {
        (1, 1) => (q[0] as f32 - 128.0) / 128.0,
        (1, 2) => i16::from_le_bytes([q[0], q[1]]) as f32 / 32768.0,
        (1, 3) => ((((q[2] as i8) as i32) << 16) | ((q[1] as i32) << 8) | q[0] as i32) as f32 / 8388608.0,
        (1, _) => i32::from_le_bytes([q[0], q[1], q[2], q[3]]) as f32 / 2147483648.0,
        _ => f32::from_le_bytes([q[0], q[1], q[2], q[3]]),
    }
}

/// PCM 8/16/24/32-bit or 32-bit float WAV; mono is duplicated, extra channels are dropped.
pub fn decode(b: &[u8]) -> Result<Audio, String> {
    let p = parse(b)?;
    let frame = p.ch * p.bps;
    let n = p.data.len() / frame;
    let mut out = Vec::with_capacity(n * 2);
    for f in 0..n {
        let l = read_sample(&p, f * frame);
        out.push(l);
        out.push(if p.ch >= 2 { read_sample(&p, f * frame + p.bps) } else { l });
    }
    Ok(Audio { rate: p.rate, data: out })
}

/// Same formats, one channel out (several channels are averaged). Recordings are mono, so playing them through
/// this costs half the memory of [`decode`]; the decoded length is exactly what the header's data chunk holds.
pub fn decode_mono(b: &[u8]) -> Result<(u32, Vec<f32>), String> {
    let p = parse(b)?;
    let frame = p.ch * p.bps;
    let n = p.data.len() / frame;
    let mut out = Vec::with_capacity(n);
    for f in 0..n {
        let sum: f32 = (0..p.ch).map(|c| read_sample(&p, f * frame + c * p.bps)).sum();
        out.push(sum / p.ch as f32);
    }
    Ok((p.rate, out))
}

/// Resample mono audio from `from` Hz to `to` Hz with 4-point Hermite interpolation (what a take from a 48 kHz-only USB mic needs to play
/// at the project rate). Cheap rather than studio-grade: fine below about 0.4 of the lower rate, with a little loss above.
pub fn resample_mono(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if x.is_empty() || from == 0 || to == 0 {
        return Vec::new();
    }
    if from == to {
        return x.to_vec();
    }
    let n = (x.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    let at = |i: i64| x[i.clamp(0, x.len() as i64 - 1) as usize];
    (0..n)
        .map(|k| {
            let t = k as f64 * step;
            let i = t.floor() as i64;
            let f = (t - i as f64) as f32;
            let (y0, y1, y2, y3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            let c0 = y1;
            let c1 = 0.5 * (y2 - y0);
            let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
            let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
            ((c3 * f + c2) * f + c1) * f + c0
        })
        .collect()
}

/// Linear-interpolation resample (cheap; fine for import and auditioning).
pub fn resample_linear(a: &Audio, rate: u32) -> Audio {
    if a.rate == rate || a.frames() == 0 {
        return Audio { rate, data: a.data.clone() };
    }
    let n = ((a.frames() as f64) * rate as f64 / a.rate as f64).round() as usize;
    let last = a.frames() - 1;
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let x = if n > 1 { i as f64 * last as f64 / (n - 1) as f64 } else { 0.0 };
        let (i0, t) = (x.floor() as usize, (x - x.floor()) as f32);
        let i1 = (i0 + 1).min(last);
        for c in 0..2 {
            out.push(a.data[2 * i0 + c] * (1.0 - t) + a.data[2 * i1 + c] * t);
        }
    }
    Audio { rate, data: out }
}

/// Encode interleaved stereo float as 16- or 24-bit PCM WAV (clamped, no dither; dither is added at export).
pub fn encode(a: &Audio, bits: u16) -> Vec<u8> {
    assert!(bits == 16 || bits == 24);
    let bps = (bits / 8) as usize;
    let mut pcm = Vec::with_capacity(a.data.len() * bps);
    for s in &a.data {
        let s = s.clamp(-1.0, 1.0);
        if bits == 16 {
            pcm.extend_from_slice(&((s * 32767.0).round() as i16).to_le_bytes());
        } else {
            pcm.extend_from_slice(&((s * 8388607.0).round() as i32).to_le_bytes()[..3]);
        }
    }
    let mut w = Vec::with_capacity(44 + pcm.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&a.rate.to_le_bytes());
    w.extend_from_slice(&(a.rate * 2 * bps as u32).to_le_bytes());
    w.extend_from_slice(&((2 * bps) as u16).to_le_bytes());
    w.extend_from_slice(&bits.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(&pcm);
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(frames: usize, rate: u32) -> Audio {
        Audio { rate, data: (0..frames).flat_map(|i| [i as f32 / frames as f32, -(i as f32) / frames as f32]).collect() }
    }

    #[test]
    fn resample_mono_keeps_pitch_level_and_length() {
        let sine = |hz: f64, rate: u32, n: usize| -> Vec<f32> { (0..n).map(|i| (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin() as f32 * 0.8).collect() };
        let x = sine(1000.0, 48_000, 48_000);
        let y = resample_mono(&x, 48_000, 44_100);
        assert_eq!(y.len(), 44_100, "one second stays one second");
        // the same 1 kHz tone at the new rate: compare with a sine generated directly at 44.1 kHz
        let want = sine(1000.0, 44_100, 44_100);
        let worst = y.iter().zip(&want).skip(10).take(44_000).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst < 0.01, "1 kHz is reproduced to within 1%: {worst}");
        // a higher tone still has its level
        let hi = resample_mono(&sine(10_000.0, 48_000, 48_000), 48_000, 44_100);
        let amp = hi[100..40_000].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(amp > 0.7 && amp < 0.85, "10 kHz keeps about its level: {amp}");
        assert_eq!(resample_mono(&x, 48_000, 48_000), x);
        assert!(resample_mono(&[], 48_000, 44_100).is_empty() && resample_mono(&x, 0, 44_100).is_empty());
        assert_eq!(resample_mono(&sine(100.0, 44_100, 44_100), 44_100, 48_000).len(), 48_000);
    }

    #[test]
    fn decode_mono_reads_a_mono_24_bit_recording_exactly_and_averages_stereo() {
        let a = Audio { rate: 44100, data: (0..200).map(|i| if i % 2 == 0 { 0.5 } else { -0.25 }).collect() };
        let bytes = encode(&a, 24);
        let (rate, mono) = decode_mono(&bytes).unwrap();
        assert_eq!((rate, mono.len()), (44100, 100));
        assert!((mono[7] - 0.125).abs() < 1e-5, "(0.5 + -0.25) / 2");
        assert!(decode_mono(b"nope").is_err());
    }

    #[test]
    fn roundtrip_16_and_24_bit() {
        let a = ramp(500, 44100);
        // encode scales by 2^(n-1)-1 and decode divides by 2^(n-1) (the usual convention): up to 2 LSB apart
        for (bits, tol) in [(16u16, 2.0 / 32768.0), (24, 2.0 / 8_388_608.0)] {
            let d = decode(&encode(&a, bits)).unwrap();
            assert_eq!((d.rate, d.frames()), (44100, 500));
            assert!(a.data.iter().zip(&d.data).all(|(x, y)| (x - y).abs() <= tol), "{bits}-bit");
        }
    }

    #[test]
    fn mono_is_duplicated() {
        let mut w = encode(&Audio { rate: 8000, data: vec![0.5, 0.5, -0.5, -0.5] }, 16);
        w[22] = 1; // channels = 1
        w[32] = 2; // block align = 2
        let d = decode(&w).unwrap();
        // two bytes per frame now: 8 data bytes = 4 mono frames
        assert_eq!(d.frames(), 4);
        assert_eq!(d.data[0], d.data[1]);
    }

    #[test]
    fn resample_changes_length_and_keeps_endpoints() {
        let a = ramp(1000, 22050);
        let r = resample_linear(&a, 44100);
        assert_eq!(r.rate, 44100);
        assert_eq!(r.frames(), 2000);
        assert!((r.data[0] - a.data[0]).abs() < 1e-6);
        assert!((r.data[r.data.len() - 2] - a.data[a.data.len() - 2]).abs() < 1e-6);
        assert_eq!(resample_linear(&a, 22050).frames(), 1000);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(b"hello").is_err());
        assert!(decode(&[]).is_err());
        let mut w = encode(&ramp(10, 44100), 16);
        w[20] = 9; // unknown format tag
        assert!(decode(&w).is_err());
    }
}
