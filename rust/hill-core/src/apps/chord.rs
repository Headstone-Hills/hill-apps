//! HillChord: a live chord / note instrument with an event-list looper,
//! arpeggiator and metronome.

use crate::fb::*;
use crate::host::Host;
use crate::input::{Btn, Held, Repeat};
use crate::instrument::Instrument;
use crate::kv::Kv;
use crate::library::{self, Entry};
use crate::mixer::Mixer;
use crate::synth;
use crate::theory::*;
use crate::ui::{quit_bar, Browser, BrowserOut, Help, Toast, QUIT_HOLD_MS};
use crate::{App, SAMPLE_RATE};
use std::sync::Arc;

const BLOCK: usize = 64;
const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [i32; 7] = [0, 2, 3, 5, 7, 8, 10];
const ROMAN_MAJ: [&str; 7] = ["I", "ii", "iii", "IV", "V", "vi", "vii*"];
const ROMAN_MIN: [&str; 7] = ["i", "ii*", "III", "iv", "v", "VI", "VII"];
const ARP_DIVS: [(f64, &str); 3] = [(1.0, "1/4"), (0.5, "1/8"), (0.25, "1/16")];
/// Voice slots: 0-3 are the face buttons (A B X Y), 4-7 the D-pad (Up Right Down Left,
/// used in note mode).
const FACE_KEYS: [u32; 8] = [100, 101, 102, 103, 104, 105, 106, 107];
const SLOTS: usize = 8;
const DPAD_DIRS: [Btn; 4] = [Btn::Up, Btn::Right, Btn::Down, Btn::Left];
/// Chord mode waits this long after a face button changes before voicing the chord.
const FACE_SETTLE_MS: u32 = 30;
const FACE_NAMES: [&str; 4] = ["A", "B", "X", "Y"];
const FACE_COLS: [u32; 4] = [rgb(120, 220, 160), rgb(255, 110, 110), rgb(90, 150, 255), rgb(255, 210, 80)];

const HELP: &[(&str, &str)] = &[
    ("PLAYING", ""),
    ("A B X Y", "chord mode: I IV V vi; pairs A+B ii, A+X iii, B+Y vii*"),
    ("R2", "chord mode / note mode"),
    ("SELECT+X", "chord mode / note mode (same)"),
    ("NOTE MODE", ""),
    ("D-pad", "notes 1-4: up, right, down, left"),
    ("A / B / X", "notes 5 / 6 / 7"),
    ("Y", "note 1, an octave up"),
    ("A+B / A+X", "octave down / up"),
    ("hold a button", "note sustains; release stops it"),
    ("D-pad (chord mode, chord held)", "L sus2, R sus4, D add9, U 7th"),
    ("D-pad Up (chord mode, alone)", "major / minor"),
    ("D-pad L/R (chord mode, alone)", "BPM - / + (hold repeats)"),
    ("L1 / R1", "previous / next instrument"),
    ("START", "instrument library (Y star, X favorites)"),
    ("FN+Left/Right", "octave down / up"),
    ("FN+Up/Down", "key root up / down"),
    ("LOOPER", ""),
    ("L2", "record > loop > overdub > loop"),
    ("L2 (hold 2s)", "clear loop"),
    ("SELECT+L2", "pause / resume loop (keeps it)"),
    ("ARP & CLICK", ""),
    ("SELECT+B", "arpeggiator on / off"),
    ("SELECT+Y", "arp rate 1/4 - 1/8 - 1/16"),
    ("SELECT+A", "metronome on / off"),
    ("OTHER", ""),
    ("START+Y", "save"),
    ("START+SELECT", "tap: panic + clear loop, hold 1.5s: quit"),
    ("FN+SELECT", "this help"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Chord,
    Note,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LState {
    Idle,
    Rec,
    Loop,
    Over,
}

struct Ev {
    beat: f64,
    dur: f64,
    note: u8,
    vel: f32,
}

struct Open {
    key: u32,
    start: f64, // absolute beat
    notes: Vec<u8>,
    vel: f32,
}

struct Looper {
    state: LState,
    t0: f64,
    len: f64,
    events: Vec<Ev>,
    open: Vec<Open>,
    /// Loop kept but not playing (SELECT+L2). Resumes from its start on the next beat.
    paused: bool,
}

impl Looper {
    fn new() -> Self {
        Looper { state: LState::Idle, t0: 0.0, len: 4.0, events: Vec::new(), open: Vec::new(), paused: false }
    }

    fn recording(&self) -> bool {
        matches!(self.state, LState::Rec | LState::Over)
    }

    fn clear(&mut self) {
        *self = Looper::new();
    }

    /// Store a finished note. `start`/`end` are absolute beats.
    fn commit(&mut self, notes: &[u8], vel: f32, start: f64, end: f64) {
        let (beat, dur) = match self.state {
            LState::Rec => ((start - self.t0).max(0.0), (end - start).max(0.02)),
            LState::Over | LState::Loop => {
                let b = (start - self.t0).rem_euclid(self.len);
                (b, (end - start).max(0.02).min(self.len - b))
            }
            LState::Idle => return,
        };
        for &n in notes {
            self.events.push(Ev { beat, dur, note: n, vel });
        }
    }

    fn close_all(&mut self, now: f64) {
        let open = std::mem::take(&mut self.open);
        for o in open {
            self.commit(&o.notes, o.vel, o.start, now);
        }
    }
}

enum Overlay {
    None,
    Help,
    Browse,
}

pub struct ChordApp {
    host: Box<dyn Host>,
    catalog: Vec<Entry>,
    favs: Vec<String>,
    rate: f32,
    mixer: Mixer,
    inst: Option<Instrument>,
    inst_key: String,
    click_hi: Arc<crate::wav::Sample>,
    click_lo: Arc<crate::wav::Sample>,

    root: u8,
    minor: bool,
    octave: i32,
    mode: Mode,

    bpm: f32,
    beat: f64,
    click_on: bool,
    arp_on: bool,
    arp_div: usize,
    arp_i: usize,

    face: [bool; 4],
    settle_ms: Option<u32>,
    dpad: [bool; 4],
    face_notes: [Vec<u8>; SLOTS],
    modifier: Option<Btn>,
    looper: Looper,
    pend: Vec<(f64, u32)>,
    next_key: u32,

    held: Held,
    start_plain: bool,
    ss: bool,
    ss_ms: u32,
    quit: bool,
    l2_ms: u32,
    l2_down: bool,
    l2_used: bool,
    rep: Repeat,
    overlay: Overlay,
    browser: Browser,
    help: Help,
    toast: Toast,
}

pub fn new(host: Box<dyn Host>) -> ChordApp {
    let catalog = library::build_catalog(&host.list_samples());
    let mut app = ChordApp {
        host,
        catalog,
        favs: Vec::new(),
        rate: SAMPLE_RATE,
        mixer: Mixer::new(SAMPLE_RATE),
        inst: synth::builtin_by_name("Pluck"),
        inst_key: "builtin:Pluck".into(),
        click_hi: synth::click(true),
        click_lo: synth::click(false),
        root: 0,
        minor: false,
        octave: 3,
        mode: Mode::Chord,
        bpm: 120.0,
        beat: 0.0,
        click_on: false,
        arp_on: false,
        arp_div: 1,
        arp_i: 0,
        face: [false; 4],
        settle_ms: None,
        dpad: [false; 4],
        face_notes: Default::default(),
        modifier: None,
        looper: Looper::new(),
        pend: Vec::new(),
        next_key: 1000,
        held: Held::default(),
        start_plain: false,
        ss: false,
        ss_ms: 0,
        quit: false,
        l2_ms: 0,
        l2_down: false,
        l2_used: false,
        rep: Repeat::default(),
        overlay: Overlay::None,
        browser: Browser::default(),
        help: Help::default(),
        toast: Toast::default(),
    };
    if let Some(text) = app.host.load_state("hillchord") {
        app.apply_state(&text);
    }
    app
}

impl ChordApp {
    // ---- music -----------------------------------------------------------

    fn root_midi(&self) -> i32 {
        12 * (self.octave + 1) + self.root as i32
    }

    fn scale(&self) -> &'static [i32; 7] {
        if self.minor {
            &MINOR
        } else {
            &MAJOR
        }
    }

    fn degree_note(&self, d: usize) -> i32 {
        self.root_midi() + self.scale()[d % 7] + 12 * (d / 7) as i32
    }

    /// Note mode (the original layout): D-pad Up/Right/Down/Left = scale notes 1-4,
    /// A/B/X = notes 5-7, Y = the tonic an octave up. Returns the 0-based degree.
    fn note_degree(slot: usize) -> usize {
        [4, 5, 6, 7, 0, 1, 2, 3][slot.min(7)]
    }

    /// Chord mode: the chord degree for a set of held face buttons (the original
    /// HillChord layout): A=I B=IV X=V Y=vi, and the pairs A+B=ii A+X=iii B+Y=vii*.
    fn combo_degree(set: [bool; 4]) -> Option<usize> {
        match set {
            [true, false, false, false] => Some(0),
            [false, true, false, false] => Some(3),
            [false, false, true, false] => Some(4),
            [false, false, false, true] => Some(5),
            [true, true, false, false] => Some(1),
            [true, false, true, false] => Some(2),
            [false, true, false, true] => Some(6),
            _ => None,
        }
    }

    /// Notes for a face button on its own, with an optional D-pad modifier applied.
    fn notes_for(&self, face: usize, modifier: Option<Btn>) -> Vec<u8> {
        if self.mode == Mode::Note {
            return vec![clamp_note(self.degree_note(Self::note_degree(face)))];
        }
        self.chord_notes(Self::combo_degree(Self::only(face)).unwrap_or(0), modifier)
    }

    /// Triad on scale degree `d`, with an optional D-pad modifier applied.
    fn chord_notes(&self, d: usize, modifier: Option<Btn>) -> Vec<u8> {
        let mut n = vec![self.degree_note(d), self.degree_note(d + 2), self.degree_note(d + 4)];
        match modifier {
            Some(Btn::Left) => n[1] = self.degree_note(d + 1),
            Some(Btn::Right) => n[1] = self.degree_note(d + 3),
            Some(Btn::Down) => n.push(self.degree_note(d + 8)),
            Some(Btn::Up) => n.push(self.degree_note(d + 6)),
            _ => {}
        }
        n.into_iter().map(clamp_note).collect()
    }

    fn mod_suffix(m: Option<Btn>) -> &'static str {
        match m {
            Some(Btn::Left) => "sus2",
            Some(Btn::Right) => "sus4",
            Some(Btn::Down) => "add9",
            Some(Btn::Up) => "7",
            _ => "",
        }
    }

    fn only(face: usize) -> [bool; 4] {
        let mut one = [false; 4];
        one[face.min(3)] = true;
        one
    }

    fn chord_label(&self, face: usize, modifier: Option<Btn>) -> String {
        if self.mode == Mode::Note {
            return note_name(self.notes_for(face, modifier)[0]);
        }
        self.degree_label(Self::combo_degree(Self::only(face)).unwrap_or(0), modifier)
    }

    fn degree_label(&self, d: usize, modifier: Option<Btn>) -> String {
        let notes = self.chord_notes(d, modifier);
        let root = notes[0];
        let iv: Vec<i32> = notes.iter().map(|n| *n as i32 - root as i32).collect();
        chord_name(root % 12, &iv, Self::mod_suffix(modifier))
    }

    // ---- voices ----------------------------------------------------------

    fn sound(&mut self, notes: &[u8], vel: f32, key: u32) {
        let g = vel * 0.85 / (notes.len() as f32).sqrt();
        if let Some(inst) = self.inst.as_ref() {
            for &n in notes {
                if let Some((s, r)) = inst.pick(n) {
                    self.mixer.trigger(s, r, g, key, 0);
                }
            }
        }
    }

    /// What each face slot should be sounding right now. Chord mode: the whole held
    /// set is one chord, voiced under its first button. Note mode: one note per button.
    fn desired(&self) -> [Vec<u8>; SLOTS] {
        let mut want: [Vec<u8>; SLOTS] = Default::default();
        if self.mode == Mode::Note {
            for slot in 0..SLOTS {
                let held = if slot < 4 { self.face[slot] } else { self.dpad[slot - 4] };
                if held {
                    want[slot] = vec![clamp_note(self.degree_note(Self::note_degree(slot)))];
                }
            }
        } else if let (Some(d), Some(slot)) = (Self::combo_degree(self.face), self.face.iter().position(|x| *x)) {
            want[slot] = self.chord_notes(d, self.modifier);
        }
        want
    }

    /// Make the voices match `desired()`: re-voices on a changed chord, modifier or release.
    fn revoice(&mut self) {
        let want = self.desired();
        for f in 0..SLOTS {
            if want[f] == self.face_notes[f] {
                continue;
            }
            self.mixer.release(FACE_KEYS[f]);
            let now = self.beat;
            if let Some(p) = self.looper.open.iter().position(|o| o.key == FACE_KEYS[f]) {
                let o = self.looper.open.remove(p);
                self.looper.commit(&o.notes, o.vel, o.start, now);
            }
            self.face_notes[f] = want[f].clone();
            if !want[f].is_empty() && !self.arp_on {
                let notes = want[f].clone();
                self.sound(&notes, 1.0, FACE_KEYS[f]);
                if self.looper.recording() {
                    self.looper.open.push(Open { key: FACE_KEYS[f], start: now, notes, vel: 1.0 });
                }
            }
        }
    }

    /// A face button went down or up. In chord mode, wait a moment (`FACE_SETTLE_MS`)
    /// so two buttons pressed or released nearly together act as one chord.
    fn faces_changed(&mut self) {
        if self.mode == Mode::Note || !self.face.iter().any(|x| *x) {
            self.settle_ms = None;
            self.revoice();
        } else {
            self.settle_ms = Some(FACE_SETTLE_MS);
        }
    }

    /// Note mode: A+B shifts the octave down, A+X up (nothing sounds for the second
    /// button). Returns true when the press was an octave shift.
    fn note_octave_combo(&mut self, b: Btn) -> bool {
        let h = |x: Btn| self.held.is(x);
        let delta = match b {
            Btn::B if h(Btn::A) => -1,
            Btn::A if h(Btn::B) => -1,
            Btn::X if h(Btn::A) => 1,
            Btn::A if h(Btn::X) => 1,
            _ => return false,
        };
        self.octave = (self.octave + delta).clamp(1, 6);
        self.toast.say(&format!("Octave {}", self.octave));
        true
    }

    /// Switch between chord and note mode, re-voicing whatever is held.
    fn set_mode(&mut self, m: Mode) {
        self.mode = m;
        self.toast.say(if m == Mode::Chord { "Chord mode" } else { "Note mode" });
        self.faces_changed();
    }

    fn face_on(&mut self, f: usize) {
        self.face[f] = true;
        self.faces_changed();
    }

    fn face_off(&mut self, f: usize) {
        self.face[f] = false;
        self.faces_changed();
    }

    fn panic(&mut self) {
        self.mixer.stop_all();
        self.face = [false; 4];
        self.dpad = [false; 4];
        self.settle_ms = None;
        for n in self.face_notes.iter_mut() {
            n.clear();
        }
        self.pend.clear();
        self.looper.open.clear();
    }

    // ---- timeline --------------------------------------------------------

    fn block(&mut self, out: &mut [f32]) {
        let n = out.len() / 2;
        let beats = n as f64 * self.bpm as f64 / 60.0 / self.rate as f64;
        let prev = self.beat;
        let cur = prev + beats;

        if self.click_on && cur.floor() > prev.floor() {
            let k = cur.floor() as i64;
            let s = if k % 4 == 0 { self.click_hi.clone() } else { self.click_lo.clone() };
            self.mixer.trigger(&s, 1.0, 0.6, 7, 2);
        }

        if self.arp_on {
            let div = ARP_DIVS[self.arp_div].0;
            if (cur / div).floor() > (prev / div).floor() {
                let mut notes: Vec<u8> = self.face_notes.iter().flatten().copied().collect();
                notes.sort_unstable();
                notes.dedup();
                if !notes.is_empty() {
                    let note = notes[self.arp_i % notes.len()];
                    self.arp_i += 1;
                    let at = (cur / div).floor() * div;
                    let dur = div * 0.9;
                    let key = self.fresh_key();
                    self.sound(&[note], 1.0, key);
                    self.pend.push((at + dur, key));
                    if self.looper.recording() {
                        let l = &mut self.looper;
                        l.commit(&[note], 1.0, at, at + dur);
                    }
                } else {
                    self.arp_i = 0;
                }
            }
        }

        if matches!(self.looper.state, LState::Loop | LState::Over) && !self.looper.paused {
            let (t0, len) = (self.looper.t0, self.looper.len);
            let c0 = ((prev - t0) / len).floor() as i64;
            let c1 = ((cur - t0) / len).floor() as i64;
            let mut fire: Vec<(u8, f32, f64, f64)> = Vec::new();
            for c in c0.max(0)..=c1 {
                for e in &self.looper.events {
                    let abs = t0 + c as f64 * len + e.beat;
                    if abs >= prev && abs < cur {
                        fire.push((e.note, e.vel, abs, e.dur));
                    }
                }
            }
            for (note, vel, abs, dur) in fire {
                let key = self.fresh_key();
                self.sound(&[note], vel * 0.85, key);
                self.pend.push((abs + dur, key));
            }
        }

        let mut i = 0;
        while i < self.pend.len() {
            if self.pend[i].0 < cur {
                let (_, key) = self.pend.swap_remove(i);
                self.mixer.release(key);
            } else {
                i += 1;
            }
        }

        self.mixer.render(out);
        self.beat = cur;
    }

    fn fresh_key(&mut self) -> u32 {
        self.next_key += 1;
        if self.next_key > 1_000_000 {
            self.next_key = 1000;
        }
        self.next_key
    }

    /// SELECT+L2: stop the loop playing without clearing it, or start it again from
    /// its first beat on the next whole beat.
    fn toggle_loop_pause(&mut self) {
        let now = self.beat;
        match self.looper.state {
            LState::Idle => self.toast.say("No loop to pause"),
            LState::Rec => self.toast.say("Finish recording first"),
            LState::Loop | LState::Over => {
                if self.looper.paused {
                    self.looper.t0 = now.ceil();
                    self.looper.paused = false;
                    self.toast.say("Loop resumed");
                } else {
                    self.looper.close_all(now);
                    self.looper.state = LState::Loop;
                    self.looper.paused = true;
                    self.toast.say("Loop paused");
                }
            }
        }
    }

    fn l2_advance(&mut self) {
        if self.looper.paused {
            self.toggle_loop_pause(); // a plain L2 tap brings a paused loop back
            return;
        }
        let now = self.beat;
        match self.looper.state {
            LState::Idle => {
                self.looper.events.clear();
                self.looper.open.clear();
                self.looper.t0 = now.round();
                self.looper.state = LState::Rec;
                self.toast.say("Recording");
            }
            LState::Rec => {
                let len = (now - self.looper.t0).round().max(1.0);
                self.looper.close_all(now);
                // drop anything that landed past the loop end
                self.looper.len = len;
                let l = len;
                self.looper.events.retain(|e| e.beat < l);
                for e in self.looper.events.iter_mut() {
                    e.dur = e.dur.min(l - e.beat);
                }
                self.looper.state = LState::Loop;
                self.toast.say(&format!("Loop {} beats", len as i64));
            }
            LState::Loop => {
                self.looper.state = LState::Over;
                self.toast.say("Overdub");
            }
            LState::Over => {
                self.looper.close_all(now);
                self.looper.state = LState::Loop;
                self.toast.say("Loop");
            }
        }
    }

    // ---- instruments -----------------------------------------------------

    fn load_entry(&mut self, e: &Entry) -> bool {
        match library::load_entry(self.host.as_ref(), e, library::BUDGET_CHORD) {
            Some(i) => {
                self.inst = Some(i);
                self.inst_key = e.key.clone();
                true
            }
            None => false,
        }
    }

    fn step_instrument(&mut self, d: isize) {
        if self.catalog.is_empty() {
            return;
        }
        let n = self.catalog.len() as isize;
        let cur = self.catalog.iter().position(|e| e.key == self.inst_key).unwrap_or(0) as isize;
        for k in 1..=n {
            let i = ((cur + d * k).rem_euclid(n)) as usize;
            let e = self.catalog[i].clone();
            if self.load_entry(&e) {
                self.toast.say(&clip(&e.label, 24));
                return;
            }
        }
        self.toast.say("No playable sound");
    }

    // ---- state -----------------------------------------------------------

    fn state_string(&self) -> String {
        let mut k = Kv::new();
        k.set("bpm", self.bpm);
        k.set("root", self.root);
        k.set_bool("minor", self.minor);
        k.set("octave", self.octave);
        k.set("mode", if self.mode == Mode::Note { 1 } else { 0 });
        k.set_bool("click", self.click_on);
        k.set_bool("arp", self.arp_on);
        k.set("arp_div", self.arp_div);
        k.set("inst", &self.inst_key);
        k.set("favs", self.favs.join("|"));
        k.render()
    }

    fn apply_state(&mut self, text: &str) {
        let k = Kv::parse(text);
        self.bpm = k.f32("bpm").unwrap_or(120.0).clamp(40.0, 240.0);
        self.root = k.usize("root").unwrap_or(0).min(11) as u8;
        self.minor = k.bool("minor").unwrap_or(false);
        self.octave = k.usize("octave").unwrap_or(3).clamp(1, 6) as i32;
        self.mode = if k.usize("mode") == Some(1) { Mode::Note } else { Mode::Chord };
        self.click_on = k.bool("click").unwrap_or(false);
        self.arp_on = k.bool("arp").unwrap_or(false);
        self.arp_div = k.usize("arp_div").unwrap_or(1).min(ARP_DIVS.len() - 1);
        self.favs = k.get("favs").map(|s| s.split('|').filter(|x| !x.is_empty()).map(String::from).collect()).unwrap_or_default();
        if let Some(key) = k.get("inst") {
            if let Some(e) = library::find_key(&self.catalog, key).cloned() {
                self.load_entry(&e);
            }
        }
    }

    fn save_now(&mut self) {
        let s = self.state_string();
        self.host.save_state("hillchord", &s);
    }

    // ---- input -----------------------------------------------------------

    fn face_index(b: Btn) -> Option<usize> {
        match b {
            Btn::A => Some(0),
            Btn::B => Some(1),
            Btn::X => Some(2),
            Btn::Y => Some(3),
            _ => None,
        }
    }

    fn press(&mut self, b: Btn) {
        let st = self.held.is(Btn::Start);
        let se = self.held.is(Btn::Select);
        let fnh = self.held.is(Btn::Fn);
        match b {
            Btn::Start => {
                if se {
                    self.ss = true;
                    self.ss_ms = 0;
                } else {
                    self.start_plain = true;
                }
            }
            Btn::Select => {
                if fnh {
                    self.help.show();
                    self.overlay = Overlay::Help;
                } else if st {
                    self.ss = true;
                    self.ss_ms = 0;
                    self.start_plain = false;
                }
            }
            Btn::Fn => {
                if se {
                    self.help.show();
                    self.overlay = Overlay::Help;
                }
            }
            Btn::A | Btn::B | Btn::X | Btn::Y => {
                let f = Self::face_index(b).unwrap();
                if st && b == Btn::Y {
                    self.start_plain = false;
                    self.save_now();
                    self.toast.say("Saved");
                } else if se {
                    match b {
                        Btn::A => {
                            self.click_on = !self.click_on;
                            self.toast.say(if self.click_on { "Metronome on" } else { "Metronome off" });
                        }
                        Btn::B => {
                            self.arp_on = !self.arp_on;
                            self.arp_i = 0;
                            self.toast.say(if self.arp_on { "Arp on" } else { "Arp off" });
                        }
                        Btn::X => self.set_mode(if self.mode == Mode::Chord { Mode::Note } else { Mode::Chord }),
                        _ => {
                            self.arp_div = (self.arp_div + 1) % ARP_DIVS.len();
                            self.toast.say(&format!("Arp {}", ARP_DIVS[self.arp_div].1));
                        }
                    }
                } else if !st && !fnh {
                    if self.mode == Mode::Note && self.note_octave_combo(b) {
                        return;
                    }
                    self.face_on(f);
                }
            }
            Btn::L1 | Btn::R1 => {
                if !st && !se {
                    self.step_instrument(if b == Btn::L1 { -1 } else { 1 });
                }
            }
            Btn::R2 => {
                if !st && !se {
                    self.set_mode(if self.mode == Mode::Chord { Mode::Note } else { Mode::Chord });
                }
            }
            Btn::L2 => {
                if !st && !se {
                    self.l2_down = true;
                    self.l2_used = false;
                    self.l2_ms = 0;
                } else if se && !st {
                    self.toggle_loop_pause();
                }
            }
            Btn::Up | Btn::Down | Btn::Left | Btn::Right => {
                if fnh {
                    match b {
                        Btn::Left => self.octave = (self.octave - 1).max(1),
                        Btn::Right => self.octave = (self.octave + 1).min(6),
                        Btn::Up => self.root = (self.root + 1) % 12,
                        _ => self.root = (self.root + 11) % 12,
                    }
                    self.toast.say(&format!("{} {} oct {}", PC_NAMES[self.root as usize], if self.minor { "minor" } else { "major" }, self.octave));
                } else if self.mode == Mode::Note {
                    if !(se && matches!(b, Btn::Left | Btn::Right)) {
                        if let Some(i) = DPAD_DIRS.iter().position(|d| *d == b) {
                            self.dpad[i] = true;
                            self.faces_changed();
                        }
                    }
                } else if self.face.iter().any(|f| *f) && self.mode == Mode::Chord {
                    self.modifier = Some(b);
                    self.revoice();
                } else {
                    match b {
                        Btn::Up => {
                            self.minor = !self.minor;
                            self.toast.say(if self.minor { "Minor" } else { "Major" });
                        }
                        Btn::Left => {
                            self.bpm = (self.bpm - 1.0).max(40.0);
                            self.rep.start(b);
                        }
                        Btn::Right => {
                            self.bpm = (self.bpm + 1.0).min(240.0);
                            self.rep.start(b);
                        }
                        _ => {}
                    }
                }
            }
            Btn::L3 | Btn::R3 => {}
        }
    }

    fn release(&mut self, b: Btn) {
        if let Some(i) = DPAD_DIRS.iter().position(|d| *d == b) {
            if self.dpad[i] {
                self.dpad[i] = false;
                self.faces_changed();
            }
        }
        if let Some(f) = Self::face_index(b) {
            if self.face[f] {
                self.face_off(f);
            }
            return;
        }
        match b {
            Btn::Start => {
                if self.ss {
                    self.finish_ss();
                } else if self.start_plain {
                    let cur = self.inst_key.clone();
                    self.browser.open("Instruments", &self.catalog, false, true, &self.favs, Some(&cur));
                    self.overlay = Overlay::Browse;
                }
                self.start_plain = false;
            }
            Btn::Select => {
                if self.ss {
                    self.finish_ss();
                }
            }
            Btn::L2 => {
                if self.l2_down && !self.l2_used {
                    self.l2_advance();
                }
                self.l2_down = false;
            }
            Btn::Left | Btn::Right => self.rep.stop(b),
            Btn::Up | Btn::Down => {}
            _ => {}
        }
        if matches!(b, Btn::Up | Btn::Down | Btn::Left | Btn::Right) && self.modifier == Some(b) {
            // fall back to another held direction, if any
            self.modifier = [Btn::Up, Btn::Down, Btn::Left, Btn::Right].into_iter().find(|d| *d != b && self.held.is(*d));
            if self.face.iter().any(|f| *f) {
                self.revoice();
            }
        }
    }

    fn finish_ss(&mut self) {
        self.ss = false;
        if !self.quit {
            self.panic();
            self.looper.clear();
            self.toast.say("Stopped");
        }
    }

    fn overlay_event(&mut self, b: Btn) {
        match self.overlay {
            Overlay::None => {}
            Overlay::Help => {
                self.help.event(b, HELP.len());
                if !self.help.open {
                    self.overlay = Overlay::None;
                }
            }
            Overlay::Browse => match self.browser.event(b, &self.catalog, &self.favs) {
                BrowserOut::Pick(i) => {
                    self.overlay = Overlay::None;
                    let e = self.catalog[i].clone();
                    if self.load_entry(&e) {
                        self.toast.say(&clip(&e.label, 24));
                        self.save_now();
                    } else {
                        self.toast.say("Could not load sound");
                    }
                }
                BrowserOut::Close => self.overlay = Overlay::None,
                BrowserOut::Fav(i) => {
                    let key = self.catalog[i].key.clone();
                    if let Some(p) = self.favs.iter().position(|k| *k == key) {
                        self.favs.remove(p);
                    } else {
                        self.favs.push(key);
                    }
                    self.browser.refresh(&self.catalog, &self.favs);
                }
                BrowserOut::None => {}
            },
        }
    }

    // ---- drawing ---------------------------------------------------------

    fn draw_loop(&self, fb: &mut Fb, y: i32) {
        let l = &self.looper;
        let (label, col) = match l.state {
            LState::Idle => ("LOOP: idle", DIM),
            LState::Rec => ("RECORDING", WARN),
            LState::Loop if l.paused => ("LOOP PAUSED", WARN),
            LState::Loop => ("LOOPING", ACCENT),
            LState::Over => ("OVERDUB", DRUM),
        };
        fb.text(20, y, label, 2, col);
        let x0 = 20;
        let w = 600;
        let by = y + 30;
        fb.rect(x0, by, w, 28, PANEL);
        match l.state {
            LState::Idle => {}
            LState::Rec => {
                let beats = (self.beat - l.t0).max(0.0);
                let shown = (beats.max(4.0)).ceil();
                let f = (beats / shown) as f32;
                fb.rect(x0, by, (w as f32 * f.min(1.0)) as i32, 28, scale(WARN, 0.6));
                fb.text_right(x0 + w - 6, y, &format!("{:.1} beats", beats), 2, DIM);
            }
            _ => {
                for e in &l.events {
                    let x = x0 + (w as f64 * e.beat / l.len) as i32;
                    let ew = ((w as f64 * e.dur / l.len) as i32).clamp(2, 40);
                    fb.rect(x, by + 4, ew, 20, scale(MELODIC, 0.7));
                }
                for k in 0..l.len as i32 {
                    let x = x0 + (w as f64 * k as f64 / l.len) as i32;
                    fb.rect(x, by, 1, 28, DIM);
                }
                let pos = (self.beat - l.t0).rem_euclid(l.len);
                let px = x0 + (w as f64 * pos / l.len) as i32;
                fb.rect(px - 1, by - 3, 3, 34, WHITE);
                fb.text_right(x0 + w - 6, y, &format!("{} beats  {} notes", l.len as i64, l.events.len()), 2, DIM);
            }
        }
    }
}

impl App for ChordApp {
    fn name(&self) -> &'static str {
        "HillChord"
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
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(BLOCK);
            self.block(&mut out[done * 2..(done + n) * 2]);
            done += n;
        }
    }

    fn render(&self, fb: &mut Fb) {
        fb.clear(BG);
        fb.rect(0, 0, W, 30, PANEL);
        fb.text(8, 7, "HillChord", 2, ACCENT);
        fb.text(170, 7, &format!("{} {}", PC_NAMES[self.root as usize], if self.minor { "minor" } else { "major" }), 2, TEXT);
        fb.text(360, 7, &format!("{:.0}bpm", self.bpm), 2, TEXT);
        let mut x = 470;
        if self.click_on {
            fb.text(x, 7, "MET", 2, MELODIC);
            x += 64;
        }
        if self.arp_on {
            fb.text(x, 7, &format!("ARP {}", ARP_DIVS[self.arp_div].1), 2, DRUM);
        }
        // beat lamp
        let on = self.beat.fract() < 0.18;
        fb.rect(W - 26, 7, 16, 16, if on { if (self.beat as i64) % 4 == 0 { ACCENT } else { TEXT } } else { CELL_OFF });

        let inst = self.inst.as_ref().map(|i| i.name.as_str()).unwrap_or("-");
        let star = if self.favs.contains(&self.inst_key) { "* " } else { "" };
        fb.text(20, 44, &format!("{}{}", star, clip(inst, 24)), 2, WHITE);
        fb.text_right(W - 20, 48, &format!("{} mode   oct {}", if self.mode == Mode::Chord { "CHORD" } else { "NOTE" }, self.octave), 1, DIM);

        // four chord tiles
        let roman = if self.minor { &ROMAN_MIN } else { &ROMAN_MAJ };
        for f in 0..4 {
            let x = 20 + f as i32 * 152;
            let held = self.face[f];
            let col = FACE_COLS[f];
            fb.rect(x, 80, 140, 110, if held { scale(col, 0.55) } else { PANEL });
            fb.frame(x, 80, 140, 110, if held { 3 } else { 2 }, if held { col } else { CELL_BEAT });
            fb.text(x + 8, 88, FACE_NAMES[f], 3, col);
            // a held pair shows the chord it makes (A+B = ii), not each button's own chord
            let m = if held { self.modifier } else { None };
            let pair = if held && self.mode == Mode::Chord && self.face.iter().filter(|x| **x).count() >= 2 { Self::combo_degree(self.face) } else { None };
            let (d, label) = match pair {
                Some(d) => (d, self.degree_label(d, m)),
                None => (if self.mode == Mode::Note { Self::note_degree(f) } else { Self::combo_degree(Self::only(f)).unwrap_or(0) }, self.chord_label(f, m)),
            };
            let r = if self.mode == Mode::Note {
                format!("{}{}", d % 7 + 1, if d >= 7 { "'" } else { "" }) // note number, ' = octave up
            } else if d < 7 {
                roman[d].to_string()
            } else {
                format!("{}'", roman[d % 7])
            };
            fb.text_right(x + 132, 92, &r, 2, DIM);
            fb.text_center(x + 70, 136, &label, 2, if held { WHITE } else { TEXT });
        }

        // what is sounding
        let sounding: Vec<u8> = self.face_notes.iter().flatten().copied().collect();
        if !sounding.is_empty() {
            let mut s = sounding.clone();
            s.sort_unstable();
            s.dedup();
            let names: Vec<String> = s.iter().map(|n| note_name(*n)).collect();
            fb.text_center(320, 206, &names.join(" "), 2, ACCENT);
        } else if self.mode == Mode::Note {
            let n = |slot: usize| note_name(clamp_note(self.degree_note(Self::note_degree(slot))));
            fb.text_center(320, 200, &format!("D-pad  up {}  right {}  down {}  left {}", n(4), n(5), n(6), n(7)), 1, DIM);
            fb.text_center(320, 218, "A+B octave down   A+X octave up   FN+SEL help", 1, DIM);
        } else {
            fb.text_center(320, 206, "A B X Y   A+B ii   A+X iii   B+Y vii   FN+SEL help", 1, DIM);
        }

        self.draw_loop(fb, 250);
        if let Some(m) = self.modifier.filter(|_| self.face.iter().any(|f| *f)) {
            fb.text(20, 340, &format!("mod: {}", Self::mod_suffix(Some(m))), 2, DRUM);
        }
        fb.text(20, 440, "L2 loop  START library  L1/R1 sound  SEL+B arp  SEL+A click", 1, DIM);
        self.toast.draw(fb);
        if self.ss && self.held.is(Btn::Start) && self.held.is(Btn::Select) {
            quit_bar(fb, self.ss_ms);
        }
        match self.overlay {
            Overlay::None => {}
            Overlay::Help => self.help.draw(fb, "HillChord", HELP),
            Overlay::Browse => self.browser.draw(fb, &self.catalog, &self.favs),
        }
    }

    fn tick(&mut self, dt: u32) {
        self.toast.tick(dt);
        self.mixer.max_secs = Some(4.0 * 4.0 * 60.0 / self.bpm.max(1.0)); // 4 measures
        self.mixer.beat_secs = Some(60.0 / self.bpm.max(1.0));
        if let Some(left) = self.settle_ms {
            if left <= dt {
                self.settle_ms = None;
                self.revoice();
            } else {
                self.settle_ms = Some(left - dt);
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
        if self.l2_down && !self.l2_used && self.held.is(Btn::L2) {
            self.l2_ms += dt;
            if self.l2_ms >= 2000 {
                self.l2_used = true;
                self.looper.clear();
                self.pend.clear();
                self.toast.say("Loop cleared");
            }
        }
        if matches!(self.overlay, Overlay::None) {
            if let Some(b) = self.rep.tick(dt) {
                if b == Btn::Left {
                    self.bpm = (self.bpm - 1.0).max(40.0);
                } else {
                    self.bpm = (self.bpm + 1.0).min(240.0);
                }
            }
        } else {
            self.rep.clear();
        }
    }

    fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
        self.mixer.set_rate(rate);
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
    use crate::MemHost;

    fn app() -> ChordApp {
        new(Box::new(MemHost::default()))
    }

    #[test]
    fn chord_degrees_major_and_minor() {
        let mut a = app();
        // C major, octave 3 -> root midi 48. Single buttons: A=I, B=IV, X=V, Y=vi
        assert_eq!(a.notes_for(0, None), vec![48, 52, 55]); // I = C E G
        assert_eq!(a.notes_for(1, None), vec![53, 57, 60]); // IV = F A C
        assert_eq!(a.notes_for(2, None), vec![55, 59, 62]); // V = G B D
        assert_eq!(a.notes_for(3, None), vec![57, 60, 64]); // vi = A C E
        // pairs: A+B = ii, A+X = iii, B+Y = vii*
        let notes = |a: &ChordApp, set: [bool; 4]| a.chord_notes(ChordApp::combo_degree(set).unwrap(), None);
        assert_eq!(notes(&a, [true, true, false, false]), vec![50, 53, 57]); // ii = D F A
        assert_eq!(notes(&a, [true, false, true, false]), vec![52, 55, 59]); // iii = E G B
        assert_eq!(notes(&a, [false, true, false, true]), vec![59, 62, 65]); // vii* = B D F
        // other combinations play nothing
        assert!(ChordApp::combo_degree([false, false, true, true]).is_none());
        assert!(ChordApp::combo_degree([true, true, true, false]).is_none());
        a.minor = true;
        assert_eq!(a.notes_for(0, None), vec![48, 51, 55]); // i = C Eb G
    }

    #[test]
    fn face_pairs_play_one_chord_without_a_blip() {
        let mut a = app();
        let (i, ii, iv) = (vec![48, 52, 55], vec![50, 53, 57], vec![53, 57, 60]);
        // A then B a few ms later: nothing sounds until the settle window ends, then only ii
        a.face_on(0);
        a.tick(5);
        a.face_on(1);
        assert!(a.face_notes.iter().all(|n| n.is_empty()), "nothing sounded yet");
        a.tick(40);
        assert_eq!(a.face_notes[0], ii, "A+B is ii, voiced under the first button");
        assert!(a.face_notes[1].is_empty());
        assert_ne!(a.face_notes[0], i);
        // release one then the other almost together: no single-chord blip
        a.face_off(0);
        a.tick(5);
        a.face_off(1);
        assert!(a.face_notes.iter().all(|n| n.is_empty()), "all released: silent at once");
        // release only A and keep holding B: after the settle window B plays its own chord
        a.face_on(0);
        a.face_on(1);
        a.tick(40);
        a.face_off(0);
        a.tick(40);
        assert_eq!(a.face_notes[1], iv);
        assert!(a.face_notes[0].is_empty());
        // a single button sounds after the same short wait
        let mut b = app();
        b.face_on(0);
        b.tick(40);
        assert_eq!(b.face_notes[0], i);
        // an unassigned pair (X+Y) is silent
        let mut c = app();
        c.face_on(2);
        c.face_on(3);
        c.tick(40);
        assert!(c.face_notes.iter().all(|n| n.is_empty()));
    }

    #[test]
    fn note_mode_has_the_original_layout() {
        let mut a = app();
        assert!(a.mode == Mode::Chord);
        a.event(Btn::R2, true);
        a.event(Btn::R2, false);
        assert!(a.mode == Mode::Note, "R2 -> note mode");
        // C major, octave 3: tonic 48. D-pad up/right/down/left = notes 1-4 = C D E F
        let sound = |a: &mut ChordApp, b: Btn, slot: usize| {
            a.event(b, true);
            let n = a.face_notes[slot].clone();
            a.event(b, false);
            assert!(a.face_notes[slot].is_empty(), "release stops the note");
            n
        };
        assert_eq!(sound(&mut a, Btn::Up, 4), vec![48]);
        assert_eq!(sound(&mut a, Btn::Right, 5), vec![50]);
        assert_eq!(sound(&mut a, Btn::Down, 6), vec![52]);
        assert_eq!(sound(&mut a, Btn::Left, 7), vec![53]);
        // A/B/X = notes 5/6/7 = G A B, Y = the tonic an octave up
        assert_eq!(sound(&mut a, Btn::A, 0), vec![55]);
        assert_eq!(sound(&mut a, Btn::B, 1), vec![57]);
        assert_eq!(sound(&mut a, Btn::X, 2), vec![59]);
        assert_eq!(sound(&mut a, Btn::Y, 3), vec![60]);
        // notes can be held together (a drone plus a melody)
        a.event(Btn::Up, true);
        a.event(Btn::A, true);
        assert_eq!(a.face_notes[4], vec![48]);
        assert_eq!(a.face_notes[0], vec![55]);
        a.event(Btn::A, false);
        a.event(Btn::Up, false);
        // octave: A+B down, A+X up; the second button plays nothing
        a.event(Btn::A, true);
        a.event(Btn::B, true);
        assert_eq!(a.octave, 2);
        assert!(a.face_notes[1].is_empty(), "B made no note");
        a.event(Btn::B, false);
        a.event(Btn::X, true);
        assert_eq!(a.octave, 3, "A+X is octave up");
        a.event(Btn::X, false);
        a.event(Btn::A, false);
        // R2 again: back to chord mode, where the D-pad is not a note
        a.event(Btn::R2, true);
        a.event(Btn::R2, false);
        assert!(a.mode == Mode::Chord);
        a.event(Btn::Up, true);
        assert!(a.face_notes[4].is_empty());
    }

    #[test]
    fn modifiers() {
        let a = app();
        assert_eq!(a.notes_for(0, Some(Btn::Left)), vec![48, 50, 55]); // sus2
        assert_eq!(a.notes_for(0, Some(Btn::Right)), vec![48, 53, 55]); // sus4
        assert_eq!(a.notes_for(0, Some(Btn::Up)), vec![48, 52, 55, 59]); // maj7
        assert_eq!(a.notes_for(0, Some(Btn::Down)).len(), 4);
        // V (face X) gets a dominant 7th (F) from the scale
        assert_eq!(a.notes_for(2, Some(Btn::Up)), vec![55, 59, 62, 65]);
    }

    #[test]
    fn labels() {
        let mut a = app();
        assert_eq!(a.chord_label(0, None), "C");
        assert_eq!(a.chord_label(1, None), "F");
        assert_eq!(a.chord_label(3, None), "Am");
        assert_eq!(a.chord_label(0, Some(Btn::Up)), "C7");
        a.mode = Mode::Note;
        assert_eq!(a.chord_label(0, None), "G3"); // note mode: A is note 5
    }

    #[test]
    fn looper_commit_handles_wrap_and_clip() {
        let mut l = Looper::new();
        l.state = LState::Over;
        l.t0 = 0.0;
        l.len = 4.0;
        l.commit(&[60], 1.0, 7.5, 9.0); // starts at beat 3.5 of the loop, clipped to 0.5
        assert_eq!(l.events.len(), 1);
        assert!((l.events[0].beat - 3.5).abs() < 1e-9);
        assert!((l.events[0].dur - 0.5).abs() < 1e-9);
    }
}
