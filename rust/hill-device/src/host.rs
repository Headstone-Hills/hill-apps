//! Filesystem host: samples from the SD card, state next to the executable.

use hill_core::Host;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_FILES: usize = 50_000;
const MAX_DEPTH: usize = 8;

pub struct FsHost {
    pub sample_root: Option<PathBuf>,
    pub data_dir: PathBuf,
}

pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

impl FsHost {
    pub fn detect() -> Self {
        let exe = exe_dir();
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(p) = std::env::var("HILL_SAMPLES") {
            candidates.push(PathBuf::from(p));
        }
        for p in ["/mnt/sdcard/ROMs/Samples", "/mnt/sdcard/ROMS/Samples", "/mnt/mmc/ROMS/Samples"] {
            candidates.push(PathBuf::from(p));
        }
        candidates.push(exe.join("samples"));
        candidates.push(PathBuf::from("samples"));
        let sample_root = candidates.into_iter().find(|p| p.is_dir());
        let data_dir = std::env::var("HILL_DATA").map(PathBuf::from).unwrap_or(exe);
        FsHost { sample_root, data_dir }
    }
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // hidden files and macOS ._ sidecars
        }
        let path = e.path();
        if path.is_dir() {
            walk(root, &path, depth + 1, out);
        } else if name.to_lowercase().ends_with(".wav") {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

impl Host for FsHost {
    fn list_samples(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(root) = &self.sample_root {
            walk(root, root, 0, &mut out);
        }
        out
    }

    fn read_sample(&self, name: &str) -> Option<Vec<u8>> {
        if name.contains("..") {
            return None;
        }
        fs::read(self.sample_root.as_ref()?.join(name)).ok()
    }

    fn load_state(&self, app: &str) -> Option<String> {
        fs::read_to_string(self.data_dir.join(format!("{}_state.txt", app))).ok()
    }

    fn save_state(&mut self, app: &str, data: &str) {
        let path = self.data_dir.join(format!("{}_state.txt", app));
        let tmp = self.data_dir.join(format!("{}_state.txt.tmp", app));
        if fs::write(&tmp, data).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_wavs_recursively_skipping_hidden() {
        let dir = std::env::temp_dir().join(format!("hill_host_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Kit/Kick")).unwrap();
        fs::write(dir.join("Kit/Kick/a.wav"), b"x").unwrap();
        fs::write(dir.join("Kit/Kick/._a.wav"), b"x").unwrap();
        fs::write(dir.join("Kit/readme.txt"), b"x").unwrap();
        fs::write(dir.join("top.WAV"), b"x").unwrap();
        let h = FsHost { sample_root: Some(dir.clone()), data_dir: dir.clone() };
        let mut l = h.list_samples();
        l.sort();
        assert_eq!(l, vec!["Kit/Kick/a.wav".to_string(), "top.WAV".to_string()]);
        assert_eq!(h.read_sample("Kit/Kick/a.wav"), Some(b"x".to_vec()));
        assert_eq!(h.read_sample("../etc/passwd"), None);
        let mut h = h;
        h.save_state("t", "a=1\n");
        assert_eq!(h.load_state("t").as_deref(), Some("a=1\n"));
        let _ = fs::remove_dir_all(&dir);
    }
}
