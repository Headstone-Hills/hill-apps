//! Capturing a USB audio input (the FM-1, or any class-compliant USB mic or interface) to disk, and reading recordings back for playback.
//!
//! The app talks to a [`Recorder`] and never to USB, so everything about *which frames belong to which take* (that is
//! `hilldaw_core::session::RecordSession`) is tested without hardware. The real recorder is [`UsbRecorder`]: one continuous
//! capture per recording session (count-in, loop passes and punch windows are cut out of it afterwards as clips; the file itself is
//! never modified), mono 24-bit at 44.1 kHz, written by a separate thread through a bounded queue, with the WAV header refreshed
//! about once a second so a crash or power loss leaves a valid file that is at most a second short.

use hilldaw_core::project::is_safe_relative;
use hilldaw_core::record::{MonoMixer, StreamingWav};
use hilldaw_core::wav::{decode_mono, resample_mono};
pub use hilldaw_usb::audio::{pick_input, FM1_ID};
use hilldaw_usb::audio::{list_inputs, AudioInput};
use hilldaw_usb::dev::SYSFS_USB;
use hilldaw_usb::iso::IsoCapture;
use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const RATE: u32 = 44100;

/// What a finished recording session produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecInfo {
    /// mono frames on disk
    pub frames: u64,
    /// USB chunks the disk writer could not keep up with (0 = every captured frame is in the file)
    pub overruns: u64,
}

/// A file an export is written to.
pub trait ExportFile: Write + Seek + Send {}
impl<T: Write + Seek + Send> ExportFile for T {}

/// Capture + storage for recordings. Files live under the project folder, addressed by a relative path (`audio/rec-001.wav`).
pub trait Recorder: Send {
    /// Start one continuous capture. Returns the new file's path relative to the project folder.
    fn start(&mut self) -> Result<String, String>;
    /// Mono frames captured so far in the running session.
    fn frames(&self) -> u64;
    /// Keep capturing until at least `min_frames` have arrived (the audio lags the transport by the latency), then stop and close the file.
    fn finish(&mut self, min_frames: u64) -> Result<RecInfo, String>;
    /// A recording as mono float, for playback.
    fn load(&mut self, rel: &str) -> Option<Vec<f32>>;
    /// Which USB audio input to use: `Some("vid:pid")` for that device, `None` for automatic (the FM-1 if it is there, else the first input
    /// found). Takes effect the next time the capture is opened.
    fn select_input(&mut self, _id: Option<String>) {}
    /// Start or stop listening to the FM-1 without writing anything, so the input level can be shown before recording.
    fn monitor(&mut self, _on: bool) -> Result<(), String> {
        Ok(())
    }
    /// Loudest input sample since the last call, 0..1 (0 when nothing is being captured).
    fn level(&mut self) -> f32 {
        0.0
    }
    /// Switch to another song's folder (a slug like `song` or `song-2`): recordings, loading and exports then use it. Not while recording.
    fn set_project(&mut self, _slug: &str) {}
    /// The recordings in the current song's `audio/` folder that the app can place: (path relative to the song, length in project frames).
    fn list_recordings(&mut self) -> Vec<(String, u64)> {
        Vec::new()
    }
    /// A new file for an export, named after `stem` (a number is added rather than overwriting an earlier export).
    /// Returns the file and a path to show the user.
    fn create_export(&mut self, _stem: &str) -> Result<(Box<dyn ExportFile>, String), String> {
        Err("this storage cannot export".into())
    }
}

/// `song-mixdown.wav`, then `song-mixdown-2.wav`, ...: the first name not yet used in `dir` **with any export extension**. A WAV that was
/// converted to FLAC or MP3 is deleted, so checking only `.wav` would hand its name out again and overwrite the earlier export.
pub fn unique_export_name(dir: &Path, stem: &str) -> String {
    let taken = |n: &str| ["wav", "flac", "mp3"].iter().any(|e| dir.join(format!("{n}.{e}")).exists());
    if !taken(stem) {
        return format!("{stem}.wav");
    }
    (2u32..).map(|n| format!("{stem}-{n}")).find(|n| !taken(n)).map_or_else(|| format!("{stem}.wav"), |n| format!("{n}.wav"))
}

/// Recordings longer than this are not loaded for playback (they are held in memory as 32-bit floats).
pub const MAX_LOAD_BYTES: u64 = 600 * 1024 * 1024;

/// `rec-001.wav`, `rec-002.wav` ...: the first number not yet used in `audio_dir`.
pub fn next_take_name(audio_dir: &Path) -> String {
    let used: Vec<u32> = std::fs::read_dir(audio_dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.strip_prefix("rec-")?.strip_suffix(".wav")?.parse::<u32>().ok()
                })
                .collect()
        })
        .unwrap_or_default();
    format!("rec-{:03}.wav", used.iter().max().map_or(1, |m| m + 1))
}

/// The recordings in `<project_dir>/audio/` (`rec-NNN.wav`), oldest first, with their length in project frames from the WAV header.
/// A file that is not a readable WAV (a crash before the header was first written) is left out.
pub fn list_recordings(project_dir: &Path) -> Vec<(String, u64)> {
    let dir = project_dir.join("audio");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("rec-") && n.ends_with(".wav")).collect())
        .unwrap_or_default();
    names.sort();
    names
        .into_iter()
        .filter_map(|n| {
            let bytes = wav_header_frames(&dir.join(&n))?;
            Some((format!("audio/{n}"), bytes))
        })
        .collect()
}

/// Frames at the project rate declared by a mono/stereo PCM WAV header (`None` if it is not one): only the first 64 bytes are read.
fn wav_header_frames(path: &Path) -> Option<u64> {
    use std::io::Read;
    let mut h = [0u8; 64];
    let n = File::open(path).ok()?.read(&mut h).ok()?;
    let h = &h[..n];
    if n < 44 || &h[0..4] != b"RIFF" || &h[8..12] != b"WAVE" {
        return None;
    }
    // walk the chunks that fit in the first bytes: fmt, then data (the writer puts them first)
    let (mut i, mut channels, mut rate, mut bits, mut data) = (12usize, 0u32, 0u32, 0u32, None);
    while i + 8 <= h.len() {
        let size = u32::from_le_bytes([h[i + 4], h[i + 5], h[i + 6], h[i + 7]]) as u64;
        match &h[i..i + 4] {
            b"fmt " if i + 24 <= h.len() => {
                channels = u16::from_le_bytes([h[i + 10], h[i + 11]]) as u32;
                rate = u32::from_le_bytes([h[i + 12], h[i + 13], h[i + 14], h[i + 15]]);
                bits = u16::from_le_bytes([h[i + 22], h[i + 23]]) as u32;
            }
            b"data" => {
                data = Some(size);
                break;
            }
            _ => {}
        }
        i += 8 + size as usize + (size as usize & 1);
    }
    let (data, frame) = (data?, (channels * bits / 8) as u64);
    if frame == 0 || rate == 0 {
        return None;
    }
    let file_len = std::fs::metadata(path).ok()?.len();
    // a header that was not finished (power cut) can claim more or less than is there: trust the smaller of header and file
    let data = data.min(file_len.saturating_sub(44));
    let frames = data / frame;
    (frames > 0).then_some(frames * RATE as u64 / rate as u64)
}

/// Read a recording under `project_dir`, refusing paths that could leave the folder and files too big to hold in memory.
pub fn load_recording(project_dir: &Path, rel: &str) -> Option<Vec<f32>> {
    if !is_safe_relative(rel) {
        return None;
    }
    let path = project_dir.join(rel);
    if std::fs::metadata(&path).ok()?.len() > MAX_LOAD_BYTES {
        eprintln!("hilldaw: {rel} is too long to load for playback");
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let (rate, mono) = decode_mono(&bytes).ok()?;
    if rate == 0 || rate > 384_000 {
        return None;
    }
    // a take from a device that cannot do 44.1 kHz was captured at its own rate (48 kHz usually); bring it to the project rate once,
    // here, so the rest of the app works in project frames
    Some(if rate == RATE { mono } else { resample_mono(&mono, rate, RATE) })
}

/// The pieces of a running capture, before it is filed as the recording or the monitor.
type Launched = (Arc<AtomicBool>, Arc<AtomicU64>, Arc<AtomicU64>, JoinHandle<Result<(), String>>, u32);
/// What the capture thread reports once the device is open: the rate it is really streaming at.
type Ready = Result<u32, String>;

struct Active {
    stop: Arc<AtomicBool>,
    captured: Arc<AtomicU64>,
    overruns: Arc<AtomicU64>,
    capture: JoinHandle<Result<(), String>>,
    /// `None` while only monitoring
    writer: Option<JoinHandle<u64>>,
    /// the device's sample rate (frames counted by `captured` and written by the writer are at this rate)
    rate: u32,
}

/// The FM-1 over USB, written under `<project_dir>/audio/`.
pub struct UsbRecorder {
    project_dir: PathBuf,
    /// the folder that holds every song (`<root>/<slug>/`), when songs can be switched
    root: Option<PathBuf>,
    active: Option<Active>,
    /// listening only (no file)
    monitor: Option<Active>,
    /// the input chosen by the user (`vid:pid`); `None` = automatic
    selected: Option<String>,
    /// where USB devices are listed (the real sysfs; tests use a temp folder)
    sysfs: PathBuf,
    /// loudest sample since the last `level()`, in 24-bit units
    peak: Arc<AtomicU64>,
}

impl UsbRecorder {
    pub fn new(project_dir: impl Into<PathBuf>) -> Self {
        UsbRecorder { project_dir: project_dir.into(), root: None, active: None, monitor: None, selected: None, sysfs: PathBuf::from(SYSFS_USB), peak: Arc::new(AtomicU64::new(0)) }
    }
}

impl UsbRecorder {
    /// Songs live in `<root>/<slug>/`; start in `slug`.
    pub fn with_root(root: impl Into<PathBuf>, slug: &str) -> Self {
        let root = root.into();
        let mut r = Self::new(root.join(slug));
        r.root = Some(root);
        r
    }
}

impl UsbRecorder {
    /// Open the chosen input's capture stream and run it on its own thread. With a writer the audio goes to disk too; without one it is only
    /// measured (the input meter). The iso handle is created on the capture thread and never leaves it.
    fn launch(&self, input: AudioInput, tx: Option<mpsc::SyncSender<Vec<u8>>>) -> Result<Launched, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let captured = Arc::new(AtomicU64::new(0));
        let overruns = Arc::new(AtomicU64::new(0));
        let (st, cf, ov, pk) = (stop.clone(), captured.clone(), overruns.clone(), self.peak.clone());
        let (ready_tx, ready_rx) = mpsc::channel::<Ready>();
        // about 16 ms per URB: 16 packets at full speed's 1 ms, 128 (the most usbfs allows) at high speed's 125 us
        let per_urb = (16_000 / input.alt.interval_us.max(125) as usize).clamp(1, 128);
        let capture = std::thread::spawn(move || {
            let mut cp = match IsoCapture::open_with_rate(&input.info, &input.alt, Some(input.rate), per_urb, 8).map_err(|e| e.to_string()) {
                Ok(c) => {
                    let _ = ready_tx.send(Ok(c.rate));
                    c
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e.clone()));
                    return Err(e);
                }
            };
            let mut mix = MonoMixer::new(input.alt.channels, input.alt.subframe);
            let mut scratch = Vec::with_capacity(512);
            cp.run(&st, 0.0, |p| {
                scratch.clear();
                mix.push(p, &mut scratch);
                cf.fetch_add((scratch.len() / 3) as u64, Ordering::Relaxed);
                pk.fetch_max(peak_24(&scratch), Ordering::Relaxed);
                if let Some(tx) = &tx {
                    if tx.try_send(scratch.clone()).is_err() {
                        ov.fetch_add(1, Ordering::Relaxed); // the writer fell behind: a chunk was dropped
                    }
                }
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
        });
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(rate)) => Ok((stop, captured, overruns, capture, rate)),
            Ok(Err(e)) => {
                let _ = capture.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err("the input did not start streaming".into())
            }
        }
    }

    /// The input that would be used now, or why there is none.
    fn pick(&self) -> Result<AudioInput, String> {
        let found = list_inputs(&self.sysfs);
        match pick_input(&found, &self.selected) {
            Some(i) => Ok(i.clone()),
            None if self.selected.is_some() => Err("the chosen input is not connected".into()),
            None => Err("no USB audio input found".into()),
        }
    }

    fn stop_monitor(&mut self) {
        if let Some(m) = self.monitor.take() {
            m.stop.store(true, Ordering::Relaxed);
            let _ = m.capture.join();
        }
    }
}

/// Loudest sample in a run of 24-bit little-endian mono bytes (absolute value, 24-bit units).
pub(crate) fn peak_24(bytes: &[u8]) -> u64 {
    bytes.chunks_exact(3).map(|p| (((p[2] as i8) as i32) << 16 | (p[1] as i32) << 8 | p[0] as i32).unsigned_abs() as u64).max().unwrap_or(0)
}

impl Recorder for UsbRecorder {
    fn start(&mut self) -> Result<String, String> {
        if self.active.is_some() {
            return Err("already recording".into());
        }
        let input = self.pick()?;
        self.stop_monitor(); // the interface can be claimed once: recording takes over from monitoring
        let audio_dir = self.project_dir.join("audio");
        std::fs::create_dir_all(&audio_dir).map_err(|e| format!("cannot create {}: {e}", audio_dir.display()))?;
        let name = next_take_name(&audio_dir);
        let file = File::create(audio_dir.join(&name)).map_err(|e| format!("cannot create {name}: {e}"))?;

        // capture thread -> bounded queue -> writer thread -> SD card
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(512); // ~512 USB-sized chunks of slack
        // the file's header needs the rate the device really runs at (a USB Audio 2.0 device settles it when it opens)
        let (rate_tx, rate_rx) = mpsc::channel::<u32>();
        let writer = std::thread::spawn(move || {
            let Ok(dev_rate) = rate_rx.recv() else { return 0 }; // the capture failed to open: nothing to write
            let Ok(mut w) = StreamingWav::new(BufWriter::with_capacity(256 * 1024, file), 1, 3, dev_rate) else { return 0 };
            let mut last_flush = Instant::now();
            for chunk in rx {
                if w.write(&chunk).is_err() {
                    break; // disk full or card removed: stop writing, keep what is there
                }
                if last_flush.elapsed() >= Duration::from_secs(1) {
                    let _ = w.flush_header(); // a crash now loses at most about a second
                    last_flush = Instant::now();
                }
            }
            let frames = w.data_len() / 3;
            let _ = w.finish();
            frames
        });
        match self.launch(input, Some(tx)) {
            Ok((stop, captured, overruns, capture, rate)) => {
                let _ = rate_tx.send(rate);
                self.active = Some(Active { stop, captured, overruns, capture, writer: Some(writer), rate });
                Ok(format!("audio/{name}"))
            }
            Err(e) => {
                drop(rate_tx);
                let _ = writer.join(); // the writer stops: nothing was captured
                Err(e)
            }
        }
    }

    fn select_input(&mut self, id: Option<String>) {
        self.selected = id;
    }

    fn set_project(&mut self, slug: &str) {
        if let (Some(root), true) = (&self.root, hilldaw_core::project::is_safe_relative(slug) && !slug.contains('/')) {
            self.project_dir = root.join(slug);
        }
    }

    fn list_recordings(&mut self) -> Vec<(String, u64)> {
        list_recordings(&self.project_dir)
    }

    fn monitor(&mut self, on: bool) -> Result<(), String> {
        if !on {
            self.stop_monitor();
            return Ok(());
        }
        if self.monitor.is_some() || self.active.is_some() {
            return Ok(());
        }
        let input = self.pick()?;
        let (stop, captured, overruns, capture, rate) = self.launch(input, None)?;
        self.monitor = Some(Active { stop, captured, overruns, capture, writer: None, rate });
        Ok(())
    }

    fn level(&mut self) -> f32 {
        self.peak.swap(0, Ordering::Relaxed) as f32 / 8_388_608.0
    }

    fn frames(&self) -> u64 {
        // project-rate frames, whatever rate the device runs at
        self.active.as_ref().map_or(0, |a| a.captured.load(Ordering::Relaxed) * RATE as u64 / a.rate.max(1) as u64)
    }

    fn finish(&mut self, min_frames: u64) -> Result<RecInfo, String> {
        let a = self.active.take().ok_or("not recording")?;
        // the captured audio lags the transport: keep reading until the frames the takes need have arrived (or the stream stalls)
        let need = (min_frames * a.rate as u64).div_ceil(RATE as u64); // in the device's own frames
        let t0 = Instant::now();
        while a.captured.load(Ordering::Relaxed) < need && t0.elapsed() < Duration::from_millis(1500) {
            std::thread::sleep(Duration::from_millis(5));
        }
        a.stop.store(true, Ordering::Relaxed);
        let cap = a.capture.join().map_err(|_| "capture thread panicked".to_string())?;
        let frames = match a.writer {
            Some(w) => w.join().map_err(|_| "writer thread panicked".to_string())?,
            None => 0,
        };
        cap?;
        // the file is at the device's rate; the app counts in project frames (the file is resampled to match when it is loaded)
        let frames = frames * RATE as u64 / a.rate.max(1) as u64;
        Ok(RecInfo { frames, overruns: a.overruns.load(Ordering::Relaxed) })
    }

    fn load(&mut self, rel: &str) -> Option<Vec<f32>> {
        load_recording(&self.project_dir, rel)
    }

    fn create_export(&mut self, stem: &str) -> Result<(Box<dyn ExportFile>, String), String> {
        // the stem comes from the project name and track names: keep it to a plain file name
        let stem: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
        let dir = self.project_dir.join("export");
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let path = dir.join(unique_export_name(&dir, &stem));
        let f = File::create(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        Ok((Box::new(BufWriter::with_capacity(256 * 1024, f)), path.display().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hilldaw_core::wav::{encode, Audio};

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hilldaw-rec-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn take_names_count_up_from_the_highest_existing_one() {
        let d = tmp("names");
        assert_eq!(next_take_name(&d), "rec-001.wav");
        std::fs::write(d.join("rec-001.wav"), b"x").unwrap();
        std::fs::write(d.join("rec-007.wav"), b"x").unwrap();
        std::fs::write(d.join("notes.txt"), b"x").unwrap();
        std::fs::write(d.join("rec-abc.wav"), b"x").unwrap();
        assert_eq!(next_take_name(&d), "rec-008.wav", "never reuses a number, even with gaps");
        assert_eq!(next_take_name(&d.join("missing")), "rec-001.wav");
    }

    #[test]
    fn recordings_load_as_mono_and_unsafe_or_wrong_files_are_refused() {
        let d = tmp("load");
        std::fs::create_dir_all(d.join("audio")).unwrap();
        let a = Audio { rate: 44100, data: vec![0.25; 2000] };
        std::fs::write(d.join("audio/rec-001.wav"), encode(&a, 24)).unwrap();
        let m = load_recording(&d, "audio/rec-001.wav").unwrap();
        assert_eq!(m.len(), 1000);
        assert!((m[10] - 0.25).abs() < 1e-5);
        assert!(load_recording(&d, "../audio/rec-001.wav").is_none(), "cannot leave the project folder");
        assert!(load_recording(&d, "/etc/hosts").is_none());
        assert!(load_recording(&d, "audio/missing.wav").is_none());
        // a take from a 48 kHz-only device is brought to the project rate when it is loaded: 4800 frames at 48 kHz = 4410 at 44.1 kHz
        let tone = Audio { rate: 48000, data: vec![0.5; 9600] };
        std::fs::write(d.join("audio/rec-002.wav"), encode(&tone, 16)).unwrap();
        let r = load_recording(&d, "audio/rec-002.wav").unwrap();
        assert_eq!(r.len(), 4410);
        assert!((r[2000] - 0.5).abs() < 0.01, "level kept");
        std::fs::write(d.join("audio/rec-004.wav"), encode(&Audio { rate: 0, data: vec![0.0; 200] }, 16)).ok();
        std::fs::write(d.join("audio/rec-003.wav"), b"not a wav").unwrap();
        assert!(load_recording(&d, "audio/rec-003.wav").is_none());
    }

    fn input(id: &str, name: &str) -> AudioInput {
        use hilldaw_usb::desc::StreamAlt;
        use hilldaw_usb::dev::DeviceInfo;
        AudioInput {
            id: id.into(),
            name: name.into(),
            info: DeviceInfo { sysname: "x".into(), busnum: 1, devnum: 2, product: name.into(), midi_iface: 0, ep_out: 0, ep_in: 0, sysdir: PathBuf::new() },
            alt: StreamAlt { channels: 1, subframe: 2, bits: 16, rates: vec![48000], ep: 0x81, ..Default::default() },
            rate: 48000,
        }
    }

    #[test]
    fn automatic_input_prefers_the_fm1_then_the_first_and_a_chosen_one_is_never_swapped() {
        let list = vec![input("1234:00ab", "Mic"), input(FM1_ID, "FM-1")];
        assert_eq!(pick_input(&list, &None).unwrap().id, FM1_ID, "automatic: the FM-1 wins even when listed second");
        assert_eq!(pick_input(&list[..1], &None).unwrap().id, "1234:00ab", "no FM-1: the first input");
        assert_eq!(pick_input(&list, &Some("1234:00ab".into())).unwrap().id, "1234:00ab", "a chosen device, even with the FM-1 present");
        assert!(pick_input(&list, &Some("dead:beef".into())).is_none(), "a chosen device that is unplugged is not replaced by another");
        assert!(pick_input(&[], &None).is_none());
    }

    #[test]
    fn an_unplugged_or_missing_input_is_reported_clearly() {
        let mut r = UsbRecorder::new(tmp("pick"));
        r.sysfs = tmp("empty-sysfs");
        assert_eq!(r.start().unwrap_err(), "no USB audio input found");
        r.select_input(Some("1234:00ab".into()));
        assert_eq!(r.start().unwrap_err(), "the chosen input is not connected");
        assert!(r.monitor(true).is_err());
        assert!(r.monitor(false).is_ok());
    }

    #[test]
    fn export_files_are_named_uniquely_and_never_leave_the_export_folder() {
        let d = tmp("export");
        let mut r = UsbRecorder::new(&d);
        let (mut f, path) = r.create_export("song-mixdown").unwrap();
        f.write_all(b"x").unwrap();
        drop(f);
        assert!(path.ends_with("export/song-mixdown.wav"), "{path}");
        let (_, path2) = r.create_export("song-mixdown").unwrap();
        assert!(path2.ends_with("song-mixdown-2.wav"), "{path2}");
        let (_, path3) = r.create_export("../../etc/passwd").unwrap();
        assert!(path3.starts_with(d.join("export").to_str().unwrap()) && !path3.contains(".."), "{path3}");
    }

    #[test]
    fn without_an_input_start_fails_cleanly_and_finish_without_start_is_an_error() {
        let mut r = UsbRecorder::new(tmp("nodev"));
        r.sysfs = tmp("nodev-sysfs"); // never touch the real USB bus in a test
        let e = r.start().unwrap_err();
        assert!(e.contains("no USB audio input"), "{e}");
        assert_eq!(r.frames(), 0);
        assert!(r.finish(0).is_err());
    }

    #[test]
    fn recordings_are_listed_with_their_length_in_project_frames_including_a_crashed_one() {
        let d = tmp("list");
        let audio = d.join("audio");
        std::fs::create_dir_all(&audio).unwrap();
        // a finished 48 kHz mono 24-bit file of 2 s
        let mut w = StreamingWav::new(File::create(audio.join("rec-001.wav")).unwrap(), 1, 3, 48_000).unwrap();
        w.write(&vec![1u8; 3 * 96_000]).unwrap();
        w.finish().unwrap();
        // one that was running when the power failed: the header was refreshed after 1 s of audio, then 1 more second arrived
        let mut w = StreamingWav::new(File::create(audio.join("rec-002.wav")).unwrap(), 1, 3, 44_100).unwrap();
        w.write(&vec![1u8; 3 * 44_100]).unwrap();
        w.flush_header().unwrap();
        w.write(&vec![1u8; 3 * 44_100]).unwrap();
        std::mem::forget(w); // no finish: whatever reached the file stays, the header still says 1 s
        // not recordings, or not usable
        std::fs::write(audio.join("notes.txt"), b"x").unwrap();
        std::fs::write(audio.join("rec-003.wav"), b"RIFF").unwrap();
        let mut w = StreamingWav::new(File::create(audio.join("rec-004.wav")).unwrap(), 1, 3, 44_100).unwrap();
        w.write(&[]).unwrap();
        w.finish().unwrap(); // an empty capture
        let l = list_recordings(&d);
        assert_eq!(l.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["audio/rec-001.wav", "audio/rec-002.wav"], "{l:?}");
        assert_eq!(l[0].1, 88_200, "2 s at 48 kHz is 88 200 frames at the project rate");
        assert_eq!(l[1].1, 44_100, "the crashed file counts the second the header knows about");
        assert!(list_recordings(&d.join("missing")).is_empty());
    }

    #[test]
    fn songs_are_separate_folders_under_the_root_and_a_bad_name_cannot_leave_it() {
        let d = tmp("songs");
        let mut r = UsbRecorder::with_root(&d, "song");
        assert_eq!(r.project_dir, d.join("song"));
        r.set_project("song-2");
        assert_eq!(r.project_dir, d.join("song-2"));
        r.set_project("../elsewhere");
        r.set_project("a/b");
        assert_eq!(r.project_dir, d.join("song-2"), "unsafe names are ignored");
        // a recorder made for one fixed folder ignores song switches
        let mut fixed = UsbRecorder::new(d.join("only"));
        fixed.set_project("song-2");
        assert_eq!(fixed.project_dir, d.join("only"));
    }
}

