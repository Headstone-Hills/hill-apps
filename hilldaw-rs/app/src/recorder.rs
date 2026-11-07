//! Capturing the FM-1 to disk, and reading recordings back for playback.
//!
//! The app talks to a [`Recorder`] and never to USB, so everything about *which frames belong to which take* (that is
//! `hilldaw_core::session::RecordSession`) is tested without hardware. The real recorder is [`UsbRecorder`]: one continuous
//! capture per recording session (count-in, loop passes and punch windows are cut out of it afterwards as clips; the file itself is
//! never modified), mono 24-bit at 44.1 kHz, written by a separate thread through a bounded queue, with the WAV header refreshed
//! about once a second so a crash or power loss leaves a valid file that is at most a second short.

use hilldaw_core::project::is_safe_relative;
use hilldaw_core::record::{Downmixer, StreamingWav};
use hilldaw_core::wav::decode_mono;
use hilldaw_usb::desc::{parse_audio_streams, pick_capture};
use hilldaw_usb::dev::{find_devices, SYSFS_USB};
use hilldaw_usb::iso::IsoCapture;
use hilldaw_usb::{FM1_PID, FM1_VID};
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
    /// A new file for an export, named after `stem` (a number is added rather than overwriting an earlier export).
    /// Returns the file and a path to show the user.
    fn create_export(&mut self, _stem: &str) -> Result<(Box<dyn ExportFile>, String), String> {
        Err("this storage cannot export".into())
    }
}

/// `song-mixdown.wav`, then `song-mixdown-2.wav`, ...: the first name not yet used in `dir`.
pub fn unique_export_name(dir: &Path, stem: &str) -> String {
    let first = format!("{stem}.wav");
    if !dir.join(&first).exists() {
        return first;
    }
    (2u32..).map(|n| format!("{stem}-{n}.wav")).find(|n| !dir.join(n).exists()).unwrap_or(first)
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
    if rate != RATE {
        eprintln!("hilldaw: {rel} is {rate} Hz, expected {RATE}");
        return None;
    }
    Some(mono)
}

struct Active {
    stop: Arc<AtomicBool>,
    captured: Arc<AtomicU64>,
    overruns: Arc<AtomicU64>,
    capture: JoinHandle<Result<(), String>>,
    writer: JoinHandle<u64>,
}

/// The FM-1 over USB, written under `<project_dir>/audio/`.
pub struct UsbRecorder {
    project_dir: PathBuf,
    active: Option<Active>,
}

impl UsbRecorder {
    pub fn new(project_dir: impl Into<PathBuf>) -> Self {
        UsbRecorder { project_dir: project_dir.into(), active: None }
    }
}

impl Recorder for UsbRecorder {
    fn start(&mut self) -> Result<String, String> {
        if self.active.is_some() {
            return Err("already recording".into());
        }
        let info = find_devices(FM1_VID, FM1_PID, Path::new(SYSFS_USB)).into_iter().next().ok_or("FM-1 not found")?;
        let audio_dir = self.project_dir.join("audio");
        std::fs::create_dir_all(&audio_dir).map_err(|e| format!("cannot create {}: {e}", audio_dir.display()))?;
        let name = next_take_name(&audio_dir);
        let file = File::create(audio_dir.join(&name)).map_err(|e| format!("cannot create {name}: {e}"))?;

        // capture thread -> bounded queue -> writer thread -> SD card
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(512); // ~512 USB-sized chunks of slack
        let writer = std::thread::spawn(move || {
            let Ok(mut w) = StreamingWav::new(BufWriter::with_capacity(256 * 1024, file), 1, 3, RATE) else { return 0 };
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

        let stop = Arc::new(AtomicBool::new(false));
        let captured = Arc::new(AtomicU64::new(0));
        let overruns = Arc::new(AtomicU64::new(0));
        let (st, cf, ov) = (stop.clone(), captured.clone(), overruns.clone());
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let capture = std::thread::spawn(move || {
            // the iso handle is created on this thread and never leaves it
            let streams = parse_audio_streams(&info.descriptors().unwrap_or_default());
            let opened = match pick_capture(&streams, RATE) {
                Some(cap) => IsoCapture::open(&info, cap, 16, 8).map_err(|e| e.to_string()),
                None => Err("no capture endpoint".to_string()),
            };
            let mut cp = match opened {
                Ok(c) => {
                    let _ = ready_tx.send(Ok(()));
                    c
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e.clone()));
                    return Err(e);
                }
            };
            let mut dm = Downmixer::default();
            let mut scratch = Vec::with_capacity(512);
            cp.run(&st, 0.0, |p| {
                scratch.clear();
                dm.push(p, &mut scratch);
                cf.fetch_add((scratch.len() / 3) as u64, Ordering::Relaxed);
                if tx.try_send(scratch.clone()).is_err() {
                    ov.fetch_add(1, Ordering::Relaxed); // the writer fell behind: a chunk was dropped
                }
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
        });
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = capture.join();
                let _ = writer.join();
                return Err(e);
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                return Err("the FM-1 did not start streaming".into());
            }
        }
        self.active = Some(Active { stop, captured, overruns, capture, writer });
        Ok(format!("audio/{name}"))
    }

    fn frames(&self) -> u64 {
        self.active.as_ref().map_or(0, |a| a.captured.load(Ordering::Relaxed))
    }

    fn finish(&mut self, min_frames: u64) -> Result<RecInfo, String> {
        let a = self.active.take().ok_or("not recording")?;
        // the captured audio lags the transport: keep reading until the frames the takes need have arrived (or the stream stalls)
        let t0 = Instant::now();
        while a.captured.load(Ordering::Relaxed) < min_frames && t0.elapsed() < Duration::from_millis(1500) {
            std::thread::sleep(Duration::from_millis(5));
        }
        a.stop.store(true, Ordering::Relaxed);
        let cap = a.capture.join().map_err(|_| "capture thread panicked".to_string())?;
        let frames = a.writer.join().map_err(|_| "writer thread panicked".to_string())?;
        cap?;
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
        std::fs::write(d.join("audio/rec-002.wav"), encode(&Audio { rate: 48000, data: vec![0.0; 200] }, 16)).unwrap();
        assert!(load_recording(&d, "audio/rec-002.wav").is_none(), "wrong sample rate");
        std::fs::write(d.join("audio/rec-003.wav"), b"not a wav").unwrap();
        assert!(load_recording(&d, "audio/rec-003.wav").is_none());
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
    fn without_the_fm1_start_fails_cleanly_and_finish_without_start_is_an_error() {
        let mut r = UsbRecorder::new(tmp("nodev"));
        let e = r.start().unwrap_err();
        assert!(e.contains("FM-1") || e.contains("not found"), "{e}");
        assert_eq!(r.frames(), 0);
        assert!(r.finish(0).is_err());
    }
}
