//! Playing HillDAW's output through a USB audio device (the FM-1, a USB DAC, headphone adapter ...).
//!
//! The SP's kernel has no USB audio driver, so a USB device is never a sound card: like recording, playback talks to it directly
//! (`hilldaw_usb::iso_out`). The audio callback that the SDL runner drives from the built-in sound card stays the master: every block it renders
//! is also pushed into a ring buffer, and a thread feeds that ring to the USB device on the USB bus clock, resampling and nudging the pace
//! a hair (`hilldaw_core::outstream`) so the two clocks never run the buffer dry or full. While the device is streaming the built-in output is
//! muted (the app zeroes the block it hands back to SDL); if the device goes away the next block plays on the built-in output again.
//!
//! The price is delay: the ring (about 35 ms) plus the USB queue (about 16 ms). That also delays what a performer hears while recording over
//! the drums, so check the latency offset on the Input/Output tab after switching.

use hilldaw_core::outstream::{adaptive_step, encode_frame, StreamResampler};
use hilldaw_usb::audio::{list_outputs, pick_output};
use hilldaw_usb::dev::SYSFS_USB;
use hilldaw_usb::iso_out::IsoPlayback;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// How much audio the ring tries to hold (frames at 44.1 kHz, about 35 ms). More is safer against a late audio callback, less is lower delay.
pub const TARGET_FRAMES: usize = 1536;
/// The most it will hold: beyond this the oldest audio is dropped (a stalled device must not make the app's memory or delay grow).
pub const MAX_FRAMES: usize = 8192;

/// Where the app's finished audio can be sent besides the built-in output.
pub trait AudioOut: Send {
    /// Use the USB device `Some("vid:pid")`, or `None` to stop and use only the built-in output. `Err` says why a device could not be used
    /// (the built-in output keeps playing).
    fn select(&mut self, id: Option<String>) -> Result<(), String>;
    /// Offer a block of rendered interleaved stereo audio (from the audio callback: must not block). `true` = the device took it, so the
    /// built-in output should be silent for this block.
    fn push(&mut self, frames: &[f32]) -> bool;
    /// The name of the device while it is streaming.
    fn active(&self) -> Option<String>;
    /// (underruns, overruns) so far, for the screen.
    fn stats(&self) -> (u64, u64);
    /// Start (`true`, clearing any earlier copy) or stop (`false`) keeping a mono copy of what is sent to the device, in the device's own
    /// frames: the reference for taking the device's echo out of a recording made through it.
    fn tap(&mut self, _on: bool) {}
    /// The copy kept since `tap(true)` (empty if there is none), and stop keeping it.
    fn take_tap(&mut self) -> Vec<f32> {
        Vec::new()
    }
}

/// The most the tap keeps: 6 minutes of 44.1 kHz mono (a longer take is only cleaned as far as this reaches).
pub const TAP_MAX: usize = 44_100 * 360;

/// The buffer between the audio callback and the USB stream.
#[derive(Default)]
pub struct OutRing {
    q: VecDeque<[f32; 2]>,
    /// false until the buffer first reaches its target (and again after an underrun): the stream plays silence meanwhile, so it starts
    /// and recovers cleanly instead of stuttering
    primed: bool,
    pub underruns: u64,
    pub overruns: u64,
}

impl OutRing {
    pub fn len(&self) -> usize {
        self.q.len()
    }

    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    pub fn push(&mut self, interleaved: &[f32]) {
        self.q.extend(interleaved.chunks_exact(2).map(|f| [f[0], f[1]]));
        if self.q.len() > MAX_FRAMES {
            let drop = self.q.len() - MAX_FRAMES;
            self.q.drain(..drop);
            self.overruns += 1;
        }
    }
}

/// Turns ring audio into the bytes of one USB packet.
pub struct Packetizer {
    rs: StreamResampler,
    /// input frames per output frame at the nominal rates (44100 / device rate)
    base: f64,
    channels: u8,
    subframe: u8,
    /// what the last `fill` sent, as mono (left + right) / 2, one value per device frame (zeros for silence): the tap's source
    pub last_mono: Vec<f32>,
}

impl Packetizer {
    pub fn new(device_rate: u32, channels: u8, subframe: u8) -> Self {
        Packetizer { rs: StreamResampler::new(), base: 44_100.0 / device_rate.max(1) as f64, channels, subframe, last_mono: Vec::new() }
    }

    /// Write `frames` device frames into `bytes`.
    pub fn fill(&mut self, ring: &mut OutRing, bytes: &mut [u8], frames: usize) {
        self.last_mono.clear();
        self.fill_inner(ring, bytes, frames);
        self.last_mono.resize(frames, 0.0); // silence (not primed, or ran dry) is part of the stream too
    }

    fn fill_inner(&mut self, ring: &mut OutRing, bytes: &mut [u8], frames: usize) {
        let fb = self.channels as usize * self.subframe as usize;
        if !ring.primed {
            if ring.q.len() >= TARGET_FRAMES {
                ring.primed = true;
            } else {
                bytes[..frames * fb].fill(0);
                return;
            }
        }
        let step = adaptive_step(self.base, ring.q.len(), TARGET_FRAMES);
        for f in 0..frames {
            let mut src = || ring.q.pop_front();
            match self.rs.pull(step, &mut src) {
                Some([l, r]) => {
                    self.last_mono.push((l + r) * 0.5);
                    encode_frame(l, r, self.channels, self.subframe, &mut bytes[f * fb..(f + 1) * fb]);
                }
                None => {
                    // ran dry: the rest of this packet is silence, and the buffer refills to its target before playing again
                    bytes[f * fb..frames * fb].fill(0);
                    ring.primed = false;
                    ring.underruns += 1;
                    return;
                }
            }
        }
    }
}

struct Shared {
    ring: Mutex<OutRing>,
    /// `Some` while a copy of the stream is being kept (see [`AudioOut::tap`])
    tap: Mutex<Option<Vec<i16>>>,
    running: AtomicBool,
    name: String,
}

struct Stream {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// The real thing: a USB device over usbfs.
pub struct UsbAudioOut {
    sysfs: PathBuf,
    stream: Option<Stream>,
    /// counters of the stream that was running when it ended
    last_stats: (u64, u64),
}

impl Default for UsbAudioOut {
    fn default() -> Self {
        Self::new()
    }
}

impl UsbAudioOut {
    pub fn new() -> Self {
        UsbAudioOut { sysfs: PathBuf::from(SYSFS_USB), stream: None, last_stats: (0, 0) }
    }

    fn stop(&mut self) {
        if let Some(s) = self.stream.take() {
            s.stop.store(true, Ordering::Relaxed);
            let _ = s.thread.join();
            if let Ok(r) = s.shared.ring.lock() {
                self.last_stats = (r.underruns, r.overruns);
            }
        }
    }
}

impl AudioOut for UsbAudioOut {
    fn select(&mut self, id: Option<String>) -> Result<(), String> {
        self.stop();
        let Some(id) = id else { return Ok(()) };
        let found = list_outputs(&self.sysfs);
        let out = pick_output(&found, &Some(id)).cloned().ok_or("the chosen output is not connected")?;
        let shared = Arc::new(Shared { ring: Mutex::new(OutRing::default()), tap: Mutex::new(None), running: AtomicBool::new(false), name: out.name.clone() });
        let stop = Arc::new(AtomicBool::new(false));
        let (sh, st) = (shared.clone(), stop.clone());
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let thread = std::thread::spawn(move || {
            // about 4 ms of packets per URB (4 at full speed, 32 at high speed's 125 us), 4 URBs in flight: about 16 ms in the USB queue
            let per_urb = (4000 / out.alt.interval_us.max(125) as usize).max(1);
            let mut pb = match IsoPlayback::open(&out.info, &out.alt, out.rate, per_urb, 4) {
                Ok(p) => {
                    let _ = ready_tx.send(Ok(()));
                    p
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
            };
            sh.running.store(true, Ordering::Relaxed);
            let mut pk = Packetizer::new(out.rate, out.alt.channels, out.alt.subframe);
            let _ = pb.run(&st, |bytes, frames| {
                if let Ok(mut ring) = sh.ring.lock() {
                    pk.fill(&mut ring, bytes, frames);
                } else {
                    bytes.fill(0);
                    pk.last_mono.clear();
                    pk.last_mono.resize(frames, 0.0);
                }
                if let Ok(mut t) = sh.tap.lock() {
                    if let Some(buf) = t.as_mut().filter(|b| b.len() < TAP_MAX) {
                        buf.extend(pk.last_mono.iter().map(|m| (m.clamp(-1.0, 1.0) * 32767.0) as i16));
                    }
                }
            });
            sh.running.store(false, Ordering::Relaxed); // unplugged, or stopped: the built-in output takes over again
        });
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => {
                self.stream = Some(Stream { shared, stop, thread });
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err("the device did not start".into())
            }
        }
    }

    fn push(&mut self, frames: &[f32]) -> bool {
        let Some(s) = &self.stream else { return false };
        if !s.shared.running.load(Ordering::Relaxed) {
            return false;
        }
        match s.shared.ring.lock() {
            Ok(mut r) => {
                r.push(frames);
                true
            }
            Err(_) => false,
        }
    }

    fn active(&self) -> Option<String> {
        self.stream.as_ref().filter(|s| s.shared.running.load(Ordering::Relaxed)).map(|s| s.shared.name.clone())
    }

    fn tap(&mut self, on: bool) {
        if let Some(s) = &self.stream {
            if let Ok(mut t) = s.shared.tap.lock() {
                *t = on.then(Vec::new);
            }
        }
    }

    fn take_tap(&mut self) -> Vec<f32> {
        let Some(s) = &self.stream else { return Vec::new() };
        let kept = s.shared.tap.lock().ok().and_then(|mut t| t.take()).unwrap_or_default();
        kept.into_iter().map(|v| v as f32 / 32767.0).collect()
    }

    fn stats(&self) -> (u64, u64) {
        match &self.stream {
            Some(s) => s.shared.ring.lock().map_or(self.last_stats, |r| (r.underruns, r.overruns)),
            None => self.last_stats,
        }
    }
}

impl Drop for UsbAudioOut {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(n: usize, v: f32) -> Vec<f32> {
        vec![v; n * 2]
    }

    fn decode24(b: &[u8]) -> Vec<(i32, i32)> {
        let s = |c: &[u8]| (((c[2] as i8) as i32) << 16) | ((c[1] as i32) << 8) | c[0] as i32;
        b.chunks(6).map(|f| (s(&f[..3]), s(&f[3..]))).collect()
    }

    #[test]
    fn the_tap_source_is_the_mono_of_what_was_sent_with_zeros_for_silence() {
        let mut ring = OutRing::default();
        let mut pk = Packetizer::new(44_100, 2, 3);
        let mut bytes = vec![0u8; 6 * 10];
        pk.fill(&mut ring, &mut bytes, 10); // not primed: silence
        assert_eq!(pk.last_mono, vec![0.0; 10]);
        let loud: Vec<f32> = (0..TARGET_FRAMES).flat_map(|_| [0.5f32, -0.25]).collect();
        ring.push(&loud);
        pk.fill(&mut ring, &mut bytes, 10);
        assert_eq!(pk.last_mono.len(), 10);
        assert!(pk.last_mono.iter().all(|m| (m - 0.125).abs() < 1e-3), "{:?}", pk.last_mono);
    }

    #[test]
    fn the_stream_is_silent_until_the_buffer_has_filled_to_its_target() {
        let mut ring = OutRing::default();
        let mut pk = Packetizer::new(44_100, 2, 3);
        let mut bytes = vec![7u8; 44 * 6];
        ring.push(&block(500, 0.5));
        pk.fill(&mut ring, &mut bytes, 44);
        assert!(bytes.iter().all(|b| *b == 0), "not enough buffered yet: silence, and nothing is consumed");
        assert_eq!(ring.len(), 500);
        ring.push(&block(TARGET_FRAMES, 0.5));
        pk.fill(&mut ring, &mut bytes, 44);
        assert!(decode24(&bytes).iter().all(|(l, r)| (*l - 4194304).abs() < 2 && (*r - 4194304).abs() < 2), "now the audio plays");
        assert_eq!(ring.underruns, 0);
    }

    #[test]
    fn running_dry_gives_silence_counts_an_underrun_and_waits_for_the_buffer_to_refill() {
        let mut ring = OutRing::default();
        let mut pk = Packetizer::new(44_100, 2, 3);
        ring.push(&block(TARGET_FRAMES + 20, 0.25));
        let mut bytes = vec![0u8; 44 * 6];
        // drain: each packet takes 44 frames
        for _ in 0..40 {
            pk.fill(&mut ring, &mut bytes, 44);
        }
        assert!(ring.underruns >= 1, "{}", ring.underruns);
        assert!(bytes.iter().all(|b| *b == 0), "dry: silence");
        let before = ring.underruns;
        ring.push(&block(300, 0.25));
        pk.fill(&mut ring, &mut bytes, 44);
        assert_eq!(ring.underruns, before, "waiting to refill is not another underrun");
        assert_eq!(ring.len(), 300, "and nothing is consumed while waiting");
        ring.push(&block(TARGET_FRAMES, 0.25));
        pk.fill(&mut ring, &mut bytes, 44);
        assert!(decode24(&bytes).iter().any(|(l, _)| *l != 0), "playing again");
    }

    #[test]
    fn a_stalled_device_cannot_make_the_buffer_grow_without_limit() {
        let mut ring = OutRing::default();
        for _ in 0..100 {
            ring.push(&block(1024, 0.1));
        }
        assert_eq!(ring.len(), MAX_FRAMES);
        assert!(ring.overruns > 0);
    }

    #[test]
    fn packets_use_the_devices_own_format_and_rate() {
        // a 48 kHz mono 16-bit device: 48 frames of 2 bytes per packet, carrying the average of left and right
        let mut ring = OutRing::default();
        let mut pk = Packetizer::new(48_000, 1, 2);
        ring.push(&(0..TARGET_FRAMES * 2).map(|i| if i % 2 == 0 { 0.4 } else { 0.2 }).collect::<Vec<f32>>());
        let mut bytes = vec![0u8; 48 * 2];
        pk.fill(&mut ring, &mut bytes, 48);
        let v: Vec<i16> = bytes.chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        assert!(v.iter().all(|s| (*s as f32 - 0.3 * 32767.0).abs() < 30.0), "{:?}", &v[..4]);
        // 48 packets of 44.1 -> 48 kHz consume 44100 input frames per second: one packet takes about 44.1
        let before = ring.len();
        pk.fill(&mut ring, &mut bytes, 48);
        let used = before - ring.len();
        assert!((43..=45).contains(&used), "one 1 ms packet at 48 kHz uses about 44.1 input frames: {used}");
    }

    #[test]
    fn the_real_output_with_no_device_fails_clearly_and_leaves_the_built_in_output_alone() {
        let mut o = UsbAudioOut::new();
        o.sysfs = std::env::temp_dir().join("hilldaw-no-such-sysfs");
        assert!(o.select(None).is_ok());
        assert_eq!(o.select(Some("1234:00ab".into())).unwrap_err(), "the chosen output is not connected");
        assert!(!o.push(&[0.0; 64]), "nothing taken: the built-in output plays");
        assert!(o.active().is_none());
        assert_eq!(o.stats(), (0, 0));
    }
}
