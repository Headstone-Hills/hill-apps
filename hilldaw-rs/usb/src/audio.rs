//! Any USB audio input, not just the FM-1: find every attached USB Audio Class 1.0 device that can capture, and decide how to read it.
//!
//! The kernel exposes each device's descriptors under `/sys/bus/usb/devices/<dev>/descriptors`; [`crate::desc`] decodes them into the
//! formats the device offers, and [`choose_format`] picks one. HillDAW records mono 24-bit files at 44.1 kHz, so a device that cannot do
//! 44.1 kHz is read at its own rate (48 kHz, usually) and resampled afterwards (`hilldaw-core`'s `wav` module).
//! USB Audio Class 2.0 devices (most high-speed USB-C interfaces) are not decoded yet and do not appear in the list.

use crate::desc::{parse_audio_streams, StreamAlt};
use crate::dev::DeviceInfo;
use std::fs;
use std::path::Path;

/// A USB device HillDAW can record from.
#[derive(Debug, Clone)]
pub struct AudioInput {
    /// `vid:pid` in hex, e.g. `4c4a:c755`. Two identical devices share an id; the first is used.
    pub id: String,
    pub name: String,
    pub info: DeviceInfo,
    /// the capture format that will be used
    pub alt: StreamAlt,
    /// the sample rate that will be used (44100 when the device offers it)
    pub rate: u32,
}

impl AudioInput {
    /// `2ch 24-bit 44.1 kHz`
    pub fn summary(&self) -> String {
        let khz = self.rate as f64 / 1000.0;
        format!("{}ch {}-bit {} kHz", self.alt.channels, self.alt.bits, if (khz - khz.round()).abs() < 1e-9 { format!("{khz:.0}") } else { format!("{khz:.1}") })
    }
}

/// The FM-1's USB id: the automatic choice whenever it is attached.
pub const FM1_ID: &str = "4c4a:c755";

/// The input to use: the chosen one if there is one (and `None` if it is unplugged: silently using another device would record the wrong
/// thing), else automatic = the FM-1 when it is attached, otherwise the first audio input found.
pub fn pick_input<'a>(found: &'a [AudioInput], selected: &Option<String>) -> Option<&'a AudioInput> {
    match selected {
        Some(id) => found.iter().find(|i| i.id == *id),
        None => found.iter().find(|i| i.id == FM1_ID).or_else(|| found.first()),
    }
}

/// How to read the device: the alternate setting and the rate. Prefers 44.1 kHz, then 48 kHz, then any; among equals the smallest
/// frame (least USB bandwidth). Streams that are not 16/24/32-bit PCM are skipped.
pub fn choose_format(streams: &[StreamAlt]) -> Option<(StreamAlt, u32)> {
    let mut best: Option<((u8, usize, u8), StreamAlt, u32)> = None;
    for s in streams.iter().filter(|s| s.is_capture() && s.channels > 0 && matches!(s.subframe, 2..=4) && s.bits >= 16) {
        let rate = [44_100u32, 48_000].into_iter().find(|r| s.rates.contains(r)).or_else(|| s.rates.iter().copied().filter(|r| *r >= 44_100).min()).or_else(|| s.rates.iter().copied().max());
        let Some(rate) = rate else { continue };
        let rank = match rate {
            44_100 => 0,
            48_000 => 1,
            _ => 2,
        };
        let key = (rank, s.frame_bytes(), s.alt);
        if best.as_ref().is_none_or(|(k, _, _)| key < *k) {
            best = Some((key, s.clone(), rate));
        }
    }
    best.map(|(_, s, r)| (s, r))
}

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// Every attached device with a usable audio capture stream, in bus order.
pub fn list_inputs(sysfs: &Path) -> Vec<AudioInput> {
    let Ok(rd) = fs::read_dir(sysfs) else { return Vec::new() };
    let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let d = sysfs.join(&name);
        let (Some(v), Some(p)) = (read_trim(&d.join("idVendor")), read_trim(&d.join("idProduct"))) else { continue };
        let (Some(bus), Some(dev)) = (read_trim(&d.join("busnum")).and_then(|s| s.parse().ok()), read_trim(&d.join("devnum")).and_then(|s| s.parse().ok())) else { continue };
        let Ok(blob) = fs::read(d.join("descriptors")) else { continue };
        let Some((alt, rate)) = choose_format(&parse_audio_streams(&blob)) else { continue };
        let id = format!("{}:{}", v.to_lowercase(), p.to_lowercase());
        let product = read_trim(&d.join("product")).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("USB audio {id}"));
        out.push(AudioInput {
            id,
            name: product.clone(),
            info: DeviceInfo { sysname: name, busnum: bus, devnum: dev, product, midi_iface: 0, ep_out: 0, ep_in: 0, sysdir: d },
            alt,
            rate,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FM1: &[u8] = include_bytes!("../tests/fixtures/fm1_descriptors.bin");

    fn iface(num: u8, alt: u8, neps: u8) -> Vec<u8> {
        vec![9, 4, num, alt, neps, 1, 2, 0, 0]
    }
    fn fmt1(ch: u8, sub: u8, bits: u8, rates: &[u32]) -> Vec<u8> {
        let mut b = vec![8 + 3 * rates.len() as u8, 0x24, 2, 1, ch, sub, bits, rates.len() as u8];
        for r in rates {
            b.extend_from_slice(&[*r as u8, (*r >> 8) as u8, (*r >> 16) as u8]);
        }
        b
    }
    fn ep(addr: u8, max: u16) -> Vec<u8> {
        vec![9, 5, addr, 0x05, max as u8, (max >> 8) as u8, 1, 0, 0]
    }
    /// One capture alt: iface 1, `ch` channels, `sub`-byte samples, these rates.
    fn mic(alt: u8, ch: u8, sub: u8, bits: u8, rates: &[u32]) -> Vec<u8> {
        let mut b = iface(1, alt, 1);
        b.extend(fmt1(ch, sub, bits, rates));
        b.extend(ep(0x81, 200));
        b
    }

    #[test]
    fn the_fm1_is_chosen_as_2_channel_24_bit_44_1_khz() {
        let (alt, rate) = choose_format(&parse_audio_streams(FM1)).unwrap();
        assert_eq!((alt.channels, alt.subframe, alt.bits, rate), (2, 3, 24, 44_100));
    }

    #[test]
    fn rate_preference_is_44_1_then_48_then_anything() {
        let pick = |rates: &[u32]| choose_format(&parse_audio_streams(&mic(1, 1, 2, 16, rates))).map(|(_, r)| r);
        assert_eq!(pick(&[48_000, 44_100, 96_000]), Some(44_100));
        assert_eq!(pick(&[96_000, 48_000]), Some(48_000));
        assert_eq!(pick(&[96_000, 88_200]), Some(88_200), "the lowest rate above CD quality");
        assert_eq!(pick(&[16_000, 8_000]), Some(16_000), "else the highest it has");
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn a_44_1_alt_beats_a_smaller_48_only_alt_and_equal_rates_take_the_smallest_frame() {
        let mut b = mic(1, 1, 2, 16, &[48_000]);
        b.extend(mic(2, 2, 3, 24, &[44_100, 48_000]));
        b.extend(mic(3, 1, 3, 24, &[44_100]));
        let (alt, rate) = choose_format(&parse_audio_streams(&b)).unwrap();
        assert_eq!((alt.alt, alt.channels, rate), (3, 1, 44_100), "mono 24-bit at 44.1 kHz: the smallest frame among the 44.1 kHz alts");
    }

    #[test]
    fn unusable_streams_are_skipped() {
        // 8-bit audio, and a playback-only alt (endpoint 0x01 is OUT)
        let mut b = mic(1, 1, 1, 8, &[44_100]);
        let mut out = iface(2, 1, 1);
        out.extend(fmt1(2, 2, 16, &[44_100]));
        out.extend(ep(0x01, 200));
        b.extend(out);
        assert!(choose_format(&parse_audio_streams(&b)).is_none());
        assert!(choose_format(&[]).is_none());
    }

    /// A fake sysfs entry: (folder, vid, pid, product, bus, dev) plus its descriptor blob.
    type Dev<'a> = (&'a str, &'a str, &'a str, Option<&'a str>, u32, u32);

    fn fake_device(root: &Path, (name, vid, pid, product, bus, dev): Dev, blob: &[u8]) {
        let d = root.join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("idVendor"), format!("{vid}\n")).unwrap();
        fs::write(d.join("idProduct"), format!("{pid}\n")).unwrap();
        fs::write(d.join("busnum"), format!("{bus}\n")).unwrap();
        fs::write(d.join("devnum"), format!("{dev}\n")).unwrap();
        fs::write(d.join("descriptors"), blob).unwrap();
        if let Some(p) = product {
            fs::write(d.join("product"), format!("{p}\n")).unwrap();
        }
    }

    #[test]
    fn a_sysfs_tree_lists_every_audio_input_with_its_name_and_format() {
        let root = std::env::temp_dir().join(format!("hilldaw-audio-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fake_device(&root, ("8-1", "4C4A", "C755", Some("FM-1"), 8, 2), FM1);
        fake_device(&root, ("8-2", "1234", "00ab", Some("Studio Mic"), 8, 3), &mic(1, 1, 2, 16, &[48_000]));
        fake_device(&root, ("8-3", "dead", "beef", None, 8, 4), &mic(1, 2, 3, 24, &[44_100]));
        // a device with no audio at all, a hub, and a folder that is not a device
        fake_device(&root, ("8-4", "0bda", "0001", Some("Mouse"), 8, 5), &[9, 2, 9, 0, 1, 1, 0, 0x80, 50]);
        fs::create_dir_all(root.join("usb8")).unwrap();
        let l = list_inputs(&root);
        let ids: Vec<&str> = l.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["4c4a:c755", "1234:00ab", "dead:beef"]);
        assert_eq!(l[0].name, "FM-1");
        assert_eq!(l[0].summary(), "2ch 24-bit 44.1 kHz");
        assert_eq!(l[1].name, "Studio Mic");
        assert_eq!((l[1].summary().as_str(), l[1].rate), ("1ch 16-bit 48 kHz", 48_000), "a 48 kHz-only mic is read at 48 kHz");
        assert_eq!(l[2].name, "USB audio dead:beef", "a nameless device still gets a name");
        assert_eq!((l[0].info.busnum, l[0].info.devnum), (8, 2));
        assert_eq!(l[0].info.path(), "/dev/bus/usb/008/002");
        assert!(list_inputs(&root.join("missing")).is_empty());
    }
}
