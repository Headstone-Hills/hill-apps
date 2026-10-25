//! HillBeat's preset drum patterns (`data/presets.json`, 22 styles, 267
//! patterns) and how they map onto HillBeat's four voices.
//!
//! The JSON is compiled in, so it works the same on device, desktop and in the
//! browser build, whatever directory the app is launched from. Check the data
//! after any change with `python3 data/validate_presets.py data/presets.json`.

use std::sync::OnceLock;

pub const PRESETS_JSON: &str = include_str!("../data/presets.json");

/// Data rows to HillBeat voices (track index: 0 Kick, 1 Snare, 2 Hat, 3 Perc,
/// the order the built-ins and drum kits use). `None` = not a sound: `AC` is an
/// accent on the other hits at that step. HillBeat has 4 voices and the data
/// has 12 sounds, so several rows share a voice; their hits are merged.
pub const ROW_TO_VOICE: &[(&str, Option<usize>)] = &[
    ("BD", Some(0)),
    ("SD", Some(1)),
    ("RS", Some(1)),  // fallback: rim shot on the snare
    ("CH", Some(2)),
    ("OH", Some(2)),  // fallback: open hat on the hat
    ("CY", Some(2)),  // fallback: cymbal on the hat
    ("TAM", Some(2)), // fallback: tambourine on the hat
    ("CPS", Some(3)), // claps: the default fourth voice is Clap
    ("CB", Some(3)),  // fallback: cowbell on perc
    ("HT", Some(3)),  // fallback: toms on perc
    ("MT", Some(3)),
    ("LT", Some(3)),
    ("AC", None),
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

    /// The pattern as HillBeat cells: `voices` rows of `steps` velocities, with
    /// `FLAM_BIT` set on flams. Rows sharing a voice merge (a flam wins).
    pub fn to_cells(&self, voices: usize) -> Vec<Vec<u8>> {
        let mut out = vec![vec![0u8; self.steps]; voices];
        let accent: Vec<bool> = match self.tracks.iter().find(|(k, _)| k == "AC") {
            Some((_, s)) => s.chars().map(|c| c != '.').collect(),
            None => vec![false; self.steps],
        };
        for (key, s) in &self.tracks {
            let Some(v) = voice_of(key) else { continue };
            if v >= voices {
                continue;
            }
            for (i, c) in s.chars().enumerate().take(self.steps) {
                if c == '.' {
                    continue;
                }
                let vel = if accent[i] { ACCENT_VEL } else { NORMAL_VEL };
                let flam = if c == 'F' { FLAM_BIT } else { 0 };
                let cell = &mut out[v][i];
                *cell = (*cell & !FLAM_BIT).max(vel) | (*cell & FLAM_BIT) | flam;
            }
        }
        out
    }
}

/// The voice a data row plays on (`None` for `AC` and unknown rows).
pub fn voice_of(row: &str) -> Option<usize> {
    ROW_TO_VOICE.iter().find(|(k, _)| *k == row).and_then(|(_, v)| *v)
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
                if !ROW_TO_VOICE.iter().any(|(r, _)| r == k) {
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
        let c = p.patterns[0].to_cells(4);
        assert_eq!(c[0], vec![ACCENT_VEL, 0, NORMAL_VEL, 0]);
        assert_eq!(c[1], vec![ACCENT_VEL, NORMAL_VEL | FLAM_BIT, 0, 0]);
        assert!(c[2].iter().chain(c[3].iter()).all(|&v| v == 0), "AC is not a sound");
    }
}
