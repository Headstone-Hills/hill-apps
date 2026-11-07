//! Dev tool: which kit and sounds each preset gets, for a list of sample paths.
//! usage: preset_kits <file with one sample path per line, relative to the samples root>
//! (on the SP: `cd "/mnt/sdcard/ROMS/Hill Apps/Samples" && find . -iname "*.wav"`)
use hill_core::library::{kit_entries, Kind};
use hill_core::presets::{builtin, Kit, Sound};

fn main() {
    let list = std::fs::read_to_string(std::env::args().nth(1).expect("path list")).expect("read list");
    let paths: Vec<String> = list.lines().map(|l| l.trim().trim_start_matches("./").to_string()).filter(|l| !l.is_empty()).collect();
    let kits: Vec<Kit> = kit_entries(&paths)
        .into_iter()
        .filter_map(|e| match e.kind {
            Kind::Kit(files) => {
                let dir = e.key.strip_prefix("kit:").unwrap_or(&e.key).to_string();
                Some(Kit::new(dir.rsplit('/').next().unwrap_or(&dir), &files))
            }
            _ => None,
        })
        .collect();
    println!("{} kits", kits.len());
    for x in &builtin().patterns {
        let plan = x.plan(&kits, 8);
        let sounds: Vec<String> = plan
            .voices
            .iter()
            .filter(|v| !v.rows.is_empty())
            .map(|v| {
                let s = match &v.sound {
                    Sound::File(p) => p.rsplit('/').next().unwrap_or(p).trim_end_matches(".wav").to_string(),
                    Sound::Builtin(n) => format!("builtin {}", n),
                };
                format!("{}={}", v.rows.join("+"), s)
            })
            .collect();
        println!("{:<24} {:<22} {}", x.id, plan.kit.as_deref().unwrap_or("-"), sounds.join(" | "));
    }
}
