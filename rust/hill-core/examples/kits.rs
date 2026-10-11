//! Dev tool: show which folders under a sample root become [KIT] entries, and which
//! folders with WAVs do not (so their sounds are listed one by one).
//! usage: kits <samples root>
use hill_core::library::{kit_entries, Kind};
use std::collections::BTreeMap;
use std::path::Path;

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if p.is_dir() {
            walk(root, &p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")) {
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
}

fn main() {
    let root = std::env::args().nth(1).expect("samples root");
    let mut files = Vec::new();
    walk(Path::new(&root), Path::new(&root), &mut files);
    let kits = kit_entries(&files);
    let mut covered: BTreeMap<String, usize> = BTreeMap::new();
    for k in &kits {
        if let Kind::Kit(f) = &k.kind {
            let dir = f[0].rsplit_once('/').map(|x| x.0).unwrap_or("").to_string();
            covered.insert(dir, f.len());
        }
    }
    let mut dirs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in &files {
        let (d, n) = f.rsplit_once('/').unwrap_or(("", f));
        dirs.entry(d.to_string()).or_default().push(n.to_string());
    }
    println!("== kits ({})", kits.len());
    for (d, n) in &covered {
        println!("  KIT  {:<60} {} voices (of {} files)", d, n, dirs.get(d).map_or(0, |v| v.len()));
    }
    println!("== folders with WAVs that are NOT kits");
    for (d, v) in &dirs {
        if !covered.contains_key(d) {
            println!("  --   {:<60} {} files, e.g. {}", d, v.len(), v.iter().take(3).cloned().collect::<Vec<_>>().join(" | "));
        }
    }
}
