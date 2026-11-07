//! Step-grid engine shared by HillBeat, HillSequencer and HillBand. The apps
//! differ only in their `GridConfig`: track layout and which extras are on.

use crate::fb::*;
use crate::host::Host;
use crate::input::{Btn, Held, Repeat, TapTempo};
use crate::instrument::Instrument;
use crate::kv::{from_hex, to_hex, Kv};
use crate::library::{self, Entry};
use crate::mixer::{EnvParams, Envelope, Mixer};
use crate::preset_menu::{MenuOut, PresetMenu};
use crate::presets::{self, Plan, Sound, FLAM_BIT, FLAM_GRACE_LEVEL, FLAM_GRACE_MS};
use crate::theory::*;
use crate::transport::Transport;
use crate::ui::{modal, quit_bar, Browser, BrowserOut, Help, Toast, QUIT_HOLD_MS};
use crate::{App, SAMPLE_RATE};

pub const STEPS: usize = 16;
pub const PATTERNS: usize = 8;

/// Opens and closes the preset pattern menu (HillBeat): hold the first, press the
/// second. The first must be START or SELECT. Free alternatives on the main screen
/// of every grid app: (Start, R1), (Start, A), (Select, A), (Select, Y).
pub const PRESET_MENU_COMBO: (Btn, Btn) = (Btn::Start, Btn::L1);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Drum,
    Pitched,
}

pub struct GridConfig {
    pub name: &'static str,
    pub file: &'static str,
    pub tracks: &'static [TrackKind],
    /// Per-track chord type + scale mode overlay (START+L2).
    pub chords: bool,
    /// Key/scale overlay that spreads a scale across all tracks (START+L2).
    pub key_overlay: bool,
    pub multiselect: bool,
    /// Pre-filter the browser to drum sounds when assigning a drum track.
    pub drum_filter: bool,
    /// Sounds longer than a beat decay smoothly after the first beat (see `Mixer::decay`).
    pub decay: bool,
    /// The preset drum pattern menu (PRESET_MENU_COMBO).
    pub presets: bool,
    /// Track volume goes up to `VOL_WARN_MAX` (+12 dB), through a soft-knee limiter.
    pub vol_boost: bool,
    pub help: &'static [(&'static str, &'static str)],
}

/// Track volume ceiling when `vol_boost` is on (4.0 = +12 dB), and the step per tick above 100%.
const VOL_WARN_MAX: f32 = 4.0;
const VOL_WARN_STEP: f32 = 0.25;

struct Track {
    kind: TrackKind,
    inst_key: String,
    inst: Option<Instrument>,
    note: u8,
    mute: bool,
    vol: f32,
    chord: usize,
    scale: usize,
    scale_pos: usize,
    /// Steps left before a sustained instrument's note is released (0 = not gated).
    gate_left: u32,
}


enum Overlay {
    None,
    Help,
    Browse(Vec<usize>),
    Chord { track: usize, chord: usize, scale: usize },
    Key { root: usize, scale: usize },
    /// The envelope settings screen; `row` is the highlighted line.
    Env { row: usize },
    Presets,
}

/// A preset waiting for the next bar line (loaded while playing).
struct Staged {
    pattern: usize,
    rows: Vec<Vec<u8>>,
    pitch: Vec<Vec<i8>>,
    len: usize,
    div: u32,
    /// Per track: the preset's sound (inst key, loaded), swapped in with the notes.
    sounds: Vec<Option<(String, Instrument)>>,
}

/// The preset menu's own little player: the main sequencer is stopped while it runs.
struct Audition {
    preset: usize,
    tr: Transport,
    plan: Plan,
    insts: Vec<Option<Instrument>>,
}

/// "Pressed alone and not yet used as a modifier" flags.
#[derive(Default)]
struct Plain {
    a: bool,
    x: bool,
    y: bool,
    start: bool,
    select: bool,
}

pub struct GridApp {
    cfg: &'static GridConfig,
    host: Box<dyn Host>,
    catalog: Vec<Entry>,
    rate: f32,
    mixer: Mixer,
    tr: Transport,
    tracks: Vec<Track>,
    /// Cell value: velocity 1..=127 (0 = off), plus `FLAM_BIT` for a flam.
    cells: Vec<u8>,
    /// Semitones per cell for drum tracks (a tom sound playing high/mid/low toms).
    pitch: Vec<i8>,
    /// Drum kits in the sample library, for the preset menu's sound choice.
    kits: Vec<presets::Kit>,
    /// Sounds loaded for presets, by inst key, so browsing does not re-read files.
    sound_cache: std::collections::HashMap<String, Instrument>,
    /// The highlighted preset laid out for this app's tracks and kits.
    pm_plan: Option<(usize, Plan)>,
    /// Per pattern: (steps, steps per beat). 16/4 unless a 12-step preset was loaded.
    lens: Vec<(usize, u32)>,
    staged: Option<Staged>,
    pm: PresetMenu,
    aud: Option<Audition>,
    pattern: usize,
    pending: Option<usize>,
    cur_step: usize,
    cur_track: usize,
    sel: Vec<bool>,
    held: Held,
    plain: Plain,
    ss: bool,
    ss_ms: u32,
    quit: bool,
    overlay: Overlay,
    browser: Browser,
    help: Help,
    toast: Toast,
    rep: Repeat,
    tap: TapTempo,
    ms: u64,
    /// Envelope for long sounds (HillSequencer / HillBand), one setting for the whole app.
    env: EnvParams,
    env_on: bool,
    /// (track, ms left): a sustained audition note to cut off.
    preview: Option<(usize, u32)>,
    last_vel: u8,
}

impl GridApp {
    pub fn new(cfg: &'static GridConfig, host: Box<dyn Host>) -> Self {
        let paths = host.list_samples();
        let mut catalog = library::build_catalog(&paths);
        if cfg.drum_filter {
            // drum kits go right after the built-ins, so they are the first thing in the drum browser
            let at = catalog.iter().take_while(|e| matches!(e.kind, library::Kind::Builtin(_))).count();
            let kits = library::kit_entries(&paths);
            catalog.splice(at..at, kits);
        }
        let nt = cfg.tracks.len();
        let mut app = GridApp {
            cfg,
            host,
            catalog,
            rate: SAMPLE_RATE,
            mixer: Mixer::new(SAMPLE_RATE),
            tr: Transport::new(SAMPLE_RATE, STEPS),
            tracks: Vec::new(),
            cells: vec![0; PATTERNS * nt * STEPS],
            pitch: vec![0; PATTERNS * nt * STEPS],
            kits: Vec::new(),
            sound_cache: Default::default(),
            pm_plan: None,
            lens: vec![(STEPS, 4); PATTERNS],
            staged: None,
            pm: PresetMenu::default(),
            aud: None,
            pattern: 0,
            pending: None,
            cur_step: 0,
            cur_track: 0,
            sel: vec![false; nt],
            held: Held::default(),
            plain: Plain::default(),
            ss: false,
            ss_ms: 0,
            quit: false,
            preview: None,
            env: EnvParams::default(),
            env_on: true,
            overlay: Overlay::None,
            browser: Browser::default(),
            help: Help::default(),
            toast: Toast::default(),
            rep: Repeat::default(),
            tap: TapTempo::default(),
            ms: 0,
            last_vel: 100,
        };
        app.mixer.soft_limit = cfg.vol_boost;
        app.defaults();
        if cfg.presets {
            app.kits = app
                .catalog
                .iter()
                .filter_map(|e| match &e.kind {
                    // named by its folder (the label carries a "[KIT x8]" tag)
                    library::Kind::Kit(files) => {
                        let dir = e.key.strip_prefix("kit:").unwrap_or(&e.key);
                        Some(presets::Kit::new(dir.rsplit('/').next().unwrap_or(dir), files))
                    }
                    _ => None,
                })
                .collect();
        }
        if let Some(text) = app.host.load_state(cfg.file) {
            app.apply_state(&text);
        }
        app
    }

    fn defaults(&mut self) {
        let kinds = self.cfg.tracks;
        let mut drum_i = 0;
        let mut mel_i = 0;
        let n_mel = kinds.iter().filter(|k| **k == TrackKind::Pitched).count();
        let key_scale = if self.cfg.key_overlay { Some(scale_notes(48, 1, kinds.len())) } else { None };
        self.tracks.clear();
        for (i, &kind) in kinds.iter().enumerate() {
            let (name, note) = match kind {
                TrackKind::Drum => {
                    let n = ["Kick", "Snare", "Hat", "Clap", "Tom", "Hat", "Snare", "Kick"][drum_i % 8];
                    drum_i += 1;
                    (n, 60)
                }
                TrackKind::Pitched => {
                    let n = if n_mel >= 8 {
                        if mel_i < 4 { "Bell" } else if mel_i < 7 { "Pluck" } else { "Bass" }
                    } else if mel_i == n_mel - 1 {
                        "Bass"
                    } else if mel_i == 2 {
                        "Bell"
                    } else {
                        "Pluck"
                    };
                    mel_i += 1;
                    let note = match &key_scale {
                        Some(ns) => ns[kinds.len() - 1 - i],
                        None => {
                            if n == "Bass" { 36 } else { 60 }
                        }
                    };
                    (n, note)
                }
            };
            self.tracks.push(Track {
                kind,
                inst_key: format!("builtin:{}", name),
                inst: crate::synth::builtin_by_name(name),
                note,
                mute: false,
                vol: 0.8,
                chord: 0,
                scale: 0,
                scale_pos: 0,
                gate_left: 0,
            });
        }
    }

    fn nt(&self) -> usize {
        self.tracks.len()
    }

    fn idx(&self, p: usize, t: usize, s: usize) -> usize {
        (p * self.nt() + t) * STEPS + s
    }

    // ---- audio -----------------------------------------------------------

    fn chord_notes(t: &mut Track) -> Vec<u8> {
        let mut base = t.note as i32;
        if t.scale > 0 {
            let iv = SCALE_MODES[t.scale].intervals;
            base += iv[t.scale_pos % iv.len()];
            t.scale_pos += 1;
        }
        CHORD_TYPES[t.chord].intervals.iter().map(|i| clamp_note(base + i)).collect()
    }

    fn play_track(mixer: &mut Mixer, t: &mut Track, ti: usize, vel: u8) {
        Self::play_track_at(mixer, t, ti, vel, 0);
    }

    /// `semis` pitches a drum hit (toms sharing one sound); pitched tracks ignore it.
    fn play_track_at(mixer: &mut Mixer, t: &mut Track, ti: usize, vel: u8, semis: i8) {
        let gain = vel as f32 / 127.0 * t.vol * 0.9;
        match t.kind {
            TrackKind::Drum => Self::drum_hit(mixer, t.inst.as_ref(), gain, ti, semis, 0),
            TrackKind::Pitched => {
                let notes = Self::chord_notes(t);
                let g = gain / (notes.len() as f32).sqrt();
                if let Some(inst) = t.inst.as_ref() {
                    for n in notes {
                        if let Some((s, r)) = inst.pick(n) {
                            mixer.trigger(s, r, g, ti as u32, 8);
                        }
                    }
                }
            }
        }
    }

    fn drum_hit(mixer: &mut Mixer, inst: Option<&Instrument>, gain: f32, ti: usize, semis: i8, delay: u64) {
        if let Some(s) = inst.and_then(|i| i.first()) {
            let ratio = 2f64.powf(semis as f64 / 12.0);
            mixer.trigger_in(s, ratio, gain, ti as u32, 4, delay);
        }
    }

    /// The gate for a pitched track whose sound would otherwise ring for seconds.
    fn gate_of(&self, ti: usize) -> Option<crate::instrument::Gate> {
        if self.tracks[ti].kind != TrackKind::Pitched {
            return None;
        }
        self.tracks[ti].inst.as_ref().and_then(|i| i.gate())
    }

    /// A one-off note outside the sequence (setting a step, choosing a sound):
    /// long sounds are cut off after a moment instead of droning.
    fn audition(&mut self, ti: usize, vel: u8) {
        let gate = self.gate_of(ti);
        if gate.is_some() {
            self.mixer.release(ti as u32);
        }
        Self::play_track(&mut self.mixer, &mut self.tracks[ti], ti, vel);
        self.preview = gate.map(|g| (ti, g.preview_ms));
    }

    fn fire_step(&mut self, step: usize) {
        if step == 0 {
            if let Some(p) = self.pending.take() {
                self.pattern = p;
            }
            if let Some(st) = self.staged.take() {
                self.write_preset(st);
            }
            self.sync_len();
            for t in self.tracks.iter_mut() {
                t.scale_pos = 0;
            }
        }
        for ti in 0..self.tracks.len() {
            if self.tracks[ti].gate_left > 0 {
                self.tracks[ti].gate_left -= 1;
                if self.tracks[ti].gate_left == 0 {
                    self.mixer.release(ti as u32);
                }
            }
            let i = self.idx(self.pattern, ti, step);
            let v = self.cells[i] & !FLAM_BIT;
            if v > 0 && !self.tracks[ti].mute {
                if let Some(g) = self.gate_of(ti) {
                    self.mixer.release(ti as u32); // legato: the new note replaces the held one
                    self.tracks[ti].gate_left = g.steps;
                }
                Self::play_track_at(&mut self.mixer, &mut self.tracks[ti], ti, v, self.pitch[i]);
            }
        }
        // the grace notes of flams on the next step start before it, so they go out now
        let (np, ns) = if step + 1 < self.tr.steps { (self.pattern, step + 1) } else { (self.pending.unwrap_or(self.pattern), 0) };
        let delay = self.tr.until_due().saturating_sub(self.grace_lead());
        for ti in 0..self.nt() {
            // at a bar line a staged preset is what plays next, with its own sounds
            let (c, semis, inst) = match &self.staged {
                Some(st) if ns == 0 && st.pattern == np => {
                    let inst = match st.sounds.get(ti) {
                        Some(Some((_, i))) => Some(i),
                        _ => self.tracks[ti].inst.as_ref(),
                    };
                    (st.rows[ti][0], st.pitch[ti][0], inst)
                }
                _ => {
                    let i = self.idx(np, ti, ns);
                    (self.cells[i], self.pitch[i], self.tracks[ti].inst.as_ref())
                }
            };
            Self::grace(&mut self.mixer, &self.tracks[ti], inst, ti, c, semis, delay);
        }
    }

    fn grace_lead(&self) -> u64 {
        (FLAM_GRACE_MS / 1000.0 * self.rate) as u64
    }

    /// Schedule a flam's grace note `delay` frames from now, if cell `c` is a flam.
    fn grace(mixer: &mut Mixer, t: &Track, inst: Option<&Instrument>, ti: usize, c: u8, semis: i8, delay: u64) {
        if c & FLAM_BIT == 0 || t.mute || t.kind != TrackKind::Drum {
            return;
        }
        let gain = (c & !FLAM_BIT) as f32 / 127.0 * t.vol * 0.9 * FLAM_GRACE_LEVEL;
        Self::drum_hit(mixer, inst, gain, ti, semis, delay);
    }

    /// Point the clock at the current pattern's length. A length change while
    /// stopped or paused rewinds to step 1, so the clock never sits past the end.
    fn sync_len(&mut self) {
        let (len, div) = self.lens[self.pattern];
        if (self.tr.steps, self.tr.div) != (len, div) {
            self.tr.steps = len;
            self.tr.div = div;
            if !self.tr.playing {
                self.tr.stop_reset();
            }
        }
        self.cur_step = self.cur_step.min(len - 1);
    }

    fn len(&self) -> usize {
        self.lens[self.pattern].0
    }

    // ---- state -----------------------------------------------------------

    fn state_string(&self) -> String {
        let mut k = Kv::new();
        k.set("bpm", self.tr.bpm);
        k.set("swing", self.tr.swing);
        if self.cfg.decay {
            k.set_bool("env.on", self.env_on);
            k.set("env.hold", self.env.hold);
            k.set("env.decay", self.env.decay);
            k.set("env.sustain", self.env.sustain);
            k.set("env.release", self.env.release);
        }
        k.set_bool("swing_on", self.tr.swing_on);
        k.set("pattern", self.pattern);
        k.set("cur_track", self.cur_track);
        k.set("cur_step", self.cur_step);
        for (i, t) in self.tracks.iter().enumerate() {
            k.set(&format!("t{}.inst", i), &t.inst_key);
            k.set(&format!("t{}.note", i), t.note);
            k.set_bool(&format!("t{}.mute", i), t.mute);
            k.set(&format!("t{}.vol", i), t.vol);
            k.set(&format!("t{}.chord", i), t.chord);
            k.set(&format!("t{}.scale", i), t.scale);
            for p in 0..PATTERNS {
                let a = self.idx(p, i, 0);
                k.set(&format!("p{}.t{}", p, i), to_hex(&self.cells[a..a + STEPS]));
                let semis = &self.pitch[a..a + STEPS];
                if semis.iter().any(|&x| x != 0) {
                    k.set(&format!("p{}.t{}.pitch", p, i), to_hex(&semis.iter().map(|&x| x as u8).collect::<Vec<u8>>()));
                }
            }
        }
        for (p, &(len, div)) in self.lens.iter().enumerate() {
            if (len, div) != (STEPS, 4) {
                k.set(&format!("p{}.len", p), len);
                k.set(&format!("p{}.div", p), div);
            }
        }
        if self.cfg.presets {
            let (style, pat) = self.pm.position();
            k.set("presets.style", style);
            if let Some(i) = pat {
                k.set("presets.pattern", &presets::builtin().patterns[i].id);
            }
        }
        k.render()
    }

    fn apply_state(&mut self, text: &str) {
        let k = Kv::parse(text);
        if let Some(v) = k.f32("bpm") {
            self.tr.set_bpm(v);
        }
        if self.cfg.decay {
            let d = EnvParams::default();
            self.env_on = k.bool("env.on").unwrap_or(true);
            self.env = EnvParams {
                hold: k.f32("env.hold").unwrap_or(d.hold),
                decay: k.f32("env.decay").unwrap_or(d.decay),
                sustain: k.f32("env.sustain").unwrap_or(d.sustain),
                release: k.f32("env.release").unwrap_or(d.release),
            }
            .clamped();
        }
        if let Some(v) = k.f32("swing") {
            self.tr.swing = v.clamp(0.0, 0.25);
        }
        if let Some(v) = k.bool("swing_on") {
            self.tr.swing_on = v;
        }
        self.pattern = k.usize("pattern").unwrap_or(0).min(PATTERNS - 1);
        self.cur_track = k.usize("cur_track").unwrap_or(0).min(self.nt() - 1);
        self.cur_step = k.usize("cur_step").unwrap_or(0).min(STEPS - 1);
        // tracks often share one sound: load each distinct sound once and share it
        let mut loaded: std::collections::HashMap<String, Option<Instrument>> = std::collections::HashMap::new();
        for i in 0..self.nt() {
            if let Some(key) = k.get(&format!("t{}.inst", i)) {
                if let Some(e) = library::find_key(&self.catalog, key).cloned() {
                    let inst = loaded
                        .entry(e.key.clone())
                        .or_insert_with(|| library::load_entry(self.host.as_ref(), &e, library::BUDGET_GRID))
                        .clone();
                    if let Some(inst) = inst {
                        self.tracks[i].inst = Some(inst);
                        self.tracks[i].inst_key = e.key.clone();
                    }
                }
            }
            let t = &mut self.tracks[i];
            if let Some(v) = k.usize(&format!("t{}.note", i)) {
                t.note = v.clamp(0, 127) as u8;
            }
            t.mute = k.bool(&format!("t{}.mute", i)).unwrap_or(false);
            t.vol = k.f32(&format!("t{}.vol", i)).unwrap_or(0.8).clamp(0.0, if self.cfg.vol_boost { VOL_WARN_MAX } else { 1.0 });
            t.chord = k.usize(&format!("t{}.chord", i)).unwrap_or(0).min(CHORD_TYPES.len() - 1);
            t.scale = k.usize(&format!("t{}.scale", i)).unwrap_or(0).min(SCALE_MODES.len() - 1);
            if !self.cfg.chords {
                t.chord = 0;
                t.scale = 0;
            }
            for p in 0..PATTERNS {
                if let Some(h) = k.get(&format!("p{}.t{}", p, i)) {
                    let bytes = from_hex(h);
                    let a = (p * self.tracks.len() + i) * STEPS;
                    for (s, b) in bytes.iter().take(STEPS).enumerate() {
                        self.cells[a + s] = if *b & !FLAM_BIT == 0 { 0 } else { *b };
                    }
                }
                if let Some(h) = k.get(&format!("p{}.t{}.pitch", p, i)) {
                    let a = (p * self.tracks.len() + i) * STEPS;
                    for (s, b) in from_hex(h).iter().take(STEPS).enumerate() {
                        self.pitch[a + s] = (*b as i8).clamp(-24, 24);
                    }
                }
            }
        }
        for p in 0..PATTERNS {
            let len = k.usize(&format!("p{}.len", p)).unwrap_or(STEPS).clamp(1, STEPS);
            let div = k.usize(&format!("p{}.div", p)).unwrap_or(4).clamp(1, 8) as u32;
            self.lens[p] = (len, div);
        }
        self.sync_len();
        if self.cfg.presets {
            let ps = presets::builtin();
            let pat = k.get("presets.pattern").and_then(|id| ps.patterns.iter().position(|x| x.id == id));
            self.pm.restore(ps, k.usize("presets.style").unwrap_or(0), pat);
        }
    }

    // ---- editing ---------------------------------------------------------

    fn toggle_step(&mut self) {
        let a = self.idx(self.pattern, self.cur_track, self.cur_step);
        self.pitch[a] = 0;
        if self.cells[a] > 0 {
            self.last_vel = self.cells[a] & !FLAM_BIT;
            self.cells[a] = 0;
        } else {
            self.cells[a] = self.last_vel.max(8);
            // audition
            let (ti, v) = (self.cur_track, self.cells[a]);
            if !self.tracks[ti].mute {
                self.audition(ti, v);
                self.tracks[ti].scale_pos = 0;
            }
        }
    }

    fn adjust_velocity(&mut self, d: i32) {
        let a = self.idx(self.pattern, self.cur_track, self.cur_step);
        let flam = self.cells[a] & FLAM_BIT;
        let cur = if self.cells[a] == 0 { self.last_vel as i32 } else { (self.cells[a] & !FLAM_BIT) as i32 };
        let v = (cur + d).clamp(1, 127) as u8;
        self.cells[a] = v | flam;
        self.last_vel = v;
    }

    fn move_cursor(&mut self, b: Btn) {
        match b {
            Btn::Left => self.cur_step = (self.cur_step + self.len() - 1) % self.len(),
            Btn::Right => self.cur_step = (self.cur_step + 1) % self.len(),
            Btn::Up => self.cur_track = (self.cur_track + self.nt() - 1) % self.nt(),
            Btn::Down => self.cur_track = (self.cur_track + 1) % self.nt(),
            _ => {}
        }
    }

    fn transpose(&mut self, b: Btn) {
        let t = &mut self.tracks[self.cur_track];
        if t.kind != TrackKind::Pitched {
            return;
        }
        let d = match b {
            Btn::Left => -1,
            Btn::Right => 1,
            Btn::Up => 12,
            Btn::Down => -12,
            _ => 0,
        };
        t.note = (t.note as i32 + d).clamp(24, 96) as u8;
        let ti = self.cur_track;
        let note = self.tracks[ti].note;
        self.toast.say(&format!("Track {} note {}", ti + 1, note_name(note)));
    }

    fn set_pattern(&mut self, p: usize) {
        if self.tr.playing {
            self.pending = Some(p);
            self.toast.say(&format!("Next: pattern {}", p + 1));
        } else {
            self.pattern = p;
            self.pending = None;
            self.sync_len();
        }
    }

    fn clear_pattern(&mut self) {
        let a = self.idx(self.pattern, 0, 0);
        let n = self.nt() * STEPS;
        self.cells[a..a + n].fill(0);
        self.pitch[a..a + n].fill(0);
        // a cleared pattern is a plain 16-step one again (while playing, the clock
        // follows at the next bar line)
        self.lens[self.pattern] = (STEPS, 4);
        if !self.tr.playing {
            self.sync_len();
        }
        self.toast.say("Pattern cleared");
    }

    fn change_bpm(&mut self, d: f32) {
        self.tr.set_bpm(self.tr.bpm + d);
    }

    fn change_swing(&mut self, d: f32) {
        self.tr.swing = (self.tr.swing + d).clamp(0.0, 0.25);
        self.toast.say(&format!("Swing {}%", self.tr.swing_percent()));
    }

    fn use_mods(&mut self) {
        self.plain.start = false;
        self.plain.select = false;
    }

    /// The key of the drum kit whose sounds the drum tracks currently hold, in order, if any
    /// (a kit fills the drum tracks in order; a single sound chosen since breaks the match).
    fn applied_kit(&self) -> Option<String> {
        let drums: Vec<usize> = (0..self.nt()).filter(|&i| self.tracks[i].kind == TrackKind::Drum).collect();
        self.catalog.iter().find_map(|e| match &e.kind {
            library::Kind::Kit(files) if !files.is_empty() => {
                let all = drums.iter().zip(files).all(|(&t, f)| self.tracks[t].inst_key == *f);
                all.then(|| e.key.clone())
            }
            _ => None,
        })
    }

    fn open_browser(&mut self) {
        let mut targets: Vec<usize> = (0..self.nt()).filter(|&i| self.sel[i]).collect();
        if targets.is_empty() {
            targets.push(self.cur_track);
        }
        let first = targets[0];
        let drum = self.cfg.drum_filter && self.tracks[first].kind == TrackKind::Drum;
        let title = if targets.len() > 1 { format!("Sound for {} tracks", targets.len()) } else { format!("Sound for track {}", first + 1) };
        let mut cur = self.tracks[first].inst_key.clone();
        if drum {
            // a whole kit is applied: open on the kit's row, not inside its folder
            if let Some(kit) = self.applied_kit() {
                cur = kit;
            }
        }
        self.browser.open(&title, &self.catalog, drum, false, &[], Some(&cur));
        self.overlay = Overlay::Browse(targets);
    }

    /// Spread a kit's files over every drum track, in track order.
    fn assign_kit(&mut self, files: &[String]) {
        let drums: Vec<usize> = (0..self.nt()).filter(|&i| self.tracks[i].kind == TrackKind::Drum).collect();
        let mut n = 0;
        for (&ti, path) in drums.iter().zip(files) {
            let stem = path.rsplit('/').next().unwrap_or(path);
            let one = Entry {
                dir: String::new(),
                label: stem.rsplit_once('.').map(|(s, _)| s).unwrap_or(stem).to_string(),
                key: path.clone(),
                kind: library::Kind::Files(vec![(path.clone(), None)]),
                pitched: false,
            };
            if let Some(inst) = library::load_entry(self.host.as_ref(), &one, library::BUDGET_GRID) {
                let t = &mut self.tracks[ti];
                t.inst = Some(inst);
                t.inst_key = path.clone();
                n += 1;
            }
        }
        self.sel.iter_mut().for_each(|s| *s = false);
        if let Some(&ti) = drums.first() {
            if !self.tracks[ti].mute {
                Self::play_track(&mut self.mixer, &mut self.tracks[ti], ti, 100);
            }
        }
        self.toast.say(&format!("Kit loaded: {} voices", n));
    }

    fn assign(&mut self, targets: &[usize], e: &Entry) {
        if let library::Kind::Kit(files) = &e.kind {
            self.assign_kit(files);
            return;
        }
        match library::load_entry(self.host.as_ref(), e, library::BUDGET_GRID) {
            Some(inst) => {
                let name = inst.name.clone();
                for &ti in targets {
                    let t = &mut self.tracks[ti];
                    t.inst = Some(inst.clone());
                    t.inst_key = e.key.clone();
                }
                self.sel.iter_mut().for_each(|s| *s = false);
                let ti = targets[0];
                if !self.tracks[ti].mute {
                    self.audition(ti, 100);
                    self.tracks[ti].scale_pos = 0;
                }
                self.toast.say(&format!("{} -> {}", clip(&name, 18), if targets.len() > 1 { format!("{} tracks", targets.len()) } else { format!("track {}", ti + 1) }));
            }
            None => self.toast.say("Could not load sound"),
        }
    }

    fn apply_key(&mut self, root: usize, scale: usize) {
        let n = self.nt();
        let notes = scale_notes(48 + root as i32, scale, n);
        for i in 0..n {
            if self.tracks[i].kind == TrackKind::Pitched {
                self.tracks[i].note = notes[n - 1 - i];
            }
        }
        self.toast.say(&format!("Key {} {}", PC_NAMES[root], SCALE_MODES[scale].name));
    }

    // ---- preset patterns (HillBeat) -----------------------------------------

    /// Replace a pattern's notes with a preset's, and the tracks' sounds with the
    /// ones the preset was laid out for. Tempo, swing, mute and volume are left alone.
    fn write_preset(&mut self, st: Staged) {
        for ti in 0..self.nt() {
            let a = self.idx(st.pattern, ti, 0);
            self.cells[a..a + STEPS].fill(0);
            self.pitch[a..a + STEPS].fill(0);
            if let Some(row) = st.rows.get(ti) {
                self.cells[a..a + row.len()].copy_from_slice(row);
                self.pitch[a..a + row.len()].copy_from_slice(&st.pitch[ti]);
            }
            if let Some(Some((key, inst))) = st.sounds.get(ti) {
                self.tracks[ti].inst = Some(inst.clone());
                self.tracks[ti].inst_key = key.clone();
            }
        }
        self.lens[st.pattern] = (st.len, st.div);
    }

    fn plan_of(&self, pi: usize) -> Plan {
        presets::builtin().patterns[pi].plan(&self.kits, self.nt())
    }

    /// Load (or reuse) a preset sound: (inst key, instrument).
    fn load_sound(&mut self, s: &Sound) -> Option<(String, Instrument)> {
        let key = match s {
            Sound::Builtin(n) => format!("builtin:{}", n),
            Sound::File(p) => p.clone(),
        };
        if let Some(i) = self.sound_cache.get(&key) {
            return Some((key, i.clone()));
        }
        let inst = match s {
            Sound::Builtin(n) => crate::synth::builtin_by_name(n),
            Sound::File(path) => {
                let stem = path.rsplit('/').next().unwrap_or(path);
                let one = Entry {
                    dir: String::new(),
                    label: stem.rsplit_once('.').map(|(s, _)| s).unwrap_or(stem).to_string(),
                    key: path.clone(),
                    kind: library::Kind::Files(vec![(path.clone(), None)]),
                    pitched: false,
                };
                library::load_entry(self.host.as_ref(), &one, library::BUDGET_GRID)
            }
        }?;
        self.sound_cache.insert(key.clone(), inst.clone());
        Some((key, inst))
    }

    fn load_preset(&mut self, pi: usize) {
        let x = &presets::builtin().patterns[pi];
        let plan = self.plan_of(pi);
        let sounds: Vec<Option<(String, Instrument)>> = plan.voices.iter().map(|v| self.load_sound(&v.sound)).collect();
        let st = Staged { pattern: self.pattern, rows: plan.cells, pitch: plan.pitch, len: x.steps, div: x.div, sounds };
        let kit = plan.kit.map(|k| format!(" ({})", clip(&k, 14))).unwrap_or_default();
        if self.tr.playing {
            // at the next bar line, so the groove does not jump mid-bar
            self.staged = Some(st);
            self.toast.say(&format!("{}: next bar", clip(&x.name, 18)));
        } else {
            self.write_preset(st);
            self.sync_len();
            self.toast.say(&format!("{}{}", clip(&x.name, 18), kit));
        }
    }

    /// Lay out the highlighted preset for the preview (on every cursor move).
    fn refresh_plan(&mut self) {
        let hi = self.pm.highlighted();
        if hi != self.pm_plan.as_ref().map(|p| p.0) {
            self.pm_plan = hi.map(|pi| (pi, self.plan_of(pi)));
        }
    }

    fn pattern_has_notes(&self) -> bool {
        let a = self.idx(self.pattern, 0, 0);
        self.cells[a..a + self.nt() * STEPS].iter().any(|&c| c != 0)
    }

    fn open_presets(&mut self) {
        self.pm.open(presets::builtin());
        self.pm_plan = None;
        self.refresh_plan();
        self.overlay = Overlay::Presets;
    }

    fn close_presets(&mut self) {
        self.stop_audition();
        self.pm.confirm = None;
        self.rep.clear();
        self.overlay = Overlay::None;
    }

    fn start_audition(&mut self, pi: usize) {
        let x = &presets::builtin().patterns[pi];
        let mut tr = Transport::new(self.rate, x.steps);
        tr.div = x.div;
        tr.bpm = self.tr.bpm;
        tr.swing = self.tr.swing;
        tr.swing_on = self.tr.swing_on;
        tr.playing = true;
        self.mixer.release_all();
        let plan = self.plan_of(pi);
        let insts = plan.voices.iter().map(|v| self.load_sound(&v.sound).map(|s| s.1)).collect();
        self.aud = Some(Audition { preset: pi, tr, plan, insts });
    }

    fn stop_audition(&mut self) {
        if self.aud.take().is_some() {
            self.mixer.release_all();
        }
    }

    /// One press in the preset menu (also re-fired by hold-to-repeat).
    fn preset_nav(&mut self, b: Btn) {
        let ps = presets::builtin();
        if let Some(pi) = self.pm.confirm {
            match b {
                Btn::A => {
                    self.close_presets();
                    self.load_preset(pi);
                }
                Btn::B => self.pm.confirm = None,
                _ => {}
            }
            return;
        }
        self.pm.note.clear();
        match self.pm.event(b, ps) {
            MenuOut::None => {}
            MenuOut::Close => self.close_presets(),
            MenuOut::Load(pi) => {
                if self.pattern_has_notes() {
                    self.pm.confirm = Some(pi);
                } else {
                    self.close_presets();
                    self.load_preset(pi);
                }
                return;
            }
            MenuOut::Audition => {
                if self.aud.is_some() {
                    self.stop_audition();
                } else if self.tr.playing {
                    // the menu's player shares the mixer: never under the running sequence
                    self.pm.note = "Pause the sequencer to listen".into();
                } else if let Some(pi) = self.pm.highlighted() {
                    self.start_audition(pi);
                }
            }
        }
        self.refresh_plan();
        // listening follows the cursor; leaving the pattern list stops it
        if let Some(cur) = self.aud.as_ref().map(|a| a.preset) {
            match self.pm.highlighted() {
                Some(pi) if pi != cur => self.start_audition(pi),
                None => self.stop_audition(),
                _ => {}
            }
        }
    }

    fn preset_event(&mut self, b: Btn) {
        let (m0, m1) = PRESET_MENU_COMBO;
        if b == m1 && self.held.is(m0) {
            self.close_presets();
            return;
        }
        self.preset_nav(b);
        if matches!(self.overlay, Overlay::Presets) && matches!(b, Btn::Up | Btn::Down) {
            self.rep.start(b);
        }
    }

    fn save_now(&mut self) {
        let s = self.state_string();
        self.host.save_state(self.cfg.file, &s);
    }

    /// Step one line of the envelope screen up (+1) or down (-1).
    fn adjust_env(&mut self, row: usize, dir: i32) {
        let d = dir as f32;
        let e = &mut self.env;
        let step = |v: f32, r: (f32, f32, f32)| ((v + d * r.2).clamp(r.0, r.1) / r.2).round() * r.2;
        match row {
            0 => self.env_on = dir > 0,
            1 => e.hold = step(e.hold, EnvParams::HOLD),
            2 => e.decay = step(e.decay, EnvParams::DECAY),
            3 => e.sustain = step(e.sustain, EnvParams::SUSTAIN),
            _ => e.release = step(e.release, EnvParams::RELEASE),
        }
        self.env = self.env.clamped();
    }

    // ---- input -----------------------------------------------------------

    fn press(&mut self, b: Btn) {
        let (m0, m1) = PRESET_MENU_COMBO;
        if self.cfg.presets && b == m1 && self.held.is(m0) {
            self.use_mods();
            self.open_presets();
            return;
        }
        let st = self.held.is(Btn::Start);
        let se = self.held.is(Btn::Select);
        let fnh = self.held.is(Btn::Fn);
        match b {
            Btn::Start => {
                if se {
                    self.ss = true;
                    self.ss_ms = 0;
                    self.use_mods();
                } else {
                    self.plain.start = true;
                }
            }
            Btn::Select => {
                if fnh {
                    self.help.show();
                    self.overlay = Overlay::Help;
                } else if st {
                    self.ss = true;
                    self.ss_ms = 0;
                    self.use_mods();
                } else {
                    self.plain.select = true;
                }
            }
            Btn::Fn => {
                if se {
                    self.help.show();
                    self.overlay = Overlay::Help;
                    self.use_mods();
                }
            }
            Btn::A => {
                if st {
                    self.use_mods();
                } else if !se && !fnh {
                    self.plain.a = true;
                }
            }
            Btn::B => {
                if st {
                    self.use_mods();
                    self.open_browser();
                }
            }
            Btn::X => {
                if st {
                    self.use_mods();
                    if self.cfg.decay {
                        self.overlay = Overlay::Env { row: 1 };
                    }
                } else if se {
                    self.use_mods();
                    self.tr.swing_on = !self.tr.swing_on;
                    self.toast.say(if self.tr.swing_on { "Swing on" } else { "Swing off" });
                } else {
                    self.plain.x = true;
                }
            }
            Btn::Y => {
                if st {
                    self.use_mods();
                    self.save_now();
                    self.toast.say("Saved");
                } else if !se && !fnh {
                    self.plain.y = true;
                }
            }
            Btn::L1 | Btn::R1 => {
                if se && b == Btn::L1 {
                    self.use_mods();
                    self.clear_pattern();
                } else if !st {
                    let cur = self.pending.unwrap_or(self.pattern);
                    let n = if b == Btn::L1 { (cur + PATTERNS - 1) % PATTERNS } else { (cur + 1) % PATTERNS };
                    self.set_pattern(n);
                }
            }
            Btn::L2 | Btn::R2 => {
                if st && b == Btn::R2 && self.cfg.multiselect {
                    self.use_mods();
                    // toggle every track of the cursor track's kind (all melodic, or all drums)
                    let kind = self.tracks[self.cur_track].kind;
                    let group: Vec<usize> = (0..self.nt()).filter(|&i| self.tracks[i].kind == kind).collect();
                    let all_on = group.iter().all(|&i| self.sel[i]);
                    self.sel.iter_mut().for_each(|s| *s = false);
                    if !all_on {
                        group.iter().for_each(|&i| self.sel[i] = true);
                        self.toast.say(&format!("Selected {} tracks", group.len()));
                    } else {
                        self.toast.say("Selection cleared");
                    }
                } else if st && b == Btn::L2 {
                    self.use_mods();
                    if self.cfg.chords {
                        if self.tracks[self.cur_track].kind == TrackKind::Pitched {
                            let t = &self.tracks[self.cur_track];
                            self.overlay = Overlay::Chord { track: self.cur_track, chord: t.chord, scale: t.scale };
                        } else {
                            self.toast.say("Drum tracks have no chords");
                        }
                    } else if self.cfg.key_overlay {
                        self.overlay = Overlay::Key { root: 0, scale: 1 };
                    }
                } else if se {
                    self.use_mods();
                    self.change_swing(if b == Btn::L2 { -0.01 } else { 0.01 });
                    self.rep.start(b);
                } else if !st {
                    self.change_bpm(if b == Btn::L2 { -1.0 } else { 1.0 });
                    self.rep.start(b);
                }
            }
            Btn::Up | Btn::Down | Btn::Left | Btn::Right => {
                let ud = if b == Btn::Up { 1 } else if b == Btn::Down { -1 } else { 0 };
                if self.held.is(Btn::A) {
                    self.plain.a = false;
                    if ud != 0 {
                        self.adjust_velocity(ud * 8);
                    }
                } else if self.held.is(Btn::Y) {
                    self.plain.y = false;
                    if ud != 0 {
                        let max = if self.cfg.vol_boost { VOL_WARN_MAX } else { 1.0 };
                        let t = &mut self.tracks[self.cur_track];
                        // above 100% the ticks are bigger (like the 0.25 steps of the Python app)
                        let step = if (ud > 0 && t.vol >= 0.999) || (ud < 0 && t.vol > 1.001) { VOL_WARN_STEP } else { 0.05 };
                        let v = t.vol + ud as f32 * step;
                        // land exactly on 100% when crossing it, then round off float drift
                        let v = if (t.vol - 1.0) * (v - 1.0) < 0.0 { 1.0 } else { v };
                        t.vol = ((v * 100.0).round() / 100.0).clamp(0.0, max);
                    }
                } else if self.held.is(Btn::X) {
                    self.plain.x = false;
                    self.transpose(b);
                } else {
                    self.move_cursor(b);
                }
            }
            Btn::L3 | Btn::R3 => {}
        }
    }

    fn release(&mut self, b: Btn) {
        match b {
            Btn::A => {
                if self.plain.a {
                    self.toggle_step();
                }
                self.plain.a = false;
            }
            Btn::Y => {
                if self.plain.y {
                    let t = &mut self.tracks[self.cur_track];
                    t.mute = !t.mute;
                }
                self.plain.y = false;
            }
            Btn::X => self.plain.x = false,
            Btn::Start => {
                if self.ss {
                    self.finish_ss();
                } else if self.plain.start {
                    self.tr.toggle();
                    if !self.tr.playing {
                        // pausing silences everything still ringing
                        self.mixer.release_all();
                        self.tracks.iter_mut().for_each(|t| t.gate_left = 0);
                    }
                }
                self.plain.start = false;
            }
            Btn::Select => {
                if self.ss {
                    self.finish_ss();
                } else if self.plain.select {
                    if let Some(bpm) = self.tap.tap(self.ms) {
                        self.tr.set_bpm(bpm);
                    }
                }
                self.plain.select = false;
            }
            Btn::L2 | Btn::R2 => self.rep.stop(b),
            _ => {}
        }
    }

    fn finish_ss(&mut self) {
        self.ss = false;
        if !self.quit {
            self.tr.stop_reset();
            self.mixer.stop_all();
            self.pending = None;
            self.toast.say("Stopped");
        }
    }

    fn overlay_event(&mut self, b: Btn) {
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        match overlay {
            Overlay::None => {}
            Overlay::Presets => {
                self.overlay = Overlay::Presets;
                self.preset_event(b);
            }
            Overlay::Help => {
                self.help.event(b, self.cfg.help.len());
                if self.help.open {
                    self.overlay = Overlay::Help;
                }
            }
            Overlay::Browse(targets) => match self.browser.event(b, &self.catalog, &[]) {
                BrowserOut::Pick(i) => {
                    let e = self.catalog[i].clone();
                    self.assign(&targets, &e);
                }
                BrowserOut::Close => {}
                _ => self.overlay = Overlay::Browse(targets),
            },
            Overlay::Chord { track, mut chord, mut scale } => {
                match b {
                    Btn::Left => chord = (chord + CHORD_TYPES.len() - 1) % CHORD_TYPES.len(),
                    Btn::Right => chord = (chord + 1) % CHORD_TYPES.len(),
                    Btn::Up => scale = (scale + SCALE_MODES.len() - 1) % SCALE_MODES.len(),
                    Btn::Down => scale = (scale + 1) % SCALE_MODES.len(),
                    Btn::A => {
                        let t = &mut self.tracks[track];
                        t.chord = chord;
                        t.scale = scale;
                        t.scale_pos = 0;
                        self.toast.say(&format!("{} / {}", CHORD_TYPES[chord].name, SCALE_MODES[scale].name));
                        return;
                    }
                    Btn::B => return,
                    _ => {}
                }
                self.overlay = Overlay::Chord { track, chord, scale };
            }
            Overlay::Env { mut row } => {
                match b {
                    Btn::Up => row = (row + 4) % 5,
                    Btn::Down => row = (row + 1) % 5,
                    Btn::Left | Btn::Right => {
                        self.adjust_env(row, if b == Btn::Left { -1 } else { 1 });
                        self.rep.start(b);
                    }
                    Btn::X => {
                        self.env = EnvParams::default();
                        self.env_on = true;
                        self.toast.say("Envelope reset");
                    }
                    Btn::A | Btn::B => {
                        self.rep.clear();
                        self.save_now();
                        return;
                    }
                    _ => {}
                }
                self.overlay = Overlay::Env { row };
            }
            Overlay::Key { mut root, mut scale } => {
                match b {
                    Btn::Left => root = (root + 11) % 12,
                    Btn::Right => root = (root + 1) % 12,
                    Btn::Up => scale = (scale + SCALE_MODES.len() - 1) % SCALE_MODES.len(),
                    Btn::Down => scale = (scale + 1) % SCALE_MODES.len(),
                    Btn::A => {
                        self.apply_key(root, scale);
                        return;
                    }
                    Btn::B => return,
                    _ => {}
                }
                self.overlay = Overlay::Key { root, scale };
            }
        }
    }

    // ---- drawing ---------------------------------------------------------

    fn draw_grid(&self, fb: &mut Fb) {
        let nt = self.nt();
        let x0 = 130;
        let cw = 28;
        let y0 = 58;
        let rh = row_h(nt);
        let play = self.tr.playhead().filter(|_| self.tr.playing);
        // a 12-step preset shows 12 columns, grouped by beat (3 for triplets and 12/8)
        let (len, div) = self.lens[self.pattern];
        let div = div.max(1) as usize;
        let col_x = |s: usize| x0 + s as i32 * 30 + (s / div) as i32 * 4;

        // step numbers + playhead
        for s in 0..len {
            let x = col_x(s);
            let on = play == Some(s);
            if on {
                fb.rect(x, y0 - 16, cw, 12, ACCENT);
            }
            fb.text_center(x + cw / 2, y0 - 15, &format!("{}", s + 1), 1, if on { BG } else { DIM });
        }

        for ti in 0..nt {
            let t = &self.tracks[ti];
            let y = y0 + ti as i32 * (rh + 4);
            let base = if t.kind == TrackKind::Drum { DRUM } else { MELODIC };
            let sel_cur = ti == self.cur_track;
            // label
            fb.rect(6, y, 118, rh, if sel_cur { rgb(34, 42, 58) } else { PANEL });
            fb.text(10, y + 4, &format!("{}", ti + 1), 2, if t.mute { DIM } else { base });
            if self.sel[ti] {
                fb.rect(110, y + 4, 8, 8, ACCENT);
            }
            let name = t.inst.as_ref().map(|i| i.name.as_str()).unwrap_or("-");
            fb.text(28, y + 4, &clip(name, 11), 1, if t.mute { DIM } else { TEXT });
            let mut sub = String::new();
            if t.kind == TrackKind::Pitched {
                sub.push_str(&note_name(t.note));
                if self.cfg.chords && (t.chord > 0 || t.scale > 0) {
                    sub.push(' ');
                    sub.push_str(&clip(CHORD_TYPES[t.chord].name, 5));
                }
            }
            if t.mute {
                sub.push_str(" MUTE");
            }
            fb.text(28, y + 16, &sub, 1, DIM);
            // volume bar
            // above 100% the bar shows the boost amount in a warning colour
            fb.rect(10, y + rh - 8, 100, 4, CELL_OFF);
            if t.vol > 1.0 {
                let vb = ((t.vol - 1.0) / (VOL_WARN_MAX - 1.0) * 100.0) as i32;
                fb.rect(10, y + rh - 8, vb, 4, if t.mute { scale(WARN, 0.3) } else { WARN });
            } else {
                let vb = (t.vol * 100.0) as i32;
                fb.rect(10, y + rh - 8, vb, 4, scale(base, if t.mute { 0.3 } else { 0.8 }));
            }

            for s in 0..len {
                let x = col_x(s);
                let c = self.cells[self.idx(self.pattern, ti, s)];
                let v = c & !FLAM_BIT;
                let head = play == Some(s);
                let off = if (s / div) % 2 == 0 { CELL_BEAT } else { CELL_OFF };
                fb.rect(x, y, cw, rh, if head { scale(off, 1.5) } else { off });
                if v > 0 {
                    let dimf = if t.mute { 0.35 } else { 1.0 };
                    fb.rect(x, y, cw, rh, scale(base, 0.25 * dimf));
                    let fh = (rh * v as i32 / 127).max(3);
                    fb.rect(x, y + rh - fh, cw, fh, scale(base, dimf * if head { 1.2 } else { 1.0 }));
                    if c & FLAM_BIT != 0 {
                        // flam: a grace mark at the left of the hit
                        fb.rect(x + 3, y + 4, 4, rh - 8, WHITE);
                    }
                }
                if sel_cur && s == self.cur_step {
                    fb.frame(x - 2, y - 2, cw + 4, rh + 4, 2, WHITE);
                }
            }
        }
    }

    fn draw_header(&self, fb: &mut Fb) {
        fb.rect(0, 0, W, 30, PANEL);
        fb.text(8, 7, self.cfg.name, 2, ACCENT);
        let pat = match self.pending {
            Some(p) => format!("P{}>{}", self.pattern + 1, p + 1),
            None => format!("P{}/{}", self.pattern + 1, PATTERNS),
        };
        fb.text(232, 7, &pat, 2, TEXT);
        fb.text(328, 7, &format!("{:.0}bpm", self.tr.bpm), 2, TEXT);
        if self.tr.swing_on {
            fb.text(440, 7, &format!("sw{}%", self.tr.swing_percent()), 2, MELODIC);
        }
        let (label, col) = if self.tr.playing { (">", ACCENT) } else { ("||", DIM) };
        fb.text_right(W - 8, 7, label, 2, col);
    }

    fn draw_detail(&self, fb: &mut Fb) {
        let t = &self.tracks[self.cur_track];
        let rh = row_h(self.nt());
        let y = 58 + self.nt() as i32 * (rh + 4) + 6;
        fb.rect(6, y, W - 12, 92, PANEL);
        let inst = t.inst.as_ref().map(|i| i.name.as_str()).unwrap_or("-");
        let mut l1 = format!("Track {}  {}", self.cur_track + 1, clip(inst, 24));
        if t.kind == TrackKind::Pitched {
            l1.push_str(&format!("  note {}", note_name(t.note)));
        }
        fb.text(14, y + 8, &l1, 2, TEXT);
        let c = self.cells[self.idx(self.pattern, self.cur_track, self.cur_step)];
        let v = c & !FLAM_BIT;
        let flam = if c & FLAM_BIT != 0 { " flam" } else { "" };
        let l2 = format!("Step {}  {}   vol {}%", self.cur_step + 1, if v > 0 { format!("vel {}{}", v, flam) } else { "off".into() }, (t.vol * 100.0).round());
        fb.text(14, y + 30, &l2, 2, DIM);
        let mut l3 = String::new();
        if self.cfg.chords && t.kind == TrackKind::Pitched {
            l3.push_str(&format!("chord {}  scale {}", CHORD_TYPES[t.chord].name, SCALE_MODES[t.scale].name));
        }
        fb.text(14, y + 52, &l3, 1, DIM);
        let hint = "A step  Y mute  X xpose  START play  SEL tap  FN+SEL help";
        fb.text(14, y + 72, hint, 1, DIM);
    }
}

fn row_h(tracks: usize) -> i32 {
    if tracks <= 4 {
        62
    } else {
        34
    }
}

impl App for GridApp {
    fn name(&self) -> &'static str {
        self.cfg.name
    }

    fn event(&mut self, b: Btn, down: bool) {
        if down {
            if !matches!(self.overlay, Overlay::None) {
                self.held.set(b, true);
                self.overlay_event(b);
                return;
            }
            self.press(b);
            self.held.set(b, true);
        } else {
            self.held.set(b, false);
            if matches!(self.overlay, Overlay::None) {
                self.release(b);
            } else {
                self.rep.stop(b);
            }
        }
    }

    fn audio(&mut self, out: &mut [f32]) {
        out.iter_mut().for_each(|x| *x = 0.0);
        let frames = out.len() / 2;
        let mut done = 0usize;
        let lead = self.grace_lead();
        while done < frames {
            while let Some(s) = self.tr.take_due() {
                self.fire_step(s);
            }
            if let Some(a) = self.aud.as_mut() {
                // the menu's player: the preset with its own sounds, on the tracks' volume/mute
                while let Some(s) = a.tr.take_due() {
                    let ns = (s + 1) % a.tr.steps;
                    let delay = a.tr.until_due().saturating_sub(lead);
                    for (ti, t) in self.tracks.iter().enumerate() {
                        let (cells, semis) = (&a.plan.cells[ti], &a.plan.pitch[ti]);
                        let v = cells[s] & !FLAM_BIT;
                        let inst = a.insts[ti].as_ref();
                        if v > 0 && !t.mute {
                            Self::drum_hit(&mut self.mixer, inst, v as f32 / 127.0 * t.vol * 0.9, ti, semis[s], 0);
                        }
                        Self::grace(&mut self.mixer, t, inst, ti, cells[ns], semis[ns], delay);
                    }
                }
            }
            let due = self.tr.until_due().min(self.aud.as_ref().map_or(u64::MAX, |a| a.tr.until_due()));
            let chunk = (due.min((frames - done) as u64) as usize).max(1);
            let seg = &mut out[done * 2..(done + chunk) * 2];
            self.mixer.render(seg);
            self.tr.advance(chunk as u64);
            if let Some(a) = self.aud.as_mut() {
                a.tr.advance(chunk as u64);
            }
            done += chunk;
        }
        for x in out.iter_mut() {
            *x = x.clamp(-1.0, 1.0);
        }
    }

    fn render(&self, fb: &mut Fb) {
        fb.clear(BG);
        self.draw_header(fb);
        self.draw_grid(fb);
        self.draw_detail(fb);
        self.toast.draw(fb);
        if self.ss && self.held.is(Btn::Start) && self.held.is(Btn::Select) {
            quit_bar(fb, self.ss_ms);
        }
        match &self.overlay {
            Overlay::None => {}
            Overlay::Help => self.help.draw(fb, self.cfg.name, self.cfg.help),
            Overlay::Browse(_) => self.browser.draw(fb, &self.catalog, &[]),
            Overlay::Presets => {
                let plan = self.pm_plan.as_ref().map(|p| &p.1);
                self.pm.draw(fb, presets::builtin(), plan, self.aud.is_some());
            }
            Overlay::Chord { chord, scale, .. } => {
                let (x, y) = modal(fb, 440, 200, "Chord / scale");
                fb.text(x, y + 4, "Chord", 2, DIM);
                fb.text(x + 130, y + 4, &format!("< {} >", CHORD_TYPES[*chord].name), 2, WHITE);
                fb.text(x, y + 44, "Scale", 2, DIM);
                fb.text(x + 130, y + 44, &format!("^ {} v", SCALE_MODES[*scale].name), 2, WHITE);
                fb.text(x, y + 100, "L/R chord  U/D scale  A apply  B cancel", 1, DIM);
            }
            Overlay::Env { row } => {
                let (x, y) = modal(fb, 520, 360, "Envelope: long sounds");
                let e = &self.env;
                let lines: [(&str, String); 5] = [
                    ("Envelope", if self.env_on { "ON".into() } else { "OFF".into() }),
                    ("Attack hold", format!("{:.2} beats", e.hold)),
                    ("Decay", format!("{:.3} beats", e.decay)),
                    ("Sustain", format!("{:.0}%", e.sustain * 100.0)),
                    ("Release", format!("x{:.2}", e.release)),
                ];
                for (i, (name, val)) in lines.iter().enumerate() {
                    let ly = y + i as i32 * 30;
                    if i == *row {
                        fb.rect(x - 6, ly - 4, 492, 28, scale(MELODIC, 0.45));
                    }
                    let dim = i > 0 && !self.env_on;
                    fb.text(x, ly, name, 2, if dim { DIM } else { TEXT });
                    fb.text(x + 230, ly, &format!("< {} >", val), 2, if i == *row { WHITE } else if dim { DIM } else { ACCENT });
                }
                // the curve for a sound 8 beats long (one bar of x = one pixel column)
                let (gx, gy, gw, gh) = (x, y + 160, 480, 70);
                fb.rect(gx, gy, gw, gh, scale(PANEL, 0.6));
                if self.env_on {
                    if let Some(env) = Envelope::new(1.0, 8.0, e) {
                        for px in 0..gw {
                            let g = env.gain(px as f64 / gw as f64 * 8.0);
                            let h = ((gh - 4) as f32 * g) as i32;
                            fb.rect(gx + px, gy + gh - 2 - h, 1, h.max(1), MELODIC);
                        }
                    }
                } else {
                    fb.text(gx + 150, gy + 26, "envelope off", 2, DIM);
                }
                for k in 1..8 {
                    fb.rect(gx + gw * k / 8, gy, 1, gh, DIM);
                }
                fb.text(x, y + 236, "example: a sound 8 beats long (lines = beats)", 1, DIM);
                fb.text(x, y + 256, "U/D line   L/R change (hold repeats)   X reset   A/B close", 1, DIM);
            }
            Overlay::Key { root, scale } => {
                let (x, y) = modal(fb, 440, 200, "Key / scale");
                fb.text(x, y + 4, "Root", 2, DIM);
                fb.text(x + 130, y + 4, &format!("< {} >", PC_NAMES[*root]), 2, WHITE);
                fb.text(x, y + 44, "Scale", 2, DIM);
                fb.text(x + 130, y + 44, &format!("^ {} v", SCALE_MODES[*scale].name), 2, WHITE);
                fb.text(x, y + 100, "Spreads across all tracks. A apply  B cancel", 1, DIM);
            }
        }
    }

    fn tick(&mut self, dt: u32) {
        self.ms += dt as u64;
        self.toast.tick(dt);
        self.mixer.max_secs = Some(4.0 * 4.0 * 60.0 / self.tr.bpm.max(1.0)); // 4 measures
        self.mixer.beat_secs = Some(60.0 / self.tr.bpm.max(1.0));
        self.mixer.env = (self.cfg.decay && self.env_on).then_some(self.env);
        if let Some((ti, left)) = self.preview {
            if left <= dt {
                self.mixer.release_quick(ti as u32);
                self.preview = None;
            } else {
                self.preview = Some((ti, left - dt));
            }
        }
        if self.ss && self.held.is(Btn::Start) && self.held.is(Btn::Select) {
            self.ss_ms += dt;
            if self.ss_ms >= QUIT_HOLD_MS {
                self.quit = true;
                self.ss = false;
                self.save_now();
            }
        }
        if matches!(self.overlay, Overlay::None) {
            if let Some(b) = self.rep.tick(dt) {
                let se = self.held.is(Btn::Select);
                if se {
                    self.change_swing(if b == Btn::L2 { -0.01 } else { 0.01 });
                } else {
                    self.change_bpm(if b == Btn::L2 { -1.0 } else { 1.0 });
                }
            }
        } else if let Overlay::Env { row } = self.overlay {
            if let Some(b) = self.rep.tick(dt) {
                self.adjust_env(row, if b == Btn::Left { -1 } else { 1 });
            }
        } else if matches!(self.overlay, Overlay::Presets) {
            if let Some(b) = self.rep.tick(dt) {
                self.preset_nav(b);
            }
        } else {
            self.rep.clear();
        }
    }

    fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
        self.mixer.set_rate(rate);
        self.tr.set_rate(rate);
    }

    fn wants_quit(&self) -> bool {
        self.quit
    }

    fn save(&mut self) {
        self.save_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::encode16;
    use crate::{App, MemHost};

    fn tap(a: &mut dyn App, b: Btn) {
        a.event(b, true);
        a.event(b, false);
    }

    fn app_with_two_kits() -> GridApp {
        let wav = encode16(&(0..2000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect::<Vec<_>>(), 1, 44100);
        let mut host = MemHost::default();
        for kit in ["Dusty", "Crisp"] {
            for s in ["kick", "snare", "hat", "perc"] {
                host.samples.insert(format!("Packs/{}/{}_01.wav", kit, s), wav.clone());
            }
        }
        crate::apps::beat::new(Box::new(host))
    }

    #[test]
    fn reopening_the_browser_after_a_kit_lands_on_the_kit_row() {
        let mut a = app_with_two_kits();
        // open, go up to the root, into Packs, apply the first kit (Crisp sorts before Dusty)
        a.event(Btn::Start, true);
        tap(&mut a, Btn::B);
        a.event(Btn::Start, false);
        tap(&mut a, Btn::B);
        tap(&mut a, Btn::Down);
        tap(&mut a, Btn::A);
        assert_eq!(a.browser.cwd(), "Packs", "two kit rows, no kit folders");
        tap(&mut a, Btn::A);
        assert!(a.applied_kit().is_some(), "the drum tracks hold a whole kit");
        // open it again: it opens in Packs on the kit, not inside the kit's folder
        a.event(Btn::Start, true);
        tap(&mut a, Btn::B);
        a.event(Btn::Start, false);
        assert_eq!(a.browser.cwd(), "Packs");
        // pick the other kit with a single A press from the cursor's new position
        tap(&mut a, Btn::Down);
        tap(&mut a, Btn::A);
        let k = a.applied_kit().unwrap();
        assert!(k.ends_with("Dusty"), "second kit applied: {}", k);
    }

    fn beat() -> GridApp {
        crate::apps::beat::new(Box::new(MemHost::default()))
    }

    fn combo(a: &mut GridApp) {
        let (m0, m1) = PRESET_MENU_COMBO;
        a.event(m0, true);
        tap(a, m1);
        a.event(m0, false);
    }

    /// Render `frames` of audio; index of the first frame louder than `thr`.
    fn first_sound(a: &mut GridApp, frames: usize, thr: f32) -> Option<usize> {
        let mut buf = vec![0.0f32; frames * 2];
        for c in buf.chunks_mut(512) {
            a.audio(c);
        }
        buf.chunks(2).position(|f| f[0].abs() > thr || f[1].abs() > thr)
    }

    #[test]
    fn every_preset_lands_on_its_tracks_in_book_order() {
        let ps = presets::builtin();
        for (pi, x) in ps.patterns.iter().enumerate() {
            let mut a = beat();
            let plan = a.plan_of(pi);
            a.load_preset(pi);
            assert_eq!(a.lens[0], (x.steps, x.div), "{}", x.id);
            assert_eq!((a.tr.steps, a.tr.div), (x.steps, x.div), "{} clock", x.id);
            // 8 tracks: every sounding row has a track of its own, top to bottom as in the book
            let sounding: Vec<&str> = presets::ROWS.iter().copied().filter(|r| *r != "AC" && x.tracks.iter().any(|(k, _)| k == r)).collect();
            let rows: Vec<&str> = plan.voices.iter().filter(|v| !v.rows.is_empty()).map(|v| {
                assert_eq!(v.rows.len(), 1, "{}: nothing shares a track", x.id);
                v.rows[0]
            }).collect();
            assert_eq!(rows, sounding, "{}", x.id);
            let accent = x.tracks.iter().find(|(k, _)| k == "AC").map(|(_, s)| s.as_bytes());
            for t in 0..8 {
                let src = plan.voices[t].rows.first().and_then(|r| x.tracks.iter().find(|(k, _)| k == r)).map(|(_, s)| s.as_bytes());
                for s in 0..STEPS {
                    let c = a.cells[a.idx(0, t, s)];
                    let ch = src.and_then(|p| p.get(s).copied()).unwrap_or(b'.');
                    assert_eq!(c != 0, ch != b'.', "{} track {} step {}: cell {}", x.id, t + 1, s + 1, c);
                    if c != 0 {
                        assert_eq!(c & FLAM_BIT != 0, ch == b'F', "{} track {} step {} flam", x.id, t + 1, s + 1);
                        let acc = accent.is_some_and(|ac| ac[s] != b'.');
                        let vel = if acc { presets::ACCENT_VEL } else { presets::NORMAL_VEL };
                        assert_eq!(c & !FLAM_BIT, vel, "{} track {} step {} velocity", x.id, t + 1, s + 1);
                    }
                }
            }
            // the tracks hold the planned sounds (built-ins here: no samples)
            for (t, v) in plan.voices.iter().enumerate() {
                if let presets::Sound::Builtin(n) = v.sound {
                    assert_eq!(a.tracks[t].inst_key, format!("builtin:{}", n), "{} track {}", x.id, t + 1);
                }
            }
        }
    }

    #[test]
    fn a_preset_applies_its_kit_and_a_single_tom_sound_plays_three_pitches() {
        let wav = encode16(&(0..2000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect::<Vec<_>>(), 1, 44100);
        let mut host = MemHost::default();
        for f in ["BD 1", "SD 1", "CH 1", "OH 1", "Tom 1", "Crash 1", "Cowbell 1", "Rim 1"] {
            host.samples.insert(format!("Drums/Studio/{}.wav", f), wav.clone());
        }
        let mut a = crate::apps::beat::new(Box::new(host));
        assert_eq!(a.kits.len(), 1);
        let ps = presets::builtin();
        let pi = ps.patterns.iter().position(|x| ["HT", "MT", "LT"].iter().all(|t| x.tracks.iter().any(|(k, _)| k == t))).unwrap();
        a.load_preset(pi);
        let plan = a.plan_of(pi);
        assert_eq!(plan.kit.as_deref(), Some("Studio"));
        for (t, v) in plan.voices.iter().enumerate() {
            if let presets::Sound::File(f) = &v.sound {
                assert_eq!(&a.tracks[t].inst_key, f);
            }
        }
        // HT / MT / LT each have a track, all using "Tom 1", pitched +5 / 0 / -5
        for (row, semis) in presets::TOM_SEMIS {
            let t = plan.voices.iter().position(|v| v.rows == [row]).unwrap();
            assert_eq!(a.tracks[t].inst_key, "Drums/Studio/Tom 1.wav");
            let s = (0..STEPS).find(|&s| a.cells[a.idx(0, t, s)] != 0).unwrap();
            assert_eq!(a.pitch[a.idx(0, t, s)], semis, "{}", row);
        }
        // pitch survives save / load
        let text = a.state_string();
        let mut h = MemHost::default();
        h.state.insert("hillbeat".into(), text);
        let b = crate::apps::beat::new(Box::new(h));
        assert_eq!(b.pitch[..8 * STEPS], a.pitch[..8 * STEPS]);
    }

    #[test]
    fn the_combo_opens_and_closes_the_menu_and_b_backs_out() {
        let mut a = beat();
        combo(&mut a);
        assert!(matches!(a.overlay, Overlay::Presets));
        combo(&mut a);
        assert!(matches!(a.overlay, Overlay::None));
        // START was used as a modifier: releasing it did not start playback
        assert!(!a.tr.playing);
        combo(&mut a);
        tap(&mut a, Btn::A); // into the first style
        tap(&mut a, Btn::B); // back to the styles
        assert!(matches!(a.overlay, Overlay::Presets));
        tap(&mut a, Btn::B); // close
        assert!(matches!(a.overlay, Overlay::None));
        // the main screen works as before: A toggles a step
        tap(&mut a, Btn::A);
        assert!(a.cells[a.idx(0, 0, 0)] > 0);
    }

    #[test]
    fn load_a_12_step_pattern_then_reopen_at_the_same_place() {
        let mut a = beat();
        combo(&mut a);
        tap(&mut a, Btn::Down); // Blues (12/8)
        tap(&mut a, Btn::A);
        tap(&mut a, Btn::Down); // Blues 2
        tap(&mut a, Btn::A); // the pattern is empty: loads at once and closes
        assert!(matches!(a.overlay, Overlay::None));
        assert_eq!(a.lens[0], (12, 3));
        assert_eq!(presets::builtin().patterns[a.pm.highlighted().unwrap()].id, "blues-r2");
        // reopen: same pattern highlighted; loading over notes asks first
        combo(&mut a);
        tap(&mut a, Btn::Down); // Blues 3
        tap(&mut a, Btn::A);
        assert!(a.pm.confirm.is_some());
        tap(&mut a, Btn::B); // keep
        assert!(a.pm.confirm.is_none() && matches!(a.overlay, Overlay::Presets));
        tap(&mut a, Btn::A);
        tap(&mut a, Btn::A); // replace
        assert!(matches!(a.overlay, Overlay::None));
        let x = &presets::builtin().patterns[a.pm.highlighted().unwrap()];
        assert_eq!(x.id, "blues-r3");
        // the cursor moves within the 12 steps
        tap(&mut a, Btn::Left);
        assert_eq!(a.cur_step, 11);
        // clearing makes it a 16-step pattern again
        a.event(Btn::Select, true);
        tap(&mut a, Btn::L1);
        a.event(Btn::Select, false);
        assert_eq!((a.lens[0], a.tr.steps), ((16, 4), 16));
    }

    #[test]
    fn position_and_lengths_survive_save_and_load() {
        let mut a = beat();
        combo(&mut a);
        for _ in 0..20 {
            tap(&mut a, Btn::Down); // Waltz (3/4)
        }
        tap(&mut a, Btn::A);
        tap(&mut a, Btn::A);
        let text = a.state_string();
        let mut host = MemHost::default();
        host.state.insert("hillbeat".into(), text);
        let b = crate::apps::beat::new(Box::new(host));
        assert_eq!(b.lens[0], (12, 4));
        assert_eq!(b.tr.steps, 12);
        assert_eq!(b.cells[..STEPS * 8], a.cells[..STEPS * 8]);
        assert_eq!(presets::builtin().patterns[b.pm.highlighted().unwrap()].id, "waltz-r1");
    }

    #[test]
    fn loading_while_playing_waits_for_the_bar_line() {
        let mut a = beat();
        a.tr.playing = true;
        let _ = first_sound(&mut a, 6000, 2.0); // into step 2
        let funk = presets::builtin().patterns.iter().position(|x| x.id == "funk-r1").unwrap();
        a.load_preset(funk);
        assert!(a.cells[..STEPS * 8].iter().all(|&c| c == 0), "not yet");
        let _ = first_sound(&mut a, 88200, 2.0); // past the bar line
        assert!(a.cells[..STEPS * 8].iter().any(|&c| c != 0), "applied at step 1");
        assert!(a.staged.is_none());
    }

    #[test]
    fn a_flam_sounds_its_grace_note_30ms_early() {
        let step = 5512; // a 16th at 120 bpm
        let at = 4 * step;
        let lead = (FLAM_GRACE_MS / 1000.0 * SAMPLE_RATE) as usize;
        for (cell, expect) in [(presets::NORMAL_VEL, at), (presets::NORMAL_VEL | FLAM_BIT, at - lead)] {
            let mut a = beat();
            for t in 0..8 {
                a.tracks[t].mute = t != 1;
            }
            let i = a.idx(0, 1, 4);
            a.cells[i] = cell;
            a.tr.playing = true;
            let first = first_sound(&mut a, at + 4000, 1e-4).expect("sound");
            assert!((first as i64 - expect as i64).abs() <= 2, "cell {:#x}: first sound at {} expected {}", cell, first, expect);
        }
    }

    #[test]
    fn listening_in_the_menu_plays_without_touching_the_pattern() {
        let mut a = beat();
        combo(&mut a);
        tap(&mut a, Btn::A); // Afro-Cuban
        tap(&mut a, Btn::X);
        assert!(a.aud.is_some());
        assert!(first_sound(&mut a, 44100, 1e-3).is_some(), "the audition is audible");
        assert!(a.cells.iter().all(|&c| c == 0) && !a.tr.playing);
        tap(&mut a, Btn::Down); // follows the cursor
        assert_eq!(a.aud.as_ref().map(|x| x.preset), a.pm.highlighted());
        tap(&mut a, Btn::B); // leaving the list stops it
        assert!(a.aud.is_none());
        // while the sequencer plays, listening is refused
        a.overlay = Overlay::None;
        a.tr.playing = true;
        combo(&mut a);
        tap(&mut a, Btn::A);
        tap(&mut a, Btn::X);
        assert!(a.aud.is_none() && !a.pm.note.is_empty());
    }

    #[test]
    fn hillband_and_hillsequencer_ignore_the_combo() {
        for mut a in [crate::apps::band::new(Box::new(MemHost::default())), crate::apps::sequencer::new(Box::new(MemHost::default()))] {
            combo(&mut a);
            assert!(matches!(a.overlay, Overlay::None));
            assert_eq!(a.pattern, 0, "START+L1 does not change the pattern");
        }
    }

    #[test]
    fn after_changing_one_voice_it_opens_at_that_single_sound() {
        let mut a = app_with_two_kits();
        a.event(Btn::Start, true);
        tap(&mut a, Btn::B);
        a.event(Btn::Start, false);
        tap(&mut a, Btn::B);
        tap(&mut a, Btn::Down);
        tap(&mut a, Btn::A);
        tap(&mut a, Btn::A); // a kit
        // replace voice 1 with a built-in sound: no longer exactly a kit
        a.tracks[0].inst_key = "builtin:Kick".into();
        assert!(a.applied_kit().is_none());
        a.event(Btn::Start, true);
        tap(&mut a, Btn::B);
        a.event(Btn::Start, false);
        assert_eq!(a.browser.cwd(), "Built-in", "opens where the current sound lives");
    }
}
