//! HillBeat's preset drum patterns (`data/presets.json`, 22 styles, 267
//! patterns), and how they go onto HillBeat's tracks with sounds from a drum kit.
//!
//! The JSON is compiled in, so it works the same on device, desktop and in the
//! browser build, whatever directory the app is launched from. Check the data
//! after any change with `python3 data/validate_presets.py data/presets.json`.

use std::sync::OnceLock;

pub const PRESETS_JSON: &str = include_str!("../data/presets.json");

/// Every row the data may use, top to bottom as the book lists them. The preview
/// and the loaded tracks follow this order.
pub const ROWS: [&str; 13] = ["AC", "CY", "CH", "OH", "HT", "MT", "SD", "RS", "LT", "CPS", "CB", "TAM", "BD"];

/// When a pattern sounds more rows than HillBeat has tracks, rows join a related
/// row, in this order, until it fits: (row, rows it may join; the first present wins).
/// Every shipped pattern fits after the tom and hat steps. Toms keep their pitch
/// when they share a track (see `TOM_SEMIS`).
pub const MERGES: &[(&str, &[&str])] = &[
    ("CY", &["OH", "CH"]),
    ("HT", &["MT", "LT"]),
    ("LT", &["MT", "HT"]),
    ("TAM", &["CH", "OH"]),
    ("OH", &["CH"]),
    ("RS", &["SD"]),
    ("CPS", &["SD"]),
    ("CB", &["CH", "RS", "SD"]),
];

/// Tom pitches relative to the mid tom, in semitones: a shared or generic tom
/// sample is pitched to these so tom fills still go high to low.
pub const TOM_SEMIS: [(&str, i8); 3] = [("HT", 5), ("MT", 0), ("LT", -5)];

/// Kits each style prefers (matched against the kit's folder, case-insensitive),
/// best first. A kit that has the pattern's sounds wins over preference; these
/// break ties. Names are kits in the SP's `Samples/Drums`; missing ones are skipped.
pub const STYLE_KITS: &[(&str, &[&str])] = &[
    ("Afro-Cuban", &["lm vinyl", "ampeg 808", "cleancut"]),
    ("Bossa Nova", &["lm vinyl", "ampeg 808", "boogie classic"]),
    ("Cha Cha", &["lm vinyl", "ampeg 808", "cleancut"]),
    ("Samba", &["lm vinyl", "ampeg 808", "cleancut"]),
    ("Blues", &["boogie classic", "83 on myrtle", "lm vinyl"]),
    ("Boogie", &["boogie classic", "83 on myrtle", "lm vinyl"]),
    ("Jazz", &["future retro", "boogie classic", "lm vinyl"]),
    ("Swing", &["future retro", "boogie classic", "lm vinyl"]),
    ("Shuffle", &["boogie classic", "lm vinyl", "future retro"]),
    ("Slow", &["lm vinyl", "boogie classic", "future retro"]),
    ("Waltz", &["future retro", "lm vinyl", "boogie classic"]),
    ("March & Tango", &["lm vinyl", "future retro", "boogie classic"]),
    ("Paso Doble & Charleston", &["future retro", "lm vinyl", "boogie classic"]),
    ("Twist", &["boogie classic", "lm vinyl", "future retro"]),
    ("Disco", &["909 tube", "cleancut", "ampeg 808"]),
    ("Funk", &["boogie classic", "lm vinyl", "ampeg 808"]),
    ("R&B", &["boogie classic", "ampeg 808", "lm vinyl"]),
    ("Pop", &["lm vinyl", "909 tube", "mcmxciii"]),
    ("Rock", &["lm vinyl", "909 tube", "mcmxciii"]),
    ("Reggae", &["ampeg 808", "cleancut", "heavy sat"]),
    ("Ska", &["ampeg 808", "boogie classic", "cleancut"]),
    ("Ending", &["909 tube", "lm vinyl", "ampeg 808"]),
];

/// Velocity of a normal hit and of an accented one (cells hold 1..=127).
pub const NORMAL_VEL: u8 = 96;
pub const ACCENT_VEL: u8 = 127;
/// Set in a cell next to its velocity: play this step as a flam.
pub const FLAM_BIT: u8 = 0x80;
/// A flam's grace note: how early it sounds, and its level relative to the main hit.
pub const FLAM_GRACE_MS: f32 = 30.0;
pub const FLAM_GRACE_LEVEL: f32 = 0.4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Rhythm,
    Break,
    Ending,
}

impl Kind {
    pub fn header(self) -> &'static str {
        match self {
            Kind::Rhythm => "RHYTHMS",
            Kind::Break => "BREAKS",
            Kind::Ending => "ENDINGS",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub style: String,
    pub kind: Kind,
    pub time_sig: String,
    pub steps: usize,
    /// Steps per beat: 4 for sixteenths, 3 for triplets and 12/8 eighths.
    pub div: u32,
    pub book_page: u32,
    /// (row key, pattern string), in file order. Unknown rows are dropped at load.
    pub tracks: Vec<(String, String)>,
}

impl Preset {
    /// Small tag for a non-default feel: `12/8`, `3/4` or `triplet`.
    pub fn tag(&self) -> Option<&'static str> {
        match (self.time_sig.as_str(), self.div) {
            ("12/8", _) => Some("12/8"),
            ("3/4", _) => Some("3/4"),
            (_, 3) => Some("triplet"),
            _ => None,
        }
    }

    /// How this pattern goes onto `voices` drum tracks with sounds from `kits`
    /// (or the built-in sounds when no kit fits).
    pub fn plan(&self, kits: &[Kit], voices: usize) -> Plan {
        // sounding rows, top to bottom; fold related rows until they fit
        let mut groups: Vec<Vec<&'static str>> = ROWS.iter().filter(|r| **r != "AC" && self.row(r).is_some()).map(|r| vec![*r]).collect();
        for (row, into) in MERGES {
            if groups.len() <= voices {
                break;
            }
            let Some(gi) = groups.iter().position(|g| g.contains(row)) else { continue };
            if let Some(ti) = into.iter().find_map(|t| groups.iter().position(|g| g.contains(t)).filter(|&ti| ti != gi)) {
                let g = groups.remove(gi);
                let ti = if ti > gi { ti - 1 } else { ti };
                groups[ti].extend(g);
                groups[ti].sort_by_key(|r| row_pos(r));
            }
        }
        // a merged track sits where its top row is
        groups.sort_by_key(|g| row_pos(g[0]));
        while groups.len() > voices {
            // not reached with the shipped data: fold the last row into the one above
            let g = groups.pop().unwrap_or_default();
            if let Some(last) = groups.last_mut() {
                last.extend(g);
            }
        }
        // each track plays the sound of its busiest row
        let hits = |r: &str| self.row(r).map_or(0, |s| s.chars().filter(|c| *c != '.').count());
        let leads: Vec<&'static str> = groups.iter().map(|g| *g.iter().max_by_key(|r| (hits(r), std::cmp::Reverse(row_pos(r)))).unwrap()).collect();
        let pref: &[&str] = STYLE_KITS.iter().find(|(s, _)| *s == self.style).map_or(&[], |(_, k)| *k);
        let rank = |k: &Kit| {
            let cover: u32 = leads.iter().map(|r| k.pick(r).map_or(0, |(_, q, _)| q as u32)).sum();
            let n = k.name.to_lowercase();
            let liked = pref.iter().position(|p| n.contains(p)).map_or(0, |i| pref.len() - i);
            (cover, liked)
        };
        let kit = kits.iter().filter(|k| rank(k).0 > 0).max_by_key(|k| rank(k));

        let mut out = Plan {
            kit: kit.map(|k| k.name.clone()),
            voices: Vec::new(),
            cells: vec![vec![0u8; self.steps]; voices],
            pitch: vec![vec![0i8; self.steps]; voices],
        };
        let accent: Vec<bool> = match self.row("AC") {
            Some(s) => s.chars().map(|c| c != '.').collect(),
            None => vec![false; self.steps],
        };
        for (ti, (g, lead)) in groups.iter().zip(&leads).enumerate() {
            let (sound, basis) = match kit.and_then(|k| k.pick(lead).map(|(f, _, b)| (Sound::File(k.files[f].path.clone()), b))) {
                Some(x) => x,
                None => builtin_for(lead),
            };
            for r in g {
                let semis = match (tom_semis(r), basis.and_then(tom_semis)) {
                    (Some(a), Some(b)) => a - b,
                    _ => 0,
                };
                for (i, c) in self.row(r).unwrap_or("").chars().enumerate().take(self.steps) {
                    if c == '.' {
                        continue;
                    }
                    let vel = if accent[i] { ACCENT_VEL } else { NORMAL_VEL };
                    let flam = if c == 'F' { FLAM_BIT } else { 0 };
                    let cell = &mut out.cells[ti][i];
                    if *cell == 0 {
                        out.pitch[ti][i] = semis; // two rows on one step: the upper row's pitch
                    }
                    *cell = (*cell & !FLAM_BIT).max(vel) | (*cell & FLAM_BIT) | flam;
                }
            }
            out.voices.push(PlanVoice { rows: g.clone(), sound });
        }
        // spare tracks get other sounds of the same kit, so the kit stays whole
        for role in ["BD", "SD", "CH", "OH", "CPS", "RS", "CB", "MT", "CY", "TAM"] {
            if out.voices.len() >= voices {
                break;
            }
            let taken = |s: &Sound| out.voices.iter().any(|v| v.sound == *s);
            let sound = match kit.and_then(|k| k.pick(role)).filter(|(_, q, _)| *q == 2) {
                Some((f, _, _)) => Sound::File(kit.unwrap().files[f].path.clone()),
                None if kit.is_none() => builtin_for(role).0,
                None => continue,
            };
            if !taken(&sound) {
                out.voices.push(PlanVoice { rows: vec![], sound });
            }
        }
        while out.voices.len() < voices {
            out.voices.push(PlanVoice { rows: vec![], sound: Sound::Builtin("Clap") });
        }
        out
    }

    fn row(&self, r: &str) -> Option<&str> {
        self.tracks.iter().find(|(k, _)| k == r).map(|(_, s)| s.as_str())
    }
}

fn row_pos(r: &str) -> usize {
    ROWS.iter().position(|x| *x == r).unwrap_or(ROWS.len())
}

fn tom_semis(r: &str) -> Option<i8> {
    TOM_SEMIS.iter().find(|(k, _)| *k == r).map(|(_, s)| *s)
}

/// The built-in sound for a row, and the row it stands for (for tom pitch).
fn builtin_for(row: &str) -> (Sound, Option<&'static str>) {
    match row {
        "BD" => (Sound::Builtin("Kick"), None),
        "SD" | "RS" => (Sound::Builtin("Snare"), None),
        "CPS" => (Sound::Builtin("Clap"), None),
        "HT" | "MT" | "LT" => (Sound::Builtin("Tom"), Some("MT")),
        "CB" => (Sound::Builtin("Tom"), None),
        _ => (Sound::Builtin("Hat"), None),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sound {
    /// A sample file (path as the host lists it).
    File(String),
    /// One of the built-in sounds, by name.
    Builtin(&'static str),
}

#[derive(Clone, Debug)]
pub struct PlanVoice {
    /// Data rows on this track, top to bottom (empty for a spare track).
    pub rows: Vec<&'static str>,
    pub sound: Sound,
}

/// A preset laid out for HillBeat: one entry per track, in the preview's order.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The kit the sounds come from (`None`: built-in sounds).
    pub kit: Option<String>,
    pub voices: Vec<PlanVoice>,
    /// Per track: velocities with `FLAM_BIT`, and semitones (toms sharing a sound).
    pub cells: Vec<Vec<u8>>,
    pub pitch: Vec<Vec<i8>>,
}

/// A drum kit folder, with each file's name split into lowercase words.
pub struct Kit {
    pub name: String,
    pub files: Vec<KitFile>,
}

pub struct KitFile {
    pub path: String,
    words: Vec<String>,
}

impl Kit {
    pub fn new(name: &str, paths: &[String]) -> Kit {
        let mut files: Vec<KitFile> = paths
            .iter()
            .map(|p| {
                let stem = p.rsplit('/').next().unwrap_or(p);
                let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
                KitFile { path: p.clone(), words: stem.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect() }
            })
            .collect();
        // plain sounds before effected ones ("Tom FX Tremolo"), then by name
        files.sort_by_key(|f| (f.words.iter().any(|w| w == "fx"), f.path.clone()));
        Kit { name: name.to_string(), files }
    }

    /// The file for a row: (index, 2 = that sound / 1 = a stand-in, the row the
    /// file really is, for tom pitch).
    pub fn pick(&self, row: &str) -> Option<(usize, u8, Option<&'static str>)> {
        let has = |f: &KitFile, ws: &[&str]| f.words.iter().any(|w| ws.contains(&w.as_str()));
        let tom = |f: &KitFile| has(f, &["tom", "toms"]);
        // a tom named without hi/mid/lo is a stand-in for any of them, pitched as a mid tom
        let tom_role = |f: &KitFile| -> Option<&'static str> {
            if has(f, &["hi", "high", "ht"]) {
                Some("HT")
            } else if has(f, &["lo", "low", "floor", "lt"]) {
                Some("LT")
            } else if has(f, &["mid", "mt"]) {
                Some("MT")
            } else {
                None
            }
        };
        let find = |q: u8, ok: &dyn Fn(&KitFile) -> bool| self.files.iter().position(ok).map(|i| (i, q));
        let hat = ["hat", "hihat", "hh"];
        let r = match row {
            "BD" => find(2, &|f| has(f, &["bd", "kick", "bassdrum"])),
            "SD" => find(2, &|f| has(f, &["sd", "snare"])).or_else(|| find(1, &|f| has(f, &["rim", "clap"]))),
            "CH" => find(2, &|f| has(f, &["ch", "chh", "closed"])).or_else(|| find(1, &|f| has(f, &hat) && !has(f, &["open"]))),
            "OH" => find(2, &|f| has(f, &["oh", "ohh", "open"])).or_else(|| find(1, &|f| has(f, &hat) || has(f, &["ch"]))),
            "CY" => find(2, &|f| has(f, &["cym", "cymbal", "crash", "ride"])).or_else(|| find(1, &|f| has(f, &["oh", "open"]))),
            "RS" => find(2, &|f| has(f, &["rim", "rimshot", "rs", "sidestick", "stick"])).or_else(|| find(1, &|f| has(f, &["clave", "sd", "snare"]))),
            "CPS" => find(2, &|f| has(f, &["clap", "claps", "cp"])).or_else(|| find(1, &|f| has(f, &["snap", "sd", "snare"]))),
            "CB" => find(2, &|f| has(f, &["cowbell", "cb"])).or_else(|| find(1, &|f| has(f, &["clave", "agogo", "rim"]))),
            "TAM" => find(2, &|f| has(f, &["tamb", "tambourine"])).or_else(|| find(1, &|f| has(f, &["shaker", "maracas", "cabasa"]) || has(f, &["ch"]))),
            "HT" | "MT" | "LT" => {
                let exact = find(2, &|f| tom(f) && tom_role(f) == Some(row));
                let any = || find(1, &|f| tom(f)).or_else(|| find(1, &|f| has(f, &["conga", "bongo"])));
                return exact.or_else(any).map(|(i, q)| {
                    let f = &self.files[i];
                    (i, q, Some(if tom(f) { tom_role(f).unwrap_or("MT") } else { "MT" }))
                });
            }
            _ => None,
        };
        r.map(|(i, q)| (i, q, None))
    }
}

pub struct Presets {
    pub styles: Vec<String>,
    pub patterns: Vec<Preset>,
    /// Problems skipped while loading (unknown rows, bad lengths). Empty for the shipped data.
    pub warnings: Vec<String>,
}

impl Presets {
    /// Indexes into `patterns` for one style: rhythms, then breaks, then endings.
    pub fn of_style(&self, style: usize) -> Vec<usize> {
        let name = &self.styles[style];
        let mut v: Vec<usize> = (0..self.patterns.len()).filter(|&i| self.patterns[i].style == *name).collect();
        v.sort_by_key(|&i| (self.patterns[i].kind as u8, i));
        v
    }
}

/// The built-in presets, parsed once.
pub fn builtin() -> &'static Presets {
    static P: OnceLock<Presets> = OnceLock::new();
    P.get_or_init(|| load_presets(PRESETS_JSON).unwrap_or_else(|e| Presets { styles: vec![], patterns: vec![], warnings: vec![e] }))
}

/// Parse presets.json. Light validation: a track whose length is not `steps`,
/// or an unknown row key, is skipped with a warning; it never fails the load.
pub fn load_presets(text: &str) -> Result<Presets, String> {
    let root = Json::parse(text)?;
    let styles: Vec<String> = root.get("styles").and_then(Json::arr).ok_or("no styles")?.iter().filter_map(|s| s.str().map(String::from)).collect();
    let mut warnings = Vec::new();
    let mut patterns = Vec::new();
    for p in root.get("patterns").and_then(Json::arr).ok_or("no patterns")? {
        let s = |k: &str| p.get(k).and_then(Json::str).unwrap_or("").to_string();
        let id = s("id");
        let steps = p.get("steps").and_then(Json::num).unwrap_or(0.0) as usize;
        if steps == 0 || steps > crate::grid::STEPS {
            warnings.push(format!("{}: {} steps, skipped", id, steps));
            continue;
        }
        let kind = match s("kind").as_str() {
            "break" => Kind::Break,
            "ending" => Kind::Ending,
            _ => Kind::Rhythm,
        };
        let div = if s("step_feel") == "sixteenths" { 4 } else { 3 };
        let mut tracks = Vec::new();
        if let Some(Json::Obj(t)) = p.get("tracks") {
            for (k, v) in t {
                let pat = v.str().unwrap_or("");
                if !ROWS.contains(&k.as_str()) {
                    warnings.push(format!("{}: unknown row {}, skipped", id, k));
                } else if pat.chars().count() != steps {
                    warnings.push(format!("{}: row {} is not {} steps, skipped", id, k, steps));
                } else {
                    tracks.push((k.clone(), pat.to_string()));
                }
            }
        }
        patterns.push(Preset {
            name: s("name"),
            style: s("style"),
            time_sig: s("time_sig"),
            book_page: p.get("book_page").and_then(Json::num).unwrap_or(0.0) as u32,
            id,
            kind,
            steps,
            div,
            tracks,
        });
    }
    Ok(Presets { styles, patterns, warnings })
}

// ---- a minimal JSON reader (the crate has no dependencies beyond the font) ----

#[derive(Debug)]
enum Json {
    Null,
    Bool, // the data has none; parsed so any valid JSON loads
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// Kept in file order: track rows are listed top to bottom like the book.
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn parse(s: &str) -> Result<Json, String> {
        let mut p = Parser { b: s.as_bytes(), i: 0 };
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() {
            return Err(format!("trailing data at byte {}", p.i));
        }
        Ok(v)
    }
    fn get(&self, k: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.iter().find(|(kk, _)| kk == k).map(|(_, v)| v),
            _ => None,
        }
    }
    fn arr(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
    fn str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    fn num(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{} at byte {}", what, self.i))
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            self.err(&format!("expected '{}'", c as char))
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.b.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut m = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    m.push((k, self.value()?));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(m));
                        }
                        _ => return self.err("expected ',' or '}'"),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(a));
                }
                loop {
                    a.push(self.value()?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(a));
                        }
                        _ => return self.err("expected ',' or ']'"),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Ok(Json::Bool)
            }
            Some(b'f') if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Ok(Json::Bool)
            }
            Some(b'n') if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                Ok(Json::Null)
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                let st = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.b[st..self.i]).unwrap_or("");
                t.parse().map(Json::Num).or_else(|_| self.err("bad number"))
            }
            _ => self.err("unexpected character"),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        if self.b.get(self.i) != Some(&b'"') {
            return self.err("expected string");
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            match self.b.get(self.i) {
                None => return self.err("unterminated string"),
                Some(b'"') => {
                    self.i += 1;
                    return String::from_utf8(out).or_else(|_| self.err("bad utf-8"));
                }
                Some(b'\\') => {
                    let e = self.b.get(self.i + 1).copied();
                    self.i += 2;
                    match e {
                        Some(b'n') => out.push(b'\n'),
                        Some(b't') => out.push(b'\t'),
                        Some(b'r') => out.push(b'\r'),
                        Some(b'b') => out.push(8),
                        Some(b'f') => out.push(12),
                        Some(b'u') => {
                            let h = self.b.get(self.i..self.i + 4).and_then(|h| std::str::from_utf8(h).ok());
                            let c = h.and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32).unwrap_or('?');
                            self.i += 4;
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        }
                        Some(c) => out.push(c),
                        None => return self.err("bad escape"),
                    }
                }
                Some(&c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_by_style(p: &Presets) -> std::collections::BTreeMap<String, (usize, usize, usize)> {
        let mut m: std::collections::BTreeMap<String, (usize, usize, usize)> = std::collections::BTreeMap::new();
        for x in &p.patterns {
            let e = m.entry(x.style.clone()).or_default();
            match x.kind {
                Kind::Rhythm => e.0 += 1,
                Kind::Break => e.1 += 1,
                Kind::Ending => e.2 += 1,
            }
        }
        m
    }

    #[test]
    fn shipped_data_loads_cleanly() {
        let p = builtin();
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        assert_eq!(p.styles.len(), 22);
        assert_eq!(p.patterns.len(), 267);
        assert_eq!(p.patterns.iter().filter(|x| x.steps == 12).count(), 51);
        let flams: usize = p.patterns.iter().flat_map(|x| x.tracks.iter()).map(|(_, s)| s.matches('F').count()).sum();
        assert_eq!(flams, 100);
        let c = count_by_style(p);
        assert_eq!(c["Funk"], (15, 15, 0));
        assert_eq!(c["Ending"], (0, 0, 3));
        assert_eq!(p.patterns.iter().filter(|x| x.div == 3).count(), 45);
    }

    #[test]
    fn unknown_rows_and_bad_lengths_are_skipped_not_fatal() {
        let j = r#"{"styles":["S"],"patterns":[{"id":"s-r1","name":"S 1","style":"S","kind":"rhythm","time_sig":"4/4","steps":4,"step_feel":"sixteenths","tracks":{"BD":"x...","ZZ":"x...","SD":"x.."}}]}"#;
        let p = load_presets(j).unwrap();
        assert_eq!(p.patterns[0].tracks.len(), 1);
        assert_eq!(p.warnings.len(), 2);
    }

    #[test]
    fn accent_raises_the_other_hits_and_flams_are_marked() {
        let j = r#"{"styles":["S"],"patterns":[{"id":"s-r1","name":"S 1","style":"S","kind":"rhythm","time_sig":"4/4","steps":4,"step_feel":"sixteenths","tracks":{"AC":"x...","SD":"xF..","BD":"x.x."}}]}"#;
        let p = load_presets(j).unwrap();
        let plan = p.patterns[0].plan(&[], 4);
        // top to bottom as the book lists them: SD above BD
        assert_eq!(plan.voices[0].rows, vec!["SD"]);
        assert_eq!(plan.voices[1].rows, vec!["BD"]);
        let c = &plan.cells;
        assert_eq!(c[0], vec![ACCENT_VEL, NORMAL_VEL | FLAM_BIT, 0, 0]);
        assert_eq!(c[1], vec![ACCENT_VEL, 0, NORMAL_VEL, 0]);
        assert!(c[2].iter().chain(c[3].iter()).all(|&v| v == 0), "AC is not a sound");
        assert!(plan.voices[2].rows.is_empty() && plan.kit.is_none());
    }

    fn kit(name: &str, files: &[&str]) -> Kit {
        Kit::new(name, &files.iter().map(|f| format!("Drums/{}/{}.wav", name, f)).collect::<Vec<_>>())
    }

    #[test]
    fn more_rows_than_tracks_fold_toms_and_keep_their_pitch() {
        // Funk Break 5: SD, HT, MT, LT (+ AC) -> fits in 4 without folding
        let ps = builtin();
        let fb5 = ps.patterns.iter().find(|x| x.id == "funk-b5").unwrap();
        assert!(fb5.plan(&[], 4).voices.iter().all(|v| v.rows.len() <= 1));
        // every shipped pattern fits, and its tracks keep the book's top-to-bottom order
        for x in &ps.patterns {
            let plan = x.plan(&[], 4);
            let firsts: Vec<usize> = plan.voices.iter().filter(|v| !v.rows.is_empty()).map(|v| row_pos(v.rows[0])).collect();
            assert!(firsts.windows(2).all(|w| w[0] < w[1]), "{} order {:?}", x.id, plan.voices);
            let sounding = x.tracks.iter().filter(|(k, _)| k != "AC").count();
            assert_eq!(plan.voices.iter().map(|v| v.rows.len()).sum::<usize>(), sounding, "{}", x.id);
        }
        // a 7-row pattern: the three toms share one tom sound, pitched high/mid/low
        let x = ps.patterns.iter().find(|x| x.tracks.iter().filter(|(k, _)| ["HT", "MT", "LT"].contains(&k.as_str())).count() == 3 && x.tracks.len() >= 7).unwrap();
        let k = kit("Studio", &["BD 1", "SD 1", "CH 1", "OH 1", "Tom Mid 1", "Crash 1"]);
        let plan = x.plan(&[k], 4);
        let ti = plan.voices.iter().position(|v| v.rows.contains(&"MT")).unwrap();
        let toms = &plan.voices[ti];
        assert!(toms.rows.len() >= 2, "{}: {:?}", x.id, plan.voices);
        assert_eq!(toms.sound, Sound::File("Drums/Studio/Tom Mid 1.wav".into()));
        for r in &toms.rows {
            let s = x.row(r).unwrap();
            if let Some(i) = s.find(|c| c != '.') {
                if plan.cells[ti][i] != 0 && toms.rows.iter().take_while(|o| *o != r).all(|o| x.row(o).unwrap().as_bytes()[i] == b'.') {
                    assert_eq!(plan.pitch[ti][i], tom_semis(r).unwrap(), "{} {} step {}", x.id, r, i + 1);
                }
            }
        }
    }

    #[test]
    fn the_style_picks_a_kit_that_has_the_sounds() {
        let ps = builtin();
        let lm = kit("04. LM Vinyl", &["BD LM", "SD LM", "CH Crunch", "OH LM", "Cowbell LM", "Conga Mid", "Rim LM", "Tom LM Hi", "Tom LM Mid", "Clap LM"]);
        let tube = kit("909 Tube Kit", &["BD 909", "SD 909", "CH 909", "OH 909", "Clap 909", "Rim 909", "Crash 909", "Tom 909 Hi", "Tom 909 Mid", "Tom 909 Lo"]);
        let boogie = kit("Boogie Classic", &["BD Real", "Snare Smack", "CH Analog", "OH Short", "Clap Boogie", "Cowbell Proto", "Rim Thin"]);
        let kits = [lm, tube, boogie];
        let disco = ps.patterns.iter().find(|x| x.id == "disco-r1").unwrap();
        assert_eq!(disco.plan(&kits, 4).kit.as_deref(), Some("909 Tube Kit"));
        let funk = ps.patterns.iter().find(|x| x.id == "funk-r1").unwrap();
        assert_eq!(funk.plan(&kits, 4).kit.as_deref(), Some("Boogie Classic"));
        // a funk break with toms: Boogie Classic has none, so a kit with toms wins
        let fb5 = ps.patterns.iter().find(|x| x.id == "funk-b5").unwrap();
        let plan = fb5.plan(&kits, 4);
        assert_ne!(plan.kit.as_deref(), Some("Boogie Classic"));
        assert!(plan.voices.iter().all(|v| matches!(v.sound, Sound::File(_))));
    }
}
