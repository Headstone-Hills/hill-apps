//! USB Audio Class 1.0 descriptor parser. Reads the blob the kernel exposes at
//! `/sys/bus/usb/devices/<dev>/descriptors` and returns each isochronous audio-streaming
//! alternate setting with the format it carries, so nothing about the FM-1 is hardcoded
//! (other firmware, or a generic USB mic, parse the same way). UAC2 is not decoded yet.

const DT_INTERFACE: u8 = 0x04;
const DT_ENDPOINT: u8 = 0x05;
const DT_CS_INTERFACE: u8 = 0x24;
const CLASS_AUDIO: u8 = 1;
const SUBCLASS_STREAMING: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StreamAlt {
    pub iface: u8,
    pub alt: u8,
    pub ep: u8,
    pub max_packet: u16,
    pub interval: u8,
    pub sync: &'static str,
    pub channels: u8,
    pub subframe: u8,
    pub bits: u8,
    pub rates: Vec<u32>,
}

impl StreamAlt {
    /// true = device -> host (capture)
    pub fn is_capture(&self) -> bool {
        self.ep & 0x80 != 0
    }
    pub fn frame_bytes(&self) -> usize {
        self.channels as usize * self.subframe as usize
    }
    pub fn describe(&self) -> String {
        format!(
            "iface {} alt {} ep 0x{:02x} {} {} maxpkt {}  {}ch {}-bit({}B) rates {:?}",
            self.iface,
            self.alt,
            self.ep,
            if self.is_capture() { "in" } else { "out" },
            self.sync,
            self.max_packet,
            self.channels,
            self.bits,
            self.subframe,
            self.rates
        )
    }
}

fn sync_name(attrs: u8) -> &'static str {
    match (attrs >> 2) & 3 {
        0 => "none",
        1 => "async",
        2 => "adaptive",
        _ => "sync",
    }
}

pub fn parse_audio_streams(blob: &[u8]) -> Vec<StreamAlt> {
    let mut out = Vec::new();
    let mut cur: Option<StreamAlt> = None;
    let mut waiting_for_ep = false;
    let mut i = 0;
    while i + 2 <= blob.len() {
        let len = blob[i] as usize;
        let dt = blob[i + 1];
        if len < 2 || i + len > blob.len() {
            break;
        }
        let d = &blob[i..i + len];
        match dt {
            DT_INTERFACE if len >= 9 => {
                let (num, alt, neps, cls, sub) = (d[2], d[3], d[4], d[5], d[6]);
                if cls == CLASS_AUDIO && sub == SUBCLASS_STREAMING {
                    cur = Some(StreamAlt { iface: num, alt, sync: "none", ..Default::default() });
                    waiting_for_ep = neps > 0;
                } else {
                    cur = None;
                    waiting_for_ep = false;
                }
            }
            DT_CS_INTERFACE if len >= 8 && d[2] == 2 && d[3] == 1 => {
                // FORMAT_TYPE_I
                if let Some(c) = cur.as_mut() {
                    c.channels = d[4];
                    c.subframe = d[5];
                    c.bits = d[6];
                    let n = d[7] as usize;
                    let rate = |o: usize| d[o] as u32 | (d[o + 1] as u32) << 8 | (d[o + 2] as u32) << 16;
                    c.rates = if n == 0 && len >= 14 {
                        vec![rate(8), rate(11)] // continuous range: min, max
                    } else {
                        (0..n).filter(|k| 10 + 3 * k < len).map(|k| rate(8 + 3 * k)).collect()
                    };
                }
            }
            DT_ENDPOINT if len >= 7 && waiting_for_ep => {
                let attrs = d[3];
                if attrs & 3 == 1 {
                    // isochronous
                    if let Some(mut c) = cur.take() {
                        c.ep = d[2];
                        c.max_packet = d[4] as u16 | (d[5] as u16) << 8;
                        c.sync = sync_name(attrs);
                        c.interval = d[6];
                        out.push(c);
                    }
                    waiting_for_ep = false;
                }
            }
            _ => {}
        }
        i += len;
    }
    out
}

/// The capture alt setting to use: has a rate list, prefers `prefer_rate`, then the smallest frame.
pub fn pick_capture(streams: &[StreamAlt], prefer_rate: u32) -> Option<&StreamAlt> {
    let cands: Vec<&StreamAlt> = streams.iter().filter(|s| s.is_capture() && s.channels > 0 && !s.rates.is_empty()).collect();
    let pool: Vec<&StreamAlt> = {
        let ok: Vec<&StreamAlt> = cands.iter().copied().filter(|s| s.rates.contains(&prefer_rate)).collect();
        if ok.is_empty() { cands } else { ok }
    };
    pool.into_iter().min_by_key(|s| (s.frame_bytes(), s.alt))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The REAL 334-byte descriptor blob read from the user's FM-1 on 2026-11-08.
    const REAL: &[u8] = include_bytes!("../tests/fixtures/fm1_descriptors.bin");

    #[test]
    fn real_fm1_capture_and_playback() {
        assert_eq!(REAL.len(), 334);
        assert_eq!((REAL[8] as u16 | (REAL[9] as u16) << 8, REAL[10] as u16 | (REAL[11] as u16) << 8), (0x4C4A, 0xC755));
        let s = parse_audio_streams(REAL);
        assert_eq!(s.len(), 2);
        let cap = pick_capture(&s, 44100).unwrap();
        assert_eq!((cap.iface, cap.alt, cap.ep, cap.max_packet, cap.sync), (2, 1, 0x83, 270, "async"));
        assert_eq!((cap.channels, cap.subframe, cap.bits, cap.rates.clone()), (2, 3, 24, vec![44100]));
        let play: Vec<_> = s.iter().filter(|x| !x.is_capture()).collect();
        assert_eq!(play.len(), 1);
        assert_eq!((play[0].iface, play[0].ep, play[0].sync), (1, 0x03, "adaptive"));
    }

    fn iface(num: u8, alt: u8, neps: u8, cls: u8, sub: u8) -> Vec<u8> {
        vec![9, 4, num, alt, neps, cls, sub, 0, 0]
    }
    fn fmt1(ch: u8, sub: u8, bits: u8, rates: &[u32]) -> Vec<u8> {
        let mut b = vec![8 + 3 * rates.len() as u8, 0x24, 2, 1, ch, sub, bits, rates.len() as u8];
        for r in rates {
            b.extend_from_slice(&[*r as u8, (*r >> 8) as u8, (*r >> 16) as u8]);
        }
        b
    }
    fn ep(addr: u8, attrs: u8, max: u16) -> Vec<u8> {
        vec![9, 5, addr, attrs, max as u8, (max >> 8) as u8, 1, 0, 0]
    }

    #[test]
    fn generic_mic_prefers_44100_and_smallest_frame() {
        let mut b = iface(1, 1, 1, 1, 2);
        b.extend(fmt1(1, 2, 16, &[48000, 44100]));
        b.extend(ep(0x81, 0x05, 100));
        b.extend(iface(1, 2, 1, 1, 2));
        b.extend(fmt1(2, 2, 16, &[48000, 44100]));
        b.extend(ep(0x81, 0x05, 200));
        let s = parse_audio_streams(&b);
        let cap = pick_capture(&s, 44100).unwrap();
        assert_eq!((cap.alt, cap.channels), (1, 1));
    }

    #[test]
    fn midi_and_control_interfaces_are_ignored_and_garbage_is_safe() {
        let mut b = iface(4, 0, 2, 1, 3);
        b.extend(ep(0x04, 0x02, 64));
        assert!(parse_audio_streams(&b).is_empty());
        assert!(pick_capture(&[], 44100).is_none());
        parse_audio_streams(&REAL[..REAL.len() - 5]);
        parse_audio_streams(&[0, 0, 0xff]);
        parse_audio_streams(&[]);
    }
}
