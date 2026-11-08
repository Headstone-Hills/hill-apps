//! USB Audio Class 1.0 descriptor parser. Reads the blob the kernel exposes at
//! `/sys/bus/usb/devices/<dev>/descriptors` and returns each isochronous audio-streaming
//! alternate setting with the format it carries, so nothing about the FM-1 is hardcoded
//! (other firmware, or a generic USB mic, parse the same way). USB Audio Class 2.0 (protocol 0x20) is decoded too: its descriptors do not
//! list sample rates (a clock source is asked instead), so a UAC2 stream carries candidate rates and the clock to negotiate with.

const DT_INTERFACE: u8 = 0x04;
const DT_ENDPOINT: u8 = 0x05;
const DT_CS_INTERFACE: u8 = 0x24;
const DT_CS_ENDPOINT: u8 = 0x25;
const CLASS_AUDIO: u8 = 1;
const SUBCLASS_CONTROL: u8 = 1;
const SUBCLASS_STREAMING: u8 = 2;
const PROTOCOL_V2: u8 = 0x20;

/// The clock a USB Audio 2.0 device's sample rate is set on: a control request to the audio-control interface naming this clock source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Uac2Clock {
    pub clock_id: u8,
    pub ac_iface: u8,
}

/// Rates tried on a UAC2 device (in this order after the wanted one): the device accepts what it supports and refuses the rest.
pub const UAC2_CANDIDATE_RATES: [u32; 5] = [44_100, 48_000, 96_000, 88_200, 192_000];

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
    /// `Some` for USB Audio 2.0: the rate is negotiated with this clock, `rates` being only candidates
    pub uac2: Option<Uac2Clock>,
    /// microseconds between packets: 1000 x 2^(bInterval-1) at full speed, 125 x 2^(bInterval-1) at high speed (UAC2 is assumed high speed
    /// until [`StreamAlt::set_full_speed`] says the bus is not)
    pub interval_us: u32,
    /// the class-specific endpoint descriptor advertises the sampling-frequency control (UAC1): the host should set the rate on the
    /// endpoint, and firmware such as FM-1+VA sends nothing until it does
    pub rate_control: bool,
}

impl StreamAlt {
    /// true = device -> host (capture)
    pub fn is_capture(&self) -> bool {
        self.ep & 0x80 != 0
    }
    /// The bus runs at 12 Mbit/s: packets come every 1 ms x 2^(bInterval-1), not every 125 us.
    pub fn set_full_speed(&mut self) {
        self.interval_us = 1000u32 << (self.interval.clamp(1, 16) - 1).min(12);
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
    // the stream whose endpoint was just read: its class-specific endpoint descriptor follows
    let mut last_pushed: Option<usize> = None;
    // which audio class version the interface being read belongs to, and the control interface's clock (UAC2)
    let mut v2 = false;
    let mut in_ac = false;
    let mut ac_iface = 0u8;
    let mut clock: Option<u8> = None;
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
                let (num, alt, neps, cls, sub, proto) = (d[2], d[3], d[4], d[5], d[6], d[7]);
                last_pushed = None;
                v2 = cls == CLASS_AUDIO && proto == PROTOCOL_V2;
                in_ac = cls == CLASS_AUDIO && sub == SUBCLASS_CONTROL;
                if in_ac && v2 {
                    ac_iface = num;
                }
                if cls == CLASS_AUDIO && sub == SUBCLASS_STREAMING && !(v2 && clock.is_none()) {
                    cur = Some(StreamAlt { iface: num, alt, sync: "none", uac2: if v2 { clock.map(|c| Uac2Clock { clock_id: c, ac_iface }) } else { None }, ..Default::default() });
                    waiting_for_ep = neps > 0;
                } else {
                    cur = None;
                    waiting_for_ep = false;
                }
            }
            // CLOCK_SOURCE of a USB Audio 2.0 control interface: the first one is the clock the rate is set on
            DT_CS_INTERFACE if in_ac && v2 && len >= 8 && d[2] == 0x0A => {
                clock.get_or_insert(d[3]);
            }
            // UAC2 AS_GENERAL: channel count and whether the format is PCM
            DT_CS_INTERFACE if v2 && cur.is_some() && len >= 16 && d[2] == 1 => {
                if let Some(c) = cur.as_mut() {
                    let pcm = d[6] & 1 != 0;
                    c.channels = if pcm { d[10] } else { 0 };
                }
            }
            // UAC2 FORMAT_TYPE_I: bytes per sample and bits used
            DT_CS_INTERFACE if v2 && cur.is_some() && len >= 6 && d[2] == 2 && d[3] == 1 => {
                if let Some(c) = cur.as_mut() {
                    c.subframe = d[4];
                    c.bits = d[5];
                    c.rates = UAC2_CANDIDATE_RATES.to_vec();
                }
            }
            DT_CS_INTERFACE if !v2 && len >= 8 && d[2] == 2 && d[3] == 1 => {
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
                        let w = d[4] as u16 | (d[5] as u16) << 8;
                        // bits 11-12 add transactions per microframe (high-bandwidth endpoints)
                        c.max_packet = (w & 0x7ff) * (1 + ((w >> 11) & 3));
                        c.sync = sync_name(attrs);
                        c.interval = d[6];
                        let unit = if c.uac2.is_some() || v2 { 125u32 } else { 1000 };
                        c.interval_us = unit << (d[6].clamp(1, 16) - 1).min(12);
                        if c.channels > 0 && !c.rates.is_empty() {
                            out.push(c);
                            last_pushed = Some(out.len() - 1);
                        }
                    }
                    waiting_for_ep = false;
                }
            }
            // UAC1 class-specific endpoint: bmAttributes bit 0 = sampling frequency control
            DT_CS_ENDPOINT if !v2 && len >= 4 && d[2] == 1 => {
                if let Some(c) = last_pushed.and_then(|k| out.get_mut(k)) {
                    c.rate_control = d[3] & 1 != 0;
                }
                last_pushed = None;
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

    // ---- USB Audio 2.0 ----

    /// A UAC2 control interface (iface 0) with one clock source `clock`, then a streaming interface (iface 1): alt 0 empty, alt 1 carrying
    /// `ch` channels of `sub`-byte / `bits`-bit samples on endpoint `ep` (high speed, bInterval 1).
    fn uac2(clock: Option<u8>, ch: u8, sub: u8, bits: u8, pcm: bool, ep: u8) -> Vec<u8> {
        let mut b = vec![9, 4, 0, 0, 0, 1, 1, 0x20, 0]; // AC interface, protocol 0x20
        b.extend([9, 0x24, 1, 0x00, 0x02, 0x08, 0x40, 0, 0]); // header
        if let Some(c) = clock {
            b.extend([8, 0x24, 0x0A, c, 0x03, 0x07, 0, 0]); // clock source
        }
        b.extend([9, 4, 1, 0, 0, 1, 2, 0x20, 0]); // AS iface alt 0
        b.extend([9, 4, 1, 1, 1, 1, 2, 0x20, 0]); // AS iface alt 1, one endpoint
        let mut g = vec![16, 0x24, 1, 7, 0, 1, if pcm { 1 } else { 2 }, 0, 0, 0, ch, 3, 0, 0, 0, 0]; // AS_GENERAL
        g.truncate(16);
        b.extend(g);
        b.extend([6, 0x24, 2, 1, sub, bits]); // FORMAT_TYPE_I
        b.extend([7, 5, ep, 0x05, 0x40, 0x00, 1]); // endpoint: iso async, 64 bytes, bInterval 1
        b.extend([8, 0x25, 1, 0, 0, 0, 0, 0]); // class-specific endpoint (ignored)
        b
    }

    #[test]
    fn a_uac2_stream_is_decoded_with_its_clock_candidate_rates_and_125_us_packets() {
        let s = parse_audio_streams(&uac2(Some(5), 2, 4, 24, true, 0x81));
        assert_eq!(s.len(), 1, "{s:?}");
        let a = &s[0];
        assert_eq!((a.iface, a.alt, a.ep, a.max_packet, a.sync), (1, 1, 0x81, 64, "async"));
        assert_eq!((a.channels, a.subframe, a.bits), (2, 4, 24));
        assert_eq!(a.uac2, Some(Uac2Clock { clock_id: 5, ac_iface: 0 }));
        assert_eq!(a.rates, UAC2_CANDIDATE_RATES.to_vec());
        assert_eq!(a.interval_us, 125, "high speed: one packet per microframe");
        let mut full = a.clone();
        full.set_full_speed();
        assert_eq!(full.interval_us, 1000, "on a full-speed bus it is 1 ms");
        assert!(a.is_capture());
    }

    #[test]
    fn a_uac2_stream_without_a_clock_or_that_is_not_pcm_is_not_offered() {
        assert!(parse_audio_streams(&uac2(None, 2, 4, 24, true, 0x81)).is_empty(), "no clock source: the rate cannot be set");
        assert!(parse_audio_streams(&uac2(Some(5), 2, 4, 24, false, 0x81)).is_empty(), "a non-PCM format is skipped");
        parse_audio_streams(&uac2(Some(5), 2, 4, 24, true, 0x81)[..40]); // truncated: no panic
    }

    #[test]
    fn uac1_and_uac2_devices_can_share_one_blob() {
        let mut b = uac2(Some(9), 2, 3, 24, true, 0x01);
        let mut v1 = iface(3, 1, 1, 1, 2);
        v1.extend(fmt1(1, 2, 16, &[44100]));
        v1.extend(ep(0x82, 0x05, 100));
        b.extend(v1);
        let s = parse_audio_streams(&b);
        assert_eq!(s.len(), 2);
        assert!(s[0].uac2.is_some() && !s[0].is_capture());
        assert!(s[1].uac2.is_none() && s[1].interval_us == 1000 && s[1].rates == vec![44100]);
    }
}

