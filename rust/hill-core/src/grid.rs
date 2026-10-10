//! Step-grid engine shared by HillBeat, HillSequencer and HillBand. The apps
//! differ only in their `GridConfig`: track layout and which extras are on.

use crate::fb::*;
use crate::host::Host;
use crate::input::{Btn, Held, Repeat, TapTempo};
use crate::instrument::Instrument;
use crate::kv::{from_hex, to_hex, Kv};
use crate::library::{self, Entry};
use crate::mixer::Mixer;
use crate::theory::*;
use crate::transport::Transport;
use crate::ui::{modal, quit_bar, Browser, BrowserOut, Help, Toast, QUIT_HOLD_MS};
use crate::{App, SAMPLE_RATE};

pub const STEPS: usize = 16;
pub const PATTERNS: usize = 8;

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
    pub help: &'static [(&'static str, &'static str)],
}

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
    cells: Vec<u8>,
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
            overlay: Overlay::None,
            browser: Browser::default(),
            help: Help::default(),
            toast: Toast::default(),
            rep: Repeat::default(),
            tap: TapTempo::default(),
            ms: 0,
            last_vel: 100,
        };
        app.defaults();
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
                    let n = ["Kick", "Snare", "Hat", "Clap"][drum_i % 4];
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
        let gain = vel as f32 / 127.0 * t.vol * 0.9;
        match t.kind {
            TrackKind::Drum => {
                if let Some(s) = t.inst.as_ref().and_then(|i| i.first()) {
                    mixer.trigger(s, 1.0, gain, ti as u32, 4);
                }
            }
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
            let v = self.cells[self.idx(self.pattern, ti, step)];
            if v > 0 && !self.tracks[ti].mute {
                if let Some(g) = self.gate_of(ti) {
                    self.mixer.release(ti as u32); // legato: the new note replaces the held one
                    self.tracks[ti].gate_left = g.steps;
                }
                Self::play_track(&mut self.mixer, &mut self.tracks[ti], ti, v);
            }
        }
    }

    // ---- state -----------------------------------------------------------

    fn state_string(&self) -> String {
        let mut k = Kv::new();
        k.set("bpm", self.tr.bpm);
        k.set("swing", self.tr.swing);
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
            }
        }
        k.render()
    }

    fn apply_state(&mut self, text: &str) {
        let k = Kv::parse(text);
        if let Some(v) = k.f32("bpm") {
            self.tr.set_bpm(v);
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
            t.vol = k.f32(&format!("t{}.vol", i)).unwrap_or(0.8).clamp(0.0, 1.0);
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
                        self.cells[a + s] = (*b).min(127);
                    }
                }
            }
        }
    }

    // ---- editing ---------------------------------------------------------

    fn toggle_step(&mut self) {
        let a = self.idx(self.pattern, self.cur_track, self.cur_step);
        if self.cells[a] > 0 {
            self.last_vel = self.cells[a];
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
        let cur = if self.cells[a] == 0 { self.last_vel as i32 } else { self.cells[a] as i32 };
        let v = (cur + d).clamp(1, 127) as u8;
        self.cells[a] = v;
        self.last_vel = v;
    }

    fn move_cursor(&mut self, b: Btn) {
        match b {
            Btn::Left => self.cur_step = (self.cur_step + STEPS - 1) % STEPS,
            Btn::Right => self.cur_step = (self.cur_step + 1) % STEPS,
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
        }
    }

    fn clear_pattern(&mut self) {
        let a = self.idx(self.pattern, 0, 0);
        let n = self.nt() * STEPS;
        self.cells[a..a + n].fill(0);
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

    fn open_browser(&mut self) {
        let mut targets: Vec<usize> = (0..self.nt()).filter(|&i| self.sel[i]).collect();
        if targets.is_empty() {
            targets.push(self.cur_track);
        }
        let first = targets[0];
        let drum = self.cfg.drum_filter && self.tracks[first].kind == TrackKind::Drum;
        let title = if targets.len() > 1 { format!("Sound for {} tracks", targets.len()) } else { format!("Sound for track {}", first + 1) };
        let cur = self.tracks[first].inst_key.clone();
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

    fn save_now(&mut self) {
        let s = self.state_string();
        self.host.save_state(self.cfg.file, &s);
    }

    // ---- input -----------------------------------------------------------

    fn press(&mut self, b: Btn) {
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
                        let t = &mut self.tracks[self.cur_track];
                        t.vol = (t.vol + ud as f32 * 0.05).clamp(0.0, 1.0);
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

        // step numbers + playhead
        for s in 0..STEPS {
            let x = x0 + s as i32 * 30 + (s as i32 / 4) * 4;
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
            let vb = (t.vol * 100.0) as i32;
            fb.rect(10, y + rh - 8, 100, 4, CELL_OFF);
            fb.rect(10, y + rh - 8, vb, 4, scale(base, if t.mute { 0.3 } else { 0.8 }));

            for s in 0..STEPS {
                let x = x0 + s as i32 * 30 + (s as i32 / 4) * 4;
                let v = self.cells[self.idx(self.pattern, ti, s)];
                let head = play == Some(s);
                let off = if (s / 4) % 2 == 0 { CELL_BEAT } else { CELL_OFF };
                fb.rect(x, y, cw, rh, if head { scale(off, 1.5) } else { off });
                if v > 0 {
                    let dimf = if t.mute { 0.35 } else { 1.0 };
                    fb.rect(x, y, cw, rh, scale(base, 0.25 * dimf));
                    let fh = (rh * v as i32 / 127).max(3);
                    fb.rect(x, y + rh - fh, cw, fh, scale(base, dimf * if head { 1.2 } else { 1.0 }));
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
        let v = self.cells[self.idx(self.pattern, self.cur_track, self.cur_step)];
        let l2 = format!("Step {}  {}   vol {}%", self.cur_step + 1, if v > 0 { format!("vel {}", v) } else { "off".into() }, (t.vol * 100.0).round());
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
        while done < frames {
            while let Some(s) = self.tr.take_due() {
                self.fire_step(s);
            }
            let chunk = (self.tr.until_due().min((frames - done) as u64) as usize).max(1);
            let seg = &mut out[done * 2..(done + chunk) * 2];
            self.mixer.render(seg);
            self.tr.advance(chunk as u64);
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
            Overlay::Chord { chord, scale, .. } => {
                let (x, y) = modal(fb, 440, 200, "Chord / scale");
                fb.text(x, y + 4, "Chord", 2, DIM);
                fb.text(x + 130, y + 4, &format!("< {} >", CHORD_TYPES[*chord].name), 2, WHITE);
                fb.text(x, y + 44, "Scale", 2, DIM);
                fb.text(x + 130, y + 44, &format!("^ {} v", SCALE_MODES[*scale].name), 2, WHITE);
                fb.text(x, y + 100, "L/R chord  U/D scale  A apply  B cancel", 1, DIM);
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
        self.mixer.decay = self.cfg.decay;
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
