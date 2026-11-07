//! Mixdown and stem export as a background job.
//!
//! The UI thread builds a [`Snapshot`] (the clips' audio is shared, not copied; the drum track is cloned) and creates the output files;
//! a worker thread then renders it offline through a fresh `Mixer`, so live playback is untouched and the UI keeps running. Progress and
//! cancel are atomics. The render is the same mixer the audio callback uses, so the file sounds like what was heard (checked in the tests).
//!
//! * Mixdown: every strip with its level, pan, mute and solo, through the master limiter.
//! * Stems: one file per track that has audio, plus the drums, each through its own level and pan but **not** muted/soloed and
//!   **without** the limiter, so the stems add up to the mixdown (when the mix itself is not being limited).
//! * 24-bit needs no dither; 16-bit is dithered (TPDF) unless turned off. Normalizing aims the mixdown's peak at -1 dBFS and gives the stems
//!   the same gain.

use crate::recorder::ExportFile;
use hilldaw_core::drums::DrumTrack;
use hilldaw_core::export::{db, encode_pcm, normalize_gain, render_offline, Dither, Levels};
use hilldaw_core::mixer::{Mixer, MonoClip, Source, Track};
use hilldaw_core::project::ExportModel;
use hilldaw_core::record::StreamingWav;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Silence kept after the last sound so reverb tails and ringing hits are not cut off.
pub const TAIL_S: f64 = 2.0;
pub const NORMALIZE_TARGET_DB: f32 = -1.0;
const CHUNK: usize = 4096;

#[derive(Clone)]
pub(crate) struct ClipSnap {
    pub data: Arc<Vec<f32>>,
    pub start: u64,
    pub offset: u64,
    pub len: u64,
    pub gain: f32,
    pub fade_in: u64,
    pub fade_out: u64,
    pub reverse: bool,
}

#[derive(Clone)]
pub(crate) struct StripSnap {
    pub name: String,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub clips: Vec<ClipSnap>,
}

pub(crate) struct Snapshot {
    pub rate: u32,
    /// the 8 tracks, then the drums (index 8; its audio is `drums`)
    pub strips: Vec<StripSnap>,
    pub drums: Option<DrumTrack>,
    /// the last frame anything starts or sounds (before the tail)
    pub end: u64,
}

impl Snapshot {
    fn sources(&self, i: usize) -> Vec<Box<dyn Source>> {
        if i == 8 {
            return self.drums.iter().map(|d| Box::new(d.clone()) as Box<dyn Source>).collect();
        }
        self.strips[i]
            .clips
            .iter()
            .map(|c| Box::new(MonoClip { data: c.data.clone(), start: c.start, offset: c.offset, len: c.len, gain: c.gain, fade_in: c.fade_in, fade_out: c.fade_out, reverse: c.reverse }) as Box<dyn Source>)
            .collect()
    }

    pub fn has_audio(&self, i: usize) -> bool {
        if i == 8 { self.drums.is_some() } else { !self.strips[i].clips.is_empty() }
    }

    /// The mixdown (`only = None`) or one strip on its own with no limiter (a stem).
    fn mixer(&self, only: Option<usize>) -> Mixer {
        let mut m = Mixer::new(self.rate, CHUNK);
        match only {
            None => {
                for (i, st) in self.strips.iter().enumerate() {
                    let mut t = Track::new(&st.name);
                    (t.gain, t.pan, t.mute, t.solo) = (st.gain, st.pan, st.mute, st.solo);
                    t.sources = self.sources(i);
                    m.tracks.push(t);
                }
            }
            Some(i) => {
                let st = &self.strips[i];
                let mut t = Track::new(&st.name);
                (t.gain, t.pan) = (st.gain, st.pan);
                t.sources = self.sources(i);
                m.tracks.push(t);
                m.limiter.ceiling = 1.0e9; // a stem is the strip as it is; the master limiter belongs to the mixdown
            }
        }
        m
    }
}

/// An output file and what goes in it.
pub(crate) struct OutFile {
    pub file: Box<dyn ExportFile>,
    pub path: String,
    /// `None` = the mixdown, `Some(i)` = the stem of strip `i`
    pub strip: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportReport {
    /// every file written, as a path to show
    pub files: Vec<String>,
    pub seconds: f64,
    /// peak of the mixdown file, dBFS
    pub peak_db: f32,
    /// samples that went past full scale in any file (they are clamped in the file)
    pub clipped: u64,
    /// the normalization gain applied (0 when off)
    pub gain_db: f32,
    pub cancelled: bool,
}

pub(crate) struct Job {
    handle: Option<JoinHandle<Result<ExportReport, String>>>,
    pub progress: Arc<AtomicU64>,
    pub total: u64,
    pub cancel: Arc<AtomicBool>,
}

impl Job {
    pub fn finished(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }

    pub fn fraction(&self) -> f32 {
        if self.total == 0 { 1.0 } else { (self.progress.load(Ordering::Relaxed) as f32 / self.total as f32).clamp(0.0, 1.0) }
    }

    pub fn take_result(&mut self) -> Result<ExportReport, String> {
        match self.handle.take() {
            Some(h) => h.join().unwrap_or_else(|_| Err("the export thread panicked".into())),
            None => Err("already collected".into()),
        }
    }
}

/// Frames the exported files will have.
pub(crate) fn total_frames(snap: &Snapshot) -> u64 {
    snap.end + (TAIL_S * snap.rate as f64) as u64
}

pub(crate) fn spawn(snap: Snapshot, opts: ExportModel, files: Vec<OutFile>) -> Job {
    let frames = total_frames(&snap);
    let passes = files.len() as u64 + u64::from(opts.normalize);
    let progress = Arc::new(AtomicU64::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (p2, c2) = (progress.clone(), cancel.clone());
    let handle = std::thread::spawn(move || run(snap, opts, files, &p2, &c2));
    Job { handle: Some(handle), progress, total: frames * passes, cancel }
}

fn run(snap: Snapshot, opts: ExportModel, files: Vec<OutFile>, progress: &AtomicU64, cancel: &AtomicBool) -> Result<ExportReport, String> {
    let frames = total_frames(&snap);
    let bits = if opts.bits == 16 { 16 } else { 24 };
    let mut report = ExportReport { files: Vec::new(), seconds: frames as f64 / snap.rate as f64, peak_db: -120.0, clipped: 0, gain_db: 0.0, cancelled: false };

    // normalizing needs the mixdown's peak first: render it once without writing
    let mut gain = 1.0f32;
    if opts.normalize {
        let mut peak = 0.0f32;
        let done = render_offline(&mut snap.mixer(None), frames, CHUNK, |c| {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            peak = peak.max(hilldaw_core::export::peak(c));
            progress.fetch_add((c.len() / 2) as u64, Ordering::Relaxed);
            true
        });
        if !done {
            report.cancelled = true;
            return Ok(report);
        }
        gain = normalize_gain(peak, NORMALIZE_TARGET_DB);
        report.gain_db = db(gain);
    }

    for (n, of) in files.into_iter().enumerate() {
        let mut w = StreamingWav::new(of.file, 2, bits / 8, snap.rate).map_err(|e| format!("{}: {e}", of.path))?;
        let mut dither = (bits == 16 && opts.dither).then(|| Dither::new(1 + n as u64));
        let (mut pcm, mut levels, mut write_err) = (Vec::with_capacity(CHUNK * 2 * 3), Levels::default(), None);
        let done = render_offline(&mut snap.mixer(of.strip), frames, CHUNK, |c| {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            pcm.clear();
            levels.merge(encode_pcm(c, bits, gain, dither.as_mut(), &mut pcm));
            if let Err(e) = w.write(&pcm) {
                write_err = Some(e);
                return false;
            }
            progress.fetch_add((c.len() / 2) as u64, Ordering::Relaxed);
            true
        });
        if let Some(e) = write_err {
            return Err(format!("{}: {e} (is the card full?)", of.path));
        }
        w.finish().map_err(|e| format!("{}: {e}", of.path))?;
        if !done {
            report.cancelled = true;
            return Ok(report); // the partly written file is left as it is
        }
        if of.strip.is_none() {
            report.peak_db = db(levels.peak);
        }
        report.clipped += levels.clipped;
        report.files.push(of.path);
    }
    Ok(report)
}

/// A file-name-safe form of a track or project name: `TRACK 1` -> `track-1`.
pub(crate) fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let t = out.trim_end_matches('-');
    if t.is_empty() { "untitled".into() } else { t.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_plain_file_names() {
        assert_eq!(slug("TRACK 1"), "track-1");
        assert_eq!(slug("  My Song!! (v2) "), "my-song-v2");
        assert_eq!(slug("../../etc"), "etc");
        assert_eq!(slug("???"), "untitled");
    }
}
