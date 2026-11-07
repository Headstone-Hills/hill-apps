//! Dev tool: decode every WAV under a folder and report memory use.
//! usage: memcheck <folder>
use hill_core::wav;
use std::path::Path;

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")) {
            out.push(p);
        }
    }
}

fn main() {
    let root = std::env::args().nth(1).expect("folder");
    let mut files = Vec::new();
    walk(Path::new(&root), &mut files);
    let (mut n, mut mono, mut now, mut before, mut disk) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for f in &files {
        let Ok(b) = std::fs::read(f) else { continue };
        disk += b.len();
        if let Some(s) = wav::parse(&b) {
            n += 1;
            mono += s.is_mono_stored() as usize;
            now += s.bytes();
            before += s.frames() * 8;
        }
    }
    let mb = |x: usize| x as f64 / 1048576.0;
    println!("{} files, {} stored mono", n, mono);
    println!("on disk {:.0} MB | old in-memory (f32 stereo) {:.0} MB | now {:.0} MB ({:.1}x smaller)", mb(disk), mb(before), mb(now), before as f64 / now.max(1) as f64);
}
