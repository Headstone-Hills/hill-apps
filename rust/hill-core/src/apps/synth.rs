//! HillSynth: a very simple digital synthesizer for the game controller.
//!
//! Play: the D-pad (Up, Right, Down, Left) and the face buttons (X, A, B, Y) climb a scale
//! clockwise, as in HillChord's note mode. L1/R1 shift the octave, L2 is a sustain pedal.
//! Edit (R2): the D-pad moves through the sound's parameters; the face buttons still play,
//! so every change is heard as it is made. START opens the presets, START+X exports the
//! sound as a multisample into the Samples folder for the other Hill apps.

use crate::fb::*;
use crate::host::Host;
use crate::input::{Btn, Held, Repeat};
use crate::kv::Kv;
use crate::library::parse_pitch;
use crate::subtractive::*;
use crate::theory::PC_NAMES;
use crate::ui::{quit_bar, Help, Toast, QUIT_HOLD_MS};
use crate::wav::encode16;
use crate::{App, SAMPLE_RATE};

const SLOT_BTNS: [Btn; 8] = [Btn::Up, Btn::Right, Btn::Down, Btn::Left, Btn::X, Btn::A, Btn::B, Btn::Y];
const SCALES: [(&str, &[i32]); 4] = [
    ("major", &[0, 2, 4, 5, 7, 9, 11]),
    ("minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("pent major", &[0, 2, 4, 7, 9]),
    ("pent minor", &[0, 3, 5, 7, 10]),
];
const USER_SLOTS: usize = 16;
/// Export: every third semitone, C1 to C7 (the same spread HillSynth always wrote).
const EXPORT_FROM: u8 = 24;
const EXPORT_TO: u8 = 96;
const EXPORT_STEP: usize = 3;
const EXPORT_HOLD: f32 = 3.0;
const EXPORT_TAIL: f32 = 1.5;
const EXPORT_RATE: f32 = 44100.0;

const HELP: &[(&str, &str)] = &[
    ("PLAYING", ""),
    ("D-pad", "notes 1-4: up, right, down, left"),
    ("X / A / B / Y", "notes 5-8, clockwise on from the D-pad"),
    ("L1 / R1", "octave down / up"),
    ("L2 (hold)", "sustain pedal"),
    ("FN+Up/Down", "key root up / down"),
    ("FN+Left/Right", "octave down / up"),
    ("SELECT+X", "scale: major, minor, pent major, pent minor"),
    ("EDITING THE SOUND", ""),
    ("R2", "play / edit"),
    ("  in edit: U/D", "choose a setting"),
    ("  in edit: L/R", "change it (hold repeats; SELECT = bigger steps)"),
    ("  in edit: L1/R1", "previous / next page"),
    ("  in edit: X A B Y", "play notes 5-8 to hear the change"),
    ("PRESETS & EXPORT", ""),
    ("START", "presets (A load, Y save to a User slot, X clear)"),
    ("START+X", "export the sound as a multisample to Samples"),
    ("START+Y", "save"),
    ("OTHER", ""),
    ("START+SELECT", "tap: all notes off, hold 1.5s: quit"),
    ("FN+SELECT", "this help"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Play,
    Edit,
}

enum Overlay {
    None,
    Help,
    Presets { sel: usize },
}

/// A multisample export in progress: one note rendered or written per UI tick, so the
/// sound and the screen keep going.
struct Export {
    name: String,
    patch: Patch,
    notes: Vec<u8>,
    bufs: Vec<Vec<f32>>,
    gain: f32,
    written: usize,
}

pub struct SynthApp {
    host: Box<dyn Host>,
    synth: Synth,
    mode: Mode,
    page: usize,
    row: usize,
    root: u8,
    scale: usize,
    octave: i32,
    slots: [Option<u8>; 8],
    held: Held,
    rep: Repeat,
    toast: Toast,
    help: Help,
    overlay: Overlay,
    factory: Vec<(&'static str, Patch)>,
    user: Vec<Option<(String, Patch)>>,
    preset: String,
    start_plain: bool,
    ss: bool,
    ss_ms: u32,
    quit: bool,
    export: Option<Export>,
    tmp: Vec<f32>,
    scope: [f32; 256],
    scope_pos: usize,
    scope_skip: usize,
}

pub fn new(host: Box<dyn Host>) -> SynthApp {
    let factory = factory_presets();
    let mut a = SynthApp {
        host,
        synth: Synth::new(SAMPLE_RATE),
        mode: Mode::Play,
        page: 0,
        row: 0,
        root: 0,
        scale: 0,
        octave: 3,
        slots: [None; 8],
        held: Held::default(),
        rep: Repeat::default(),
        toast: Toast::default(),
        help: Help::default(),
        overlay: Overlay::None,
        preset: factory[0].0.to_string(),
        factory,
        user: vec![None; USER_SLOTS],
        start_plain: false,
        ss: false,
        ss_ms: 0,
        quit: false,
        export: None,
        tmp: vec![0.0; 2048],
        scope: [0.0; 256],
        scope_pos: 0,
        scope_skip: 0,
    };
    a.synth.patch = a.factory[0].1;
    if let Some(text) = a.host.load_state("hillsynth") {
        a.restore(&text);
    }
    a
}

/// A file-name-safe version of a preset name. Tokens that look like note names (`A2`) are
/// dropped: the other apps would read them as the pitch of the file.
fn safe_name(s: &str) -> String {
    let parts: Vec<&str> = s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty() && parse_pitch(t).is_none()).collect();
    if parts.is_empty() {
        "Synth".to_string()
    } else {
        parts.join("_")
    }
}

/// `Eb3`-style note name, with flats like the rest of the Hill sample libraries.
fn flat_name(midi: u8) -> String {
    const N: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];
    format!("{}{}", N[(midi % 12) as usize], midi as i32 / 12 - 1)
}

impl SynthApp {
    fn slot_of(b: Btn) -> Option<usize> {
        SLOT_BTNS.iter().position(|x| *x == b)
    }

    /// MIDI note for scale degree `d` (0 = the root) in the current key, scale and octave.
    fn degree_midi(&self, d: usize) -> u8 {
        let iv = SCALES[self.scale].1;
        let m = 12 * (self.octave + 1) + self.root as i32 + iv[d % iv.len()] + 12 * (d / iv.len()) as i32;
        m.clamp(0, 127) as u8
    }

    fn note_on_slot(&mut self, slot: usize) {
        if let Some(old) = self.slots[slot].take() {
            self.synth.note_off(old);
        }
        let n = self.degree_midi(slot);
        self.slots[slot] = Some(n);
        self.synth.note_on(n);
    }

    fn note_off_slot(&mut self, slot: usize) {
        if let Some(n) = self.slots[slot].take() {
            self.synth.note_off(n);
        }
    }

    fn panic(&mut self) {
        self.synth.all_off();
        self.slots = [None; 8];
    }

    fn set_mode(&mut self, m: Mode) {
        self.mode = m;
        // the D-pad stops being notes in edit mode: let go of any it was holding
        for s in 0..4 {
            self.note_off_slot(s);
        }
        self.rep.clear();
        self.toast.say(if m == Mode::Edit { "Edit" } else { "Play" });
    }

    fn param_at(&self) -> usize {
        let rows = PAGES[self.page].1;
        rows[self.row.min(rows.len() - 1)]
    }

    fn adjust(&mut self, dir: i32) {
        let i = self.param_at();
        let coarse = self.held.is(Btn::Select);
        self.synth.patch.step(i, dir, coarse);
    }

    fn change_page(&mut self, d: i32) {
        self.page = (self.page as i32 + d).rem_euclid(PAGES.len() as i32) as usize;
        self.row = 0;
    }

    fn load_preset(&mut self, sel: usize) {
        let loaded = if sel < self.factory.len() {
            Some((self.factory[sel].0.to_string(), self.factory[sel].1))
        } else {
            self.user[sel - self.factory.len()].clone()
        };
        match loaded {
            Some((name, patch)) => {
                self.synth.patch = patch;
                self.preset = name.clone();
                self.toast.say(&format!("Loaded {}", name));
            }
            None => self.toast.say("Empty slot"),
        }
    }

    // ---- export ----------------------------------------------------------

    fn start_export(&mut self) {
        if self.export.is_some() {
            return;
        }
        let notes: Vec<u8> = (EXPORT_FROM..=EXPORT_TO).step_by(EXPORT_STEP).collect();
        let name = safe_name(&self.preset);
        self.toast.say(&format!("Exporting {}...", name));
        self.export = Some(Export { name, patch: self.synth.patch, notes, bufs: Vec::new(), gain: 1.0, written: 0 });
    }

    fn export_step(&mut self) {
        let Some(e) = self.export.as_mut() else { return };
        if e.bufs.len() < e.notes.len() {
            let midi = e.notes[e.bufs.len()];
            e.bufs.push(render_note(&e.patch, midi, EXPORT_RATE, EXPORT_HOLD, EXPORT_TAIL));
            if e.bufs.len() == e.notes.len() {
                // one gain for the whole instrument, so the balance between notes is kept
                let peak = e.bufs.iter().flat_map(|b| b.iter()).fold(0.0f32, |m, x| m.max(x.abs()));
                e.gain = if peak > 1e-4 { (0.89 / peak).min(4.0) } else { 1.0 };
            }
            return;
        }
        let i = e.written;
        let midi = e.notes[i];
        let data: Vec<f32> = e.bufs[i].iter().map(|x| x * e.gain).collect();
        let path = format!("HillSynth/{n}/{n}_midi{:03}_{}.wav", midi, flat_name(midi), n = e.name);
        let ok = self.host.write_sample(&path, &encode16(&data, 1, EXPORT_RATE as u32));
        let e = self.export.as_mut().unwrap();
        if !ok {
            self.export = None;
            self.toast.say("Can't write to the Samples folder");
            return;
        }
        e.bufs[i] = Vec::new(); // free it as we go
        e.written += 1;
        if e.written == e.notes.len() {
            let msg = format!("Exported {} notes: Samples/HillSynth/{}", e.notes.len(), e.name);
            self.export = None;
            self.toast.say(&msg);
        }
    }

    // ---- state -----------------------------------------------------------

    fn state_string(&self) -> String {
        let mut k = Kv::new();
        k.set("root", self.root);
        k.set("scale", self.scale);
        k.set("octave", self.octave);
        k.set("preset", &self.preset);
        k.set("patch", self.synth.patch.to_csv());
        for (i, u) in self.user.iter().enumerate() {
            if let Some((name, p)) = u {
                k.set(&format!("user{}", i), format!("{}|{}", name, p.to_csv()));
            }
        }
        k.render()
    }

    fn restore(&mut self, text: &str) {
        let k = Kv::parse(text);
        self.root = k.usize("root").unwrap_or(0).min(11) as u8;
        self.scale = k.usize("scale").unwrap_or(0).min(SCALES.len() - 1);
        self.octave = k.f32("octave").map(|o| o as i32).unwrap_or(3).clamp(1, 6);
        if let Some(p) = k.get("preset") {
            self.preset = p.to_string();
        }
        if let Some(p) = k.get("patch").and_then(Patch::from_csv) {
            self.synth.patch = p;
        }
        for i in 0..USER_SLOTS {
            self.user[i] = k.get(&format!("user{}", i)).and_then(|v| {
                let (name, csv) = v.split_once('|')?;
                Some((name.to_string(), Patch::from_csv(csv)?))
            });
        }
    }

    fn save_now(&mut self) {
        let s = self.state_string();
        self.host.save_state("hillsynth", &s);
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
            Btn::X | Btn::A | Btn::B | Btn::Y => {
                if st {
                    self.start_plain = false;
                    match b {
                        Btn::X => self.start_export(),
                        Btn::Y => {
                            self.save_now();
                            self.toast.say("Saved");
                        }
                        _ => {}
                    }
                } else if se && b == Btn::X {
                    self.scale = (self.scale + 1) % SCALES.len();
                    self.toast.say(&format!("Scale: {}", SCALES[self.scale].0));
                } else if !se && !fnh {
                    if let Some(s) = Self::slot_of(b) {
                        self.note_on_slot(s);
                    }
                }
            }
            Btn::L1 | Btn::R1 => {
                let d = if b == Btn::L1 { -1 } else { 1 };
                if !st && !se {
                    if self.mode == Mode::Edit {
                        self.change_page(d);
                    } else {
                        self.octave = (self.octave + d).clamp(1, 6);
                        self.toast.say(&format!("Octave {}", self.octave));
                    }
                }
            }
            Btn::R2 => {
                if !st && !se {
                    self.set_mode(if self.mode == Mode::Play { Mode::Edit } else { Mode::Play });
                }
            }
            Btn::L2 => {
                if !st && !se {
                    self.synth.set_pedal(true);
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
                    self.toast.say(&format!("{} {}  oct {}", PC_NAMES[self.root as usize], SCALES[self.scale].0, self.octave));
                } else if self.mode == Mode::Edit {
                    let rows = PAGES[self.page].1.len();
                    match b {
                        Btn::Up => self.row = (self.row + rows - 1) % rows,
                        Btn::Down => self.row = (self.row + 1) % rows,
                        _ => {
                            self.adjust(if b == Btn::Left { -1 } else { 1 });
                            self.rep.start(b);
                        }
                    }
                } else if let Some(s) = Self::slot_of(b) {
                    self.note_on_slot(s);
                }
            }
            Btn::L3 | Btn::R3 => {}
        }
    }

    fn release(&mut self, b: Btn) {
        if let Some(s) = Self::slot_of(b) {
            self.note_off_slot(s);
        }
        match b {
            Btn::Start => {
                if self.ss {
                    self.finish_ss();
                } else if self.start_plain {
                    self.overlay = Overlay::Presets { sel: 0 };
                }
                self.start_plain = false;
            }
            Btn::Select => {
                if self.ss {
                    self.finish_ss();
                }
            }
            Btn::L2 => self.synth.set_pedal(false),
            Btn::Left | Btn::Right => self.rep.stop(b),
            _ => {}
        }
    }

    fn finish_ss(&mut self) {
        self.ss = false;
        if !self.quit {
            self.panic();
            self.toast.say("All notes off");
        }
    }

    fn overlay_event(&mut self, b: Btn) {
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        match overlay {
            Overlay::None => {}
            Overlay::Help => {
                self.help.event(b, HELP.len());
                if self.help.open {
                    self.overlay = Overlay::Help;
                }
            }
            Overlay::Presets { mut sel } => {
                let total = self.factory.len() + USER_SLOTS;
                match b {
                    Btn::Up => sel = (sel + total - 1) % total,
                    Btn::Down => sel = (sel + 1) % total,
                    Btn::L1 => sel = sel.saturating_sub(8),
                    Btn::R1 => sel = (sel + 8).min(total - 1),
                    Btn::A => {
                        self.load_preset(sel);
                        return;
                    }
                    Btn::B => return,
                    Btn::Y => {
                        if sel < self.factory.len() {
                            self.toast.say("Pick a User slot to save into");
                        } else {
                            let slot = sel - self.factory.len();
                            let name = format!("User {}", slot + 1);
                            self.user[slot] = Some((name.clone(), self.synth.patch));
                            self.preset = name.clone();
                            self.save_now();
                            self.toast.say(&format!("Saved as {}", name));
                        }
                    }
                    Btn::X => {
                        if sel >= self.factory.len() {
                            self.user[sel - self.factory.len()] = None;
                            self.toast.say("Slot cleared");
                        }
                    }
                    _ => {}
                }
                self.overlay = Overlay::Presets { sel };
            }
        }
    }

    // ---- drawing ---------------------------------------------------------

    fn note_label(&self, slot: usize) -> String {
        let n = self.degree_midi(slot);
        format!("{}{}", PC_NAMES[(n % 12) as usize], n as i32 / 12 - 1)
    }

    fn draw_scope(&self, fb: &mut Fb, x: i32, y: i32, w: i32, h: i32) {
        fb.rect(x, y, w, h, scale(PANEL, 0.7));
        fb.rect(x, y + h / 2, w, 1, CELL_BEAT);
        // start at a rising zero crossing so a steady note holds still
        let n = self.scope.len();
        let at = |i: usize| self.scope[(self.scope_pos + i) % n];
        let start = (0..n / 2).find(|&i| at(i) <= 0.0 && at(i + 1) > 0.0).unwrap_or(0);
        let mut prev: Option<i32> = None;
        for px in 0..w {
            let i = start + (px as usize * (n / 2)) / w as usize;
            let v = at(i).clamp(-1.0, 1.0);
            let yy = y + h / 2 - (v * (h as f32 / 2.0 - 2.0)) as i32;
            let (a, b) = match prev {
                Some(p) => (p.min(yy), p.max(yy)),
                None => (yy, yy),
            };
            fb.rect(x + px, a, 1, (b - a).max(1), MELODIC);
            prev = Some(yy);
        }
    }

    fn draw_envelope(&self, fb: &mut Fb, x: i32, y: i32, w: i32, h: i32) {
        let p = &self.synth.patch;
        fb.rect(x, y, w, h, scale(PANEL, 0.7));
        let (a, d, s, r) = (p.v[ATK], p.v[DEC], p.v[SUS], p.v[REL]);
        let hold = 0.4;
        let total = a + d.min(2.0) * 1.5 + hold + r;
        let t_of = |px: i32| px as f32 / w as f32 * total;
        for px in 0..w {
            let t = t_of(px);
            // the curve: linear attack, exponential decay to the sustain, hold, exponential release
            let level = if t < a {
                t / a.max(1e-4)
            } else if t < a + d.min(2.0) * 1.5 {
                s + (1.0 - s) * (-4.6 * (t - a) / d.max(1e-3)).exp()
            } else if t < a + d.min(2.0) * 1.5 + hold {
                s
            } else {
                s * (-4.6 * (t - (a + d.min(2.0) * 1.5 + hold)) / r.max(1e-3)).exp()
            };
            let ph = ((h - 6) as f32 * level.clamp(0.0, 1.0)) as i32;
            fb.rect(x + px, y + h - 3 - ph, 1, ph.max(1), ACCENT);
        }
    }

    fn draw_presets(&self, fb: &mut Fb, sel: usize) {
        let (x, y) = crate::ui::modal(fb, 520, 400, "Presets");
        let total = self.factory.len() + USER_SLOTS;
        let rows = 11;
        let top = sel.saturating_sub(rows / 2).min(total.saturating_sub(rows));
        for (n, i) in (top..(top + rows).min(total)).enumerate() {
            let yy = y + n as i32 * 26;
            if i == sel {
                fb.rect(x - 6, yy - 3, 492, 24, scale(MELODIC, 0.45));
            }
            let (label, col) = if i < self.factory.len() {
                (self.factory[i].0.to_string(), if i == sel { WHITE } else { TEXT })
            } else {
                match &self.user[i - self.factory.len()] {
                    Some((name, _)) => (name.clone(), if i == sel { WHITE } else { ACCENT }),
                    None => (format!("User {}  (empty)", i - self.factory.len() + 1), DIM),
                }
            };
            fb.text(x, yy, if i < self.factory.len() { "F" } else { "U" }, 2, DIM);
            fb.text(x + 30, yy, &clip(&label, 24), 2, col);
            if label == self.preset {
                fb.text_right(x + 480, yy + 4, "current", 1, DIM);
            }
        }
        fb.text(x, y + 300, "U/D choose  A load  Y save here (User slots)  X clear  B close", 1, DIM);
    }
}

impl App for SynthApp {
    fn name(&self) -> &'static str {
        "HillSynth"
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
            if let Some(s) = Self::slot_of(b) {
                self.note_off_slot(s); // a note ends whatever screen is open
            }
            if b == Btn::L2 {
                self.synth.set_pedal(false);
            }
            if matches!(self.overlay, Overlay::None) {
                self.release(b);
            } else {
                self.rep.stop(b);
            }
        }
    }

    fn audio(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        if self.tmp.len() < frames {
            self.tmp.resize(frames, 0.0);
        }
        self.tmp[..frames].iter_mut().for_each(|x| *x = 0.0);
        self.synth.render(&mut self.tmp[..frames]);
        for (i, o) in out.chunks_mut(2).enumerate() {
            let s = self.tmp[i];
            o[0] = s;
            if o.len() > 1 {
                o[1] = s;
            }
            // keep a short window of the output for the scope (every 4th sample)
            self.scope_skip += 1;
            if self.scope_skip >= 4 {
                self.scope_skip = 0;
                self.scope[self.scope_pos] = s;
                self.scope_pos = (self.scope_pos + 1) % self.scope.len();
            }
        }
    }

    fn render(&self, fb: &mut Fb) {
        fb.clear(BG);
        let p = &self.synth.patch;
        // header
        fb.text(14, 8, "HillSynth", 2, ACCENT);
        fb.text(190, 8, &clip(&self.preset, 18), 2, WHITE);
        let (tag, col) = if self.mode == Mode::Edit { ("EDIT", DRUM) } else { ("PLAY", MELODIC) };
        fb.text_right(W - 14, 8, tag, 2, col);
        fb.text(14, 34, &format!("{} {}   octave {}   voices {}", PC_NAMES[self.root as usize], SCALES[self.scale].0, self.octave, self.synth.active()), 1, DIM);

        // page tabs
        for (i, (name, _)) in PAGES.iter().enumerate() {
            let x = 14 + i as i32 * 122;
            let sel = i == self.page;
            fb.rect(x, 52, 116, 26, if sel { scale(MELODIC, if self.mode == Mode::Edit { 0.7 } else { 0.35 }) } else { PANEL });
            fb.text_center(x + 58, 58, name, 2, if sel { WHITE } else { DIM });
        }

        // the page's settings
        let rows = PAGES[self.page].1;
        for (n, &i) in rows.iter().enumerate() {
            let y = 92 + n as i32 * 34;
            let cur = self.mode == Mode::Edit && n == self.row.min(rows.len() - 1);
            if cur {
                fb.rect(8, y - 5, W - 16, 30, scale(MELODIC, 0.4));
            }
            fb.text(20, y, PARAMS[i].name, 2, if cur { WHITE } else { TEXT });
            fb.rect(250, y + 2, 200, 14, CELL_OFF);
            fb.rect(250, y + 2, (200.0 * p.fraction(i)) as i32, 14, if cur { ACCENT } else { MELODIC });
            fb.text_right(W - 20, y, &p.display(i), 2, if cur { WHITE } else { ACCENT });
        }

        // the sound: the output, or the envelope shape on the ENV page
        if self.page == 3 {
            self.draw_envelope(fb, 14, 250, W - 28, 90);
        } else {
            self.draw_scope(fb, 14, 250, W - 28, 90);
        }

        // what the buttons play
        let names: Vec<String> = (0..8).map(|s| self.note_label(s)).collect();
        let lit = |s: usize| self.slots[s].is_some();
        let cell = |fb: &mut Fb, x: i32, y: i32, tag: &str, slot: usize| {
            let on = lit(slot);
            fb.rect(x, y, 70, 38, if on { scale(ACCENT, 0.5) } else { PANEL });
            fb.text(x + 4, y + 3, tag, 1, DIM);
            fb.text_center(x + 35, y + 17, &names[slot], 2, if on { WHITE } else { TEXT });
        };
        let tags = ["Up", "Right", "Down", "Left", "X", "A", "B", "Y"];
        for s in 0..8 {
            let x = 14 + s as i32 * 76;
            if self.mode == Mode::Edit && s < 4 {
                continue; // the D-pad is for editing in edit mode
            }
            cell(fb, x, 352, tags[s], s);
        }
        let hint = if self.mode == Mode::Edit {
            "U/D setting   L/R change (SELECT x5)   L1/R1 page   X A B Y play   R2 back"
        } else {
            "D-pad + X A B Y play   L1/R1 octave   L2 hold   R2 edit   START presets"
        };
        fb.text(14, 446, hint, 1, DIM);
        fb.text(14, 462, "START+X export to Samples   FN+SELECT help", 1, DIM);

        // export progress
        if let Some(e) = &self.export {
            let done = e.bufs.len() + e.written;
            let total = e.notes.len() * 2;
            fb.rect(100, 190, 440, 70, rgb(10, 12, 16));
            fb.frame(100, 190, 440, 70, 2, ACCENT);
            fb.text_center(W / 2, 200, &format!("Exporting {}", clip(&e.name, 20)), 2, WHITE);
            fb.frame(120, 232, 400, 16, 2, DIM);
            fb.rect(122, 234, (396 * done / total.max(1)) as i32, 12, ACCENT);
        }
        self.toast.draw(fb);
        if self.ss && self.held.is(Btn::Start) && self.held.is(Btn::Select) {
            quit_bar(fb, self.ss_ms);
        }
        match &self.overlay {
            Overlay::None => {}
            Overlay::Help => self.help.draw(fb, "HillSynth", HELP),
            Overlay::Presets { sel } => self.draw_presets(fb, *sel),
        }
    }

    fn tick(&mut self, dt: u32) {
        self.toast.tick(dt);
        if self.ss && self.held.is(Btn::Start) && self.held.is(Btn::Select) {
            self.ss_ms += dt;
            if self.ss_ms >= QUIT_HOLD_MS {
                self.quit = true;
                self.ss = false;
                self.save_now();
            }
        }
        if matches!(self.overlay, Overlay::None) && self.mode == Mode::Edit {
            if let Some(b) = self.rep.tick(dt) {
                self.adjust(if b == Btn::Left { -1 } else { 1 });
            }
        } else {
            self.rep.clear();
        }
        self.export_step();
    }

    fn set_rate(&mut self, rate: f32) {
        self.synth.set_rate(rate);
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

    fn app() -> SynthApp {
        new(Box::new(MemHost::default()))
    }

    fn tap(a: &mut SynthApp, b: Btn) {
        a.event(b, true);
        a.event(b, false);
    }

    #[test]
    fn notes_climb_the_scale_clockwise_from_the_dpad_round_the_face_buttons() {
        let a = app();
        // C major, octave 3: Up, Right, Down, Left, X, A, B, Y = C D E F G A B C
        let notes: Vec<u8> = (0..8).map(|s| a.degree_midi(s)).collect();
        assert_eq!(notes, vec![48, 50, 52, 53, 55, 57, 59, 60]);
        assert_eq!(SynthApp::slot_of(Btn::Up), Some(0));
        assert_eq!(SynthApp::slot_of(Btn::X), Some(4));
        assert_eq!(SynthApp::slot_of(Btn::Y), Some(7));
        let mut m = app();
        m.scale = 1; // minor
        assert_eq!(m.degree_midi(2), 51);
        m.scale = 3; // pentatonic minor: five notes, so the 6th button is the root an octave up
        assert_eq!((m.degree_midi(0), m.degree_midi(5), m.degree_midi(7)), (48, 60, 65));
    }

    #[test]
    fn octave_and_key_change_the_notes_and_held_notes_still_release() {
        let mut a = app();
        a.event(Btn::Up, true);
        assert_eq!(a.slots[0], Some(48));
        tap(&mut a, Btn::R1); // octave up while it is held
        assert_eq!(a.octave, 4);
        a.event(Btn::Up, false);
        assert_eq!(a.slots[0], None, "the held note was released, not left hanging");
        assert_eq!(a.degree_midi(0), 60);
        a.event(Btn::Fn, true);
        tap(&mut a, Btn::Up); // key up a semitone
        a.event(Btn::Fn, false);
        assert_eq!(a.degree_midi(0), 61);
    }

    #[test]
    fn safe_names_drop_note_like_words() {
        assert_eq!(safe_name("Warm Pad"), "Warm_Pad");
        assert_eq!(safe_name("Bass A2"), "Bass");
        assert_eq!(safe_name("F#3"), "F_3", "the # is dropped, so no part reads as a note");
        assert!(parse_pitch("F_3").is_none() && parse_pitch("Bass").is_none() && parse_pitch("Warm_Pad").is_none());
        assert_eq!(safe_name("A4"), "Synth");
        assert_eq!(safe_name("User 3"), "User_3");
        assert_eq!(flat_name(63), "Eb4");
        assert_eq!(flat_name(24), "C1");
    }
}
