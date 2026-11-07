//! Project browser (brief feature 15): list projects with modified date, duration and size; duplicate; delete.
//!
//! A project is a folder `<root>/<name>/` holding `project.json` and `audio/`. Deleting and duplicating only
//! ever touch plain-named folders directly under `root` that contain a `project.json`.

use pocketdaw_core::edit::effective_len;
use pocketdaw_core::project::Project;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone)]
pub struct ProjectInfo {
    pub dir: PathBuf,
    /// folder name (what the browser shows and what duplicate/delete take)
    pub folder: String,
    pub title: String,
    /// seconds since the Unix epoch (project.json's modification time)
    pub modified: u64,
    /// end of the last clip or note on the timeline
    pub duration_frames: u64,
    pub rate: u32,
    pub bpm: f64,
    pub tracks_used: usize,
    /// folder size on disk including audio
    pub size_bytes: u64,
    /// set when project.json cannot be read; the other fields are then zero
    pub error: Option<String>,
}

impl ProjectInfo {
    pub fn duration_secs(&self) -> f64 {
        if self.rate == 0 { 0.0 } else { self.duration_frames as f64 / self.rate as f64 }
    }
}

/// Frames in a PCM WAV from its header alone (no decoding). Falls back to the file length when the header's
/// data size is 0, which is what a recording that was never finalised looks like.
pub fn wav_frames(path: &Path) -> Option<u64> {
    let mut f = fs::File::open(path).ok()?;
    let total = f.metadata().ok()?.len();
    let mut head = vec![0u8; 4096];
    let n = f.read(&mut head).ok()?;
    let h = &head[..n];
    if h.len() < 12 || &h[0..4] != b"RIFF" || &h[8..12] != b"WAVE" {
        return None;
    }
    let (mut i, mut frame) = (12usize, None);
    while i + 8 <= h.len() {
        let size = u32::from_le_bytes([h[i + 4], h[i + 5], h[i + 6], h[i + 7]]) as u64;
        match &h[i..i + 4] {
            b"fmt " if h.len() >= i + 8 + 16 => {
                let ch = u16::from_le_bytes([h[i + 10], h[i + 11]]) as u64;
                let bits = u16::from_le_bytes([h[i + 22], h[i + 23]]) as u64;
                frame = Some((ch * bits / 8).max(1));
            }
            b"data" => {
                let bytes = if size == 0 { total.saturating_sub(i as u64 + 8) } else { size.min(total.saturating_sub(i as u64 + 8)) };
                return frame.map(|fr| bytes / fr);
            }
            _ => {}
        }
        i += 8 + size as usize + (size as usize & 1);
    }
    None
}

fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(p) else { return 0 };
    rd.filter_map(|e| e.ok())
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0, // symlinks are not followed
        })
        .sum()
}

fn mtime_secs(p: &Path) -> u64 {
    fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

fn describe(dir: &Path, folder: &str) -> ProjectInfo {
    let json = dir.join("project.json");
    let mut info = ProjectInfo {
        dir: dir.to_path_buf(), folder: folder.to_string(), title: folder.to_string(), modified: mtime_secs(&json),
        duration_frames: 0, rate: 0, bpm: 0.0, tracks_used: 0, size_bytes: dir_size(dir), error: None,
    };
    let parsed = fs::read_to_string(&json).map_err(|e| e.to_string()).and_then(|s| Project::from_json(&s));
    match parsed {
        Ok(p) => {
            info.title = p.name.clone();
            info.rate = p.rate;
            info.bpm = p.bpm;
            let mut end = 0u64;
            for t in &p.tracks {
                let mut used = !t.clips.is_empty() || !t.notes.is_empty();
                for c in &t.clips {
                    let file_frames = wav_frames(&dir.join(&c.file)).unwrap_or(0);
                    end = end.max(c.start + effective_len(c, file_frames));
                }
                for n in &t.notes {
                    end = end.max(n.start + n.len);
                }
                used |= !t.takes.is_empty();
                info.tracks_used += used as usize;
            }
            info.duration_frames = end;
        }
        Err(e) => info.error = Some(e),
    }
    info
}

/// Every project folder under `root`, newest first. Unreadable projects are listed with `error` set.
pub fn scan(root: &Path) -> Vec<ProjectInfo> {
    let Ok(rd) = fs::read_dir(root) else { return Vec::new() };
    let mut v: Vec<ProjectInfo> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.path().join("project.json").is_file())
        .map(|e| describe(&e.path(), &e.file_name().to_string_lossy()))
        .collect();
    v.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.folder.cmp(&b.folder)));
    v
}

/// A folder name that cannot escape `root`: one plain component, no dots-only, no separators.
pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\') && !name.contains('\0') && name.len() <= 80
}

fn err(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.to_string())
}

fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let t = e.file_type()?;
        let to = dst.join(e.file_name());
        if t.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else if t.is_file() {
            fs::copy(e.path(), to)?;
        } // symlinks are skipped
    }
    Ok(())
}

/// Copy `<root>/<from>` to `<root>/<to>`. Never overwrites; on failure the partial copy is removed.
pub fn duplicate(root: &Path, from: &str, to: &str) -> io::Result<()> {
    if !is_plain_name(from) || !is_plain_name(to) {
        return Err(err("invalid project name"));
    }
    let (src, dst) = (root.join(from), root.join(to));
    if !src.join("project.json").is_file() || fs::symlink_metadata(&src)?.file_type().is_symlink() {
        return Err(err("source is not a project"));
    }
    if dst.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "a project with that name already exists"));
    }
    copy_dir(&src, &dst).inspect_err(|_| {
        let _ = fs::remove_dir_all(&dst);
    })?;
    // give the copy a recognisable title
    if let Ok(s) = fs::read_to_string(dst.join("project.json")) {
        if let Ok(mut p) = Project::from_json(&s) {
            p.name = to.chars().take(16).collect();
            let _ = fs::write(dst.join("project.json"), p.to_json());
        }
    }
    Ok(())
}

/// Delete `<root>/<name>` (recursively). Refuses anything that is not a real project folder directly under `root`.
pub fn delete(root: &Path, name: &str) -> io::Result<()> {
    if !is_plain_name(name) {
        return Err(err("invalid project name"));
    }
    let dir = root.join(name);
    let meta = fs::symlink_metadata(&dir)?;
    if !meta.is_dir() || meta.file_type().is_symlink() || !dir.join("project.json").is_file() {
        return Err(err("not a project folder"));
    }
    fs::remove_dir_all(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pocketdaw_core::project::{ClipModel, NoteModel};
    use pocketdaw_core::wav::{encode, Audio};
    use std::time::{Duration, SystemTime};

    fn audio(frames: usize) -> Vec<u8> {
        encode(&Audio { rate: 44100, data: vec![0.1; frames * 2] }, 24)
    }

    fn make(root: &Path, folder: &str, title: &str, age_secs: u64, clip_start: u64, wav_frames: usize) -> PathBuf {
        let dir = root.join(folder);
        fs::create_dir_all(dir.join("audio")).unwrap();
        fs::write(dir.join("audio/t1.wav"), audio(wav_frames)).unwrap();
        let mut p = Project { name: title.into(), ..Default::default() };
        p.tracks[0].clips.push(ClipModel { file: "audio/t1.wav".into(), start: clip_start, ..Default::default() });
        fs::write(dir.join("project.json"), p.to_json()).unwrap();
        let f = fs::OpenOptions::new().write(true).open(dir.join("project.json")).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(age_secs)).unwrap();
        dir
    }

    #[test]
    fn lists_newest_first_with_duration_size_and_track_count() {
        let d = tempfile::tempdir().unwrap();
        make(d.path(), "old", "OLD ONE", 5000, 0, 44100);
        make(d.path(), "new", "NEW ONE", 10, 22050, 88200);
        fs::create_dir(d.path().join("not-a-project")).unwrap();
        fs::write(d.path().join("stray.txt"), "x").unwrap();
        let v = scan(d.path());
        assert_eq!(v.iter().map(|p| p.folder.as_str()).collect::<Vec<_>>(), vec!["new", "old"]);
        let n = &v[0];
        assert_eq!((n.title.as_str(), n.rate, n.tracks_used), ("NEW ONE", 44100, 1));
        assert_eq!(n.duration_frames, 22050 + 88200, "clip with no length plays to the end of its file (from the WAV header)");
        assert!((n.duration_secs() - 2.5).abs() < 1e-9);
        assert!(n.size_bytes > 88200 * 6, "counts the audio");
        assert!(n.modified > 0 && n.error.is_none());
        assert_eq!(v[1].duration_frames, 44100);
    }

    #[test]
    fn notes_extend_the_duration_and_bad_projects_are_listed_with_an_error() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("n");
        fs::create_dir_all(&dir).unwrap();
        let mut p = Project::default();
        p.tracks[2].notes.push(NoteModel { start: 100_000, len: 5000, note: 60, vel: 90 });
        fs::write(dir.join("project.json"), p.to_json()).unwrap();
        let bad = d.path().join("bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("project.json"), "{broken").unwrap();
        let future = d.path().join("future");
        fs::create_dir_all(&future).unwrap();
        fs::write(future.join("project.json"), r#"{"version":99}"#).unwrap();
        let v = scan(d.path());
        assert_eq!(v.len(), 3);
        let n = v.iter().find(|p| p.folder == "n").unwrap();
        assert_eq!((n.duration_frames, n.tracks_used), (105_000, 1));
        assert!(v.iter().find(|p| p.folder == "bad").unwrap().error.is_some());
        assert!(v.iter().find(|p| p.folder == "future").unwrap().error.as_ref().unwrap().contains("v99"));
        assert!(scan(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn wav_frames_reads_headers_including_unfinalised_recordings() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.wav");
        fs::write(&f, audio(1000)).unwrap();
        assert_eq!(wav_frames(&f), Some(1000));
        // a recording that never got its header fixed: data size 0, but the bytes are there
        let mut b = audio(500);
        b[40..44].copy_from_slice(&0u32.to_le_bytes());
        fs::write(&f, b).unwrap();
        assert_eq!(wav_frames(&f), Some(500));
        fs::write(&f, b"not a wav").unwrap();
        assert_eq!(wav_frames(&f), None);
        assert_eq!(wav_frames(&d.path().join("missing.wav")), None);
    }

    #[test]
    fn duplicate_copies_everything_never_overwrites_and_rejects_bad_names() {
        let d = tempfile::tempdir().unwrap();
        make(d.path(), "song", "SONG", 100, 0, 1000);
        duplicate(d.path(), "song", "song copy").unwrap();
        assert!(d.path().join("song copy/audio/t1.wav").is_file());
        let copy = scan(d.path()).into_iter().find(|p| p.folder == "song copy").unwrap();
        assert_eq!(copy.title, "song copy");
        assert_eq!(duplicate(d.path(), "song", "song copy").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        for bad in ["", ".", "..", "a/b", "../x", "a\\b"] {
            assert!(duplicate(d.path(), "song", bad).is_err(), "{bad:?}");
            assert!(duplicate(d.path(), bad, "x").is_err(), "{bad:?}");
        }
        assert!(duplicate(d.path(), "missing", "x").is_err());
        assert!(!d.path().join("x").exists());
    }

    #[test]
    fn delete_only_removes_real_project_folders_under_the_root() {
        let d = tempfile::tempdir().unwrap();
        make(d.path(), "song", "SONG", 1, 0, 100);
        fs::create_dir(d.path().join("photos")).unwrap();
        fs::write(d.path().join("photos/keep.jpg"), "x").unwrap();
        assert!(delete(d.path(), "photos").is_err(), "a folder without project.json is not ours to delete");
        assert!(d.path().join("photos/keep.jpg").exists());
        for bad in ["..", ".", "", "../x", "song/audio"] {
            assert!(delete(d.path(), bad).is_err(), "{bad:?}");
        }
        assert!(delete(d.path(), "nothing").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.path().join("song"), d.path().join("link")).unwrap();
            assert!(delete(d.path(), "link").is_err(), "symlinks are refused");
            assert!(d.path().join("song/project.json").exists());
        }
        delete(d.path(), "song").unwrap();
        assert!(!d.path().join("song").exists());
    }
}
