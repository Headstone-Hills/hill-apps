//! A read-only [`hill_core::Host`] over a plain folder of WAV files, so the drum kits can be used without
//! the SDL runner (`hill-device::FsHost` pulls in SDL). Never writes: the owner's samples are only read.

use hill_core::Host;
use std::path::{Component, Path, PathBuf};

pub struct DirHost {
    root: PathBuf,
}

const MAX_DEPTH: usize = 6;

impl DirHost {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        DirHost { root: root.into() }
    }

    /// The same search order as the Hill apps (`$HILL_SAMPLES`, then the usual SD card folders).
    pub fn detect() -> Option<Self> {
        let mut c: Vec<PathBuf> = Vec::new();
        if let Ok(p) = std::env::var("HILL_SAMPLES") {
            c.push(p.into());
        }
        for p in ["/mnt/sdcard/ROMS/Hill Apps/Samples", "/mnt/sdcard/ROMs/Samples", "/mnt/sdcard/ROMS/Samples", "/mnt/mmc/ROMS/Samples"] {
            c.push(p.into());
        }
        c.into_iter().find(|p| p.is_dir()).map(DirHost::new)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            if depth < MAX_DEPTH {
                walk(root, &p, depth + 1, out);
            }
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")) {
            if let Ok(rel) = p.strip_prefix(root) {
                out.push(rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"));
            }
        }
    }
}

impl Host for DirHost {
    fn list_samples(&self) -> Vec<String> {
        let mut v = Vec::new();
        walk(&self.root, &self.root, 0, &mut v);
        v
    }

    fn read_sample(&self, name: &str) -> Option<Vec<u8>> {
        // names come from list_samples, but never trust a path that could leave the folder
        let rel = Path::new(name);
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            return None;
        }
        std::fs::read(self.root.join(rel)).ok()
    }

    fn load_state(&self, _app: &str) -> Option<String> {
        None
    }

    fn save_state(&mut self, _app: &str, _data: &str) {}
}
