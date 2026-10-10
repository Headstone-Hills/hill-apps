//! Turns a flat list of WAV paths into browsable instruments: single files,
//! or multisample groups (several pitches of one instrument in one folder).

use crate::host::Host;
use crate::instrument::Instrument;
use crate::synth;
use crate::wav;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Builtin(usize),
    /// (path, detected root note)
    Files(Vec<(String, Option<u8>)>),
    /// A drum kit: one WAV per drum voice, already in track order (kick, snare, hat, rest).
    Kit(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub dir: String,
    pub label: String,
    /// Stable identity used for saved state and favorites.
    pub key: String,
    pub kind: Kind,
    pub pitched: bool,
}

/// Words that mark a sound or folder as a drum. Short words must start a name
/// part (`toms`, `hats_01`), so "tom" does not match "custom".
pub const DRUM_WORDS: &[&str] = &[
    "kick", "snare", "hat", "hihat", "perc", "cymbal", "clap", "tom", "drum", "kit", "808", "909", "rim", "shaker", "conga", "bongo",
    "cowbell", "tamb", "crash", "ride", "snap", "beat",
];

fn is_drum_word(part: &str, w: &str) -> bool {
    part.starts_with(w) || (w.len() >= 4 && part.contains(w))
}

fn is_wav(name: &str) -> bool {
    let base = name.rsplit('/').next().unwrap_or(name);
    !base.starts_with("._") && base.to_lowercase().ends_with(".wav")
}

fn pc_of(c: char) -> Option<i32> {
    Some(match c.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    })
}

/// `C4`, `Eb3`, `F#2`, `A-1` -> midi.
fn note_token(tok: &str) -> Option<u8> {
    let mut ch = tok.chars();
    let mut pc = pc_of(ch.next()?)?;
    let rest: String = ch.collect();
    let mut r = rest.as_str();
    if let Some(x) = r.strip_prefix('#') {
        pc += 1;
        r = x;
    } else if let Some(x) = r.strip_prefix('b') {
        pc -= 1;
        r = x;
    }
    if r.is_empty() || !r.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return None;
    }
    let oct: i32 = r.parse().ok()?;
    let m = (oct + 1) * 12 + pc;
    (0..=127).contains(&m).then_some(m as u8)
}

fn tokens(stem: &str) -> Vec<&str> {
    stem.split(['_', ' ']).filter(|t| !t.is_empty()).collect()
}

/// Root note encoded in a file stem (`midi036`, `C2`, `Piano_Eb4`).
pub fn parse_pitch(stem: &str) -> Option<u8> {
    for t in tokens(stem) {
        let l = t.to_lowercase();
        if let Some(n) = l.strip_prefix("midi") {
            if let Ok(v) = n.parse::<u32>() {
                if v <= 127 {
                    return Some(v as u8);
                }
            }
        }
    }
    tokens(stem).into_iter().find_map(note_token)
}

/// The stem with pitch/midi tokens removed: files of one instrument share it.
pub fn instr_stem(stem: &str) -> String {
    let kept: Vec<&str> = tokens(stem)
        .into_iter()
        .filter(|t| {
            let midi = t.get(..4).is_some_and(|h| h.eq_ignore_ascii_case("midi"))
                && t.get(4..).is_some_and(|n| n.parse::<u32>().is_ok());
            !midi && note_token(t).is_none()
        })
        .collect();
    if kept.is_empty() {
        stem.to_string()
    } else {
        kept.join("_")
    }
}

fn split_path(p: &str) -> (&str, &str) {
    match p.rfind('/') {
        Some(i) => (&p[..i], &p[i + 1..]),
        None => ("", p),
    }
}

fn strip_ext(f: &str) -> &str {
    f.rfind('.').map(|i| &f[..i]).unwrap_or(f)
}

/// Built-ins first, then folders of WAVs: multisample groups collapse into a
/// single pitched entry.
pub fn build_catalog(paths: &[String]) -> Vec<Entry> {
    let mut out: Vec<Entry> = synth::BUILTINS
        .iter()
        .enumerate()
        .map(|(i, (n, pitched))| Entry {
            dir: "Built-in".into(),
            label: n.to_string(),
            key: format!("builtin:{}", n),
            kind: Kind::Builtin(i),
            pitched: *pitched,
        })
        .collect();

    let mut files: Vec<&String> = paths.iter().filter(|p| is_wav(p)).collect();
    files.sort();

    // group by (dir, stem-without-pitch)
    type Group = (String, String, Vec<(String, Option<u8>)>);
    let mut groups: Vec<Group> = Vec::new();
    for p in files {
        let (dir, file) = split_path(p);
        let stem = strip_ext(file);
        let g = instr_stem(stem);
        let pitch = parse_pitch(stem);
        match groups.iter_mut().find(|(d, s, _)| d == dir && *s == g) {
            Some((_, _, v)) => v.push((p.clone(), pitch)),
            None => groups.push((dir.to_string(), g, vec![(p.clone(), pitch)])),
        }
    }

    for (dir, stem, members) in groups {
        let mut pitches: Vec<u8> = members.iter().filter_map(|m| m.1).collect();
        pitches.sort();
        pitches.dedup();
        if pitches.len() >= 2 && members.len() >= 2 {
            out.push(Entry {
                dir: dir.clone(),
                label: format!("{} ({} notes)", stem, pitches.len()),
                key: format!("{}/{}*", dir, stem),
                kind: Kind::Files(members),
                pitched: true,
            });
        } else {
            for m in members {
                let (_, file) = split_path(&m.0);
                out.push(Entry {
                    dir: dir.clone(),
                    label: strip_ext(file).to_string(),
                    key: m.0.clone(),
                    pitched: m.1.is_some(),
                    kind: Kind::Files(vec![m]),
                });
            }
        }
    }
    out
}

/// One entry per WAV file (for the loop picker).
pub fn file_catalog(paths: &[String]) -> Vec<Entry> {
    let mut v: Vec<&String> = paths.iter().filter(|p| is_wav(p)).collect();
    v.sort();
    v.into_iter()
        .map(|p| {
            let (dir, file) = split_path(p);
            Entry {
                dir: dir.to_string(),
                label: strip_ext(file).to_string(),
                key: p.clone(),
                kind: Kind::Files(vec![(p.clone(), None)]),
                pitched: false,
            }
        })
        .collect()
}

pub fn find_key<'a>(cat: &'a [Entry], key: &str) -> Option<&'a Entry> {
    cat.iter().find(|e| e.key == key)
}

pub fn matches_filter(e: &Entry, words: &[&str]) -> bool {
    let hay = format!("{} {}", e.dir, e.label).to_lowercase();
    hay.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()).any(|p| words.iter().any(|w| is_drum_word(p, w)))
}

fn drum_rank(stem: &str) -> u8 {
    let s = stem.to_lowercase();
    let has = |ws: &[&str]| s.split(|c: char| !c.is_alphanumeric()).any(|p| ws.iter().any(|w| is_drum_word(p, w)));
    if has(&["kick"]) {
        0
    } else if has(&["snare", "clap", "rim"]) {
        1
    } else if has(&["hat", "hihat", "cymbal"]) {
        2
    } else {
        3
    }
}

/// Folders holding at least two drum-named WAVs become kits. Files are ordered
/// kick, snare/clap, hats, then the rest alphabetically, so they land on the
/// drum tracks in a sensible order.
pub fn kit_entries(paths: &[String]) -> Vec<Entry> {
    let mut files: Vec<&String> = paths.iter().filter(|p| is_wav(p)).collect();
    files.sort();
    let mut dirs: Vec<(String, Vec<String>)> = Vec::new();
    for p in files {
        let (dir, file) = split_path(p);
        let probe = Entry { dir: String::new(), label: strip_ext(file).to_string(), key: String::new(), kind: Kind::Kit(vec![]), pitched: false };
        if !matches_filter(&probe, DRUM_WORDS) {
            continue;
        }
        match dirs.iter_mut().find(|(d, _)| d == dir) {
            Some((_, v)) => v.push(p.clone()),
            None => dirs.push((dir.to_string(), vec![p.clone()])),
        }
    }
    dirs.into_iter()
        .filter(|(_, v)| v.len() >= 2)
        .map(|(dir, mut v)| {
            v.sort_by_key(|p| (drum_rank(strip_ext(split_path(p).1)), p.clone()));
            let (parent, name) = split_path(&dir);
            Entry {
                dir: parent.to_string(),
                label: format!("{} [KIT x{}]", if name.is_empty() { "kit" } else { name }, v.len()),
                key: format!("kit:{}", dir),
                kind: Kind::Kit(v),
                pitched: false,
            }
        })
        .collect()
}

/// Load an entry's audio. Unpitched single files get root C4 (midi 60).
pub fn load_entry(host: &dyn Host, e: &Entry) -> Option<Instrument> {
    match &e.kind {
        Kind::Builtin(i) => synth::builtin(*i),
        Kind::Kit(_) => None, // applied per track by the app, one file each
        Kind::Files(files) => {
            let mut samples = Vec::new();
            for (path, pitch) in files {
                if let Some(s) = host.read_sample(path).as_deref().and_then(wav::parse) {
                    samples.push((pitch.unwrap_or(60), s));
                }
            }
            if samples.is_empty() {
                return None;
            }
            samples.sort_by_key(|(r, _)| *r);
            Some(Instrument { name: e.label.clone(), samples })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pitch_parsing() {
        assert_eq!(parse_pitch("HoveFlute_midi036_C2"), Some(36));
        assert_eq!(parse_pitch("Piano_Eb4"), Some(63));
        assert_eq!(parse_pitch("Piano_F#3"), Some(54));
        assert_eq!(parse_pitch("Piano_C-1"), Some(0));
        assert_eq!(parse_pitch("kick_hard"), None);
        assert_eq!(parse_pitch("Cassette"), None);
    }

    #[test]
    fn stems() {
        assert_eq!(instr_stem("HoveFlute_midi036_C2"), "HoveFlute");
        assert_eq!(instr_stem("Piano_Eb4"), "Piano");
        assert_eq!(instr_stem("kick_hard"), "kick_hard");
    }

    #[test]
    fn multisample_groups_collapse() {
        let cat = build_catalog(&p(&[
            "Keys/Piano_C3.wav",
            "Keys/Piano_C4.wav",
            "Keys/Piano_C5.wav",
            "Drums/kick_a.wav",
            "Drums/kick_b.wav",
            "Drums/._kick_a.wav",
        ]));
        let user: Vec<_> = cat.iter().filter(|e| !matches!(e.kind, Kind::Builtin(_))).collect();
        assert_eq!(user.len(), 3);
        let piano = user.iter().find(|e| e.label.starts_with("Piano")).unwrap();
        assert!(piano.pitched);
        assert!(matches!(&piano.kind, Kind::Files(f) if f.len() == 3));
        assert!(user.iter().filter(|e| e.dir == "Drums").count() == 2);
    }

    #[test]
    fn builtins_present_and_first() {
        let cat = build_catalog(&[]);
        assert_eq!(cat.len(), synth::BUILTINS.len());
        assert!(matches!(cat[0].kind, Kind::Builtin(0)));
    }

    #[test]
    fn tom_does_not_match_custom() {
        let cat = build_catalog(&p(&["Keys/custom_pad.wav", "Keys/atom.wav", "Drums/toms_low.wav"]));
        let hits: Vec<_> = cat.iter().filter(|e| matches_filter(e, DRUM_WORDS)).map(|e| e.label.clone()).collect();
        assert!(hits.contains(&"toms_low".to_string()));
        assert!(!hits.contains(&"custom_pad".to_string()));
        assert!(!hits.contains(&"atom".to_string()));
    }

    #[test]
    fn kits_are_folders_with_drum_files_in_voice_order() {
        let kits = kit_entries(&p(&[
            "Packs/Dusty/hat_closed.wav",
            "Packs/Dusty/snare_01.wav",
            "Packs/Dusty/kick_01.wav",
            "Packs/Dusty/perc_x.wav",
            "Packs/Dusty/pad_warm.wav",
            "Keys/kick_only.wav",
        ]));
        assert_eq!(kits.len(), 1);
        assert_eq!(kits[0].key, "kit:Packs/Dusty");
        let Kind::Kit(f) = &kits[0].kind else { panic!() };
        let names: Vec<_> = f.iter().map(|x| x.rsplit('/').next().unwrap()).collect();
        assert_eq!(names, ["kick_01.wav", "snare_01.wav", "hat_closed.wav", "perc_x.wav"]);
    }

    #[test]
    fn drum_filter() {
        let cat = build_catalog(&p(&["Kit/snare_1.wav", "Pads/warm.wav"]));
        let hits: Vec<_> = cat.iter().filter(|e| matches_filter(e, &DRUM_WORDS)).map(|e| e.label.clone()).collect();
        assert!(hits.contains(&"snare_1".to_string()));
        assert!(hits.contains(&"Kick".to_string()));
        assert!(!hits.contains(&"warm".to_string()));
    }
}
