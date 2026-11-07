//! HillDAW as a `hill_core::App`.
//!
//! Skeleton scope (handoff step 2): the mixer/transport screen and an arrangement screen for the drum track, driven by
//! the Hill apps' controls, with the real mixer in the audio callback. Recording, the FM-1 keys, the sampler and export
//! are not wired yet; the screens say so rather than pretend.
//!
//! Threading: the runner calls `audio` from the SDL audio thread and `event/tick/render` from the UI thread under one
//! mutex, so `render` must stay cheap. Anything slow (battery, storage, USB presence) is gathered by a background
//! thread into a [`status::StatusHandle`] and only copied here.

pub mod status;
mod view;

use hill_core::input::{Held, Repeat, TapTempo};
use hill_core::presets;
use hill_core::ui::{Help, Toast, QUIT_HOLD_MS};
use hill_core::{fb::Fb, App, Btn, Host};
use hilldaw_core::drums::DrumPattern;
use hilldaw_core::mixer::{Click, Mixer, Track};
use hilldaw_core::project::{DrumSectionModel, Project, MAX_DRUM_SECTIONS, MAX_SECTION_CYCLES, MAX_TRACKS};
use hilldaw_core::transport::Transport;
use hilldaw_drums::{find_preset, DrumLibrary};
use status::{Status, StatusHandle};
use std::sync::Arc;

/// Key under which the project is kept in the host's state store.
pub const STATE_KEY: &str = "hilldaw";
/// Mixer track indices: 0..8 are the audio tracks, then the drums, then a hidden metronome track.
pub const DRUMS: usize = MAX_TRACKS;
const CLICK: usize = MAX_TRACKS + 1;
/// Strips on screen: 8 tracks + drums.
pub const STRIPS: usize = MAX_TRACKS + 1;
pub const MIN_BPM: f64 = 40.0;
pub const MAX_BPM: f64 = 240.0;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum View {
    Mixer,
    Arrange,
}

pub(crate) const HELP: &[(&str, &str)] = &[
    ("Everywhere", ""),
    ("START", "play / stop"),
    ("SELECT", "record (not wired yet)"),
    ("L1 / R1", "previous / next screen"),
    ("FN + Left/Right", "tempo -1 / +1  (Up/Down: +-5)"),
    ("FN + A", "tap tempo"),
    ("FN + X", "click on / off"),
    ("FN + B", "back to the start"),
    ("FN + L2/R2", "drum swing -/+"),
    ("FN + SELECT", "this help"),
    ("START + SELECT hold", "quit"),
    ("Mixer", ""),
    ("Left / Right", "pick a strip"),
    ("Up / Down", "level -1 / +1 dB  (hold to repeat)"),
    ("L2 / R2", "pan left / right"),
    ("X", "mute"),
    ("Y + X", "solo"),
    ("A", "arm the track for recording"),
    ("B", "reset level and pan"),
    ("Arrange (drums)", ""),
    ("Up / Down", "pick a section"),
    ("Left / Right", "previous / next pattern"),
    ("L2 / R2", "previous / next style"),
    ("Y + Left/Right", "cycles -/+"),
    ("A", "add a section after this one"),
    ("X", "delete the section"),
];

pub struct HillDaw {
    lib: DrumLibrary,
    status_src: StatusHandle,
    pub(crate) status: Status,
    pub project: Project,
    pub(crate) mixer: Mixer,
    pub(crate) transport: Transport,
    rate: u32,
    spans: Vec<(u64, usize)>,
    /// loudest sample per strip since the last UI tick, then smoothed for drawing
    hold: [f32; STRIPS],
    hold_master: f32,
    pub(crate) meter: [f32; STRIPS],
    pub(crate) meter_master: f32,

    pub(crate) view: View,
    pub(crate) sel: usize,
    pub(crate) armed: [bool; MAX_TRACKS],
    pub(crate) click_on: bool,
    pub(crate) arr_sel: usize,

    // drum arrangement, rebuilt with the track
    pub(crate) drum_total: u64,
    pub(crate) drum_starts: Vec<u64>,
    pub(crate) section_pats: Vec<(Arc<DrumPattern>, Option<String>)>,

    held: Held,
    rep: Repeat,
    tap: TapTempo,
    ms: u64,
    /// START or SELECT went down as part of a chord, so their release must not also act
    combo: bool,
    pub(crate) quit_ms: u32,
    pub(crate) help: Help,
    pub(crate) toast: Toast,
    status_ms: u32,
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub(crate) fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-6).log10()
}

/// One 1 dB step up or down from `g`; the bottom of the range is silence.
fn step_gain(g: f32, dir: i32) -> f32 {
    if g <= 0.0 {
        return if dir > 0 { db_to_gain(-59.0) } else { 0.0 };
    }
    let db = (gain_to_db(g).round() + dir as f32).clamp(-60.0, 6.0);
    if db <= -60.0 { 0.0 } else { db_to_gain(db) }
}

impl HillDaw {
    pub fn new(host: Box<dyn Host>, status: StatusHandle) -> Self {
        let mut lib = DrumLibrary::new(host);
        let mut toast = Toast::default();
        let mut project = match lib.host_mut().load_state(STATE_KEY) {
            Some(text) => Project::from_json(&text).unwrap_or_else(|e| {
                toast.say("Saved project unreadable: new one");
                eprintln!("hilldaw: {e}");
                Self::fresh_project()
            }),
            None => Self::fresh_project(),
        };
        // a preset that no longer exists (older project, changed preset set) is dropped, not fatal
        let before = project.drums.sections.len();
        project.drums.sections.retain(|s| find_preset(&s.preset).is_some());
        if project.drums.sections.len() != before {
            toast.say("Some drum sections used unknown presets: removed");
        }
        let rate = hilldaw_core::RATE;
        let mut app = HillDaw {
            lib,
            status_src: status,
            status: Status::default(),
            project,
            mixer: Mixer::new(rate, 1024),
            transport: Transport::new(rate),
            rate,
            spans: Vec::new(),
            hold: [0.0; STRIPS],
            hold_master: 0.0,
            meter: [0.0; STRIPS],
            meter_master: 0.0,
            view: View::Mixer,
            sel: 0,
            armed: [false; MAX_TRACKS],
            click_on: false,
            arr_sel: 0,
            drum_total: 0,
            drum_starts: Vec::new(),
            section_pats: Vec::new(),
            held: Held::default(),
            rep: Repeat::default(),
            tap: TapTempo::default(),
            ms: 0,
            combo: false,
            quit_ms: 0,
            help: Help::default(),
            toast,
            status_ms: 0,
        };
        for i in 0..STRIPS + 1 {
            let name = if i < MAX_TRACKS { format!("TRACK {}", i + 1) } else if i == DRUMS { "DRUMS".into() } else { "CLICK".into() };
            app.mixer.tracks.push(Track::new(&name));
        }
        app.transport.bpm = app.project.bpm;
        app.transport.beats_per_bar = app.project.beats_per_bar;
        app.rebuild_click();
        app.rebuild_drums();
        app.apply_mixer();
        app
    }

    fn fresh_project() -> Project {
        let mut p = Project { name: "SONG".into(), bpm: 100.0, ..Project::default() };
        // a starting groove so START makes sound straight away
        if let Some(x) = presets::builtin().patterns.iter().find(|x| x.style == "Rock").or(presets::builtin().patterns.first()) {
            p.drums.sections.push(DrumSectionModel { name: x.name.clone(), preset: x.id.clone(), cycles: 4 });
        }
        p
    }

    // ---- keeping the engine in step with the project --------------------------------------------------------------

    fn rebuild_click(&mut self) {
        let c = Click { bpm: self.project.bpm, beats_per_bar: self.project.beats_per_bar as u64, amp: 0.5, enabled: self.click_on };
        self.mixer.tracks[CLICK].sources = vec![Box::new(c)];
    }

    /// Rebuild the drum source and the caches the arrangement screen draws from. Patterns and samples are cached in
    /// the library, so this is cheap enough to do on every edit.
    fn rebuild_drums(&mut self) {
        self.section_pats.clear();
        for s in &self.project.drums.sections {
            if let Ok(lp) = self.lib.pattern(&s.preset) {
                self.section_pats.push((lp.pattern, lp.kit));
            }
        }
        match self.lib.build_track(&self.project.drums, self.project.bpm) {
            Ok(mut t) => {
                t.gain = 1.0; // the strip's level is the mixer track's gain
                self.drum_total = t.total_frames(self.rate);
                self.drum_starts = t.section_starts(self.rate).into_iter().map(|(_, f)| f).collect();
                self.mixer.tracks[DRUMS].sources = vec![Box::new(t)];
            }
            Err(e) => {
                self.toast.say("Drums failed to load");
                eprintln!("hilldaw: {e}");
                self.mixer.tracks[DRUMS].sources.clear();
                self.drum_total = 0;
                self.drum_starts.clear();
            }
        }
        self.arr_sel = self.arr_sel.min(self.project.drums.sections.len().saturating_sub(1));
    }

    fn apply_mixer(&mut self) {
        for (t, m) in self.mixer.tracks.iter_mut().zip(self.project.tracks.iter()) {
            t.gain = m.gain;
            t.pan = m.pan;
            t.mute = m.mute;
            t.solo = m.solo;
        }
        let d = &self.project.drums;
        let t = &mut self.mixer.tracks[DRUMS];
        (t.gain, t.pan, t.mute, t.solo) = (d.gain, d.pan, !d.enabled, d.solo);
        // the metronome must stay audible while something is soloed
        let any_solo = self.mixer.tracks[..STRIPS].iter().any(|t| t.solo);
        let c = &mut self.mixer.tracks[CLICK];
        (c.gain, c.pan, c.mute, c.solo) = (1.0, 0.0, false, any_solo);
    }

    // ---- strip accessors (the drum strip lives in `project.drums`) -------------------------------------------------

    pub(crate) fn strip_gain(&self, i: usize) -> f32 {
        if i == DRUMS { self.project.drums.gain } else { self.project.tracks[i].gain }
    }
    pub(crate) fn strip_pan(&self, i: usize) -> f32 {
        if i == DRUMS { self.project.drums.pan } else { self.project.tracks[i].pan }
    }
    pub(crate) fn strip_muted(&self, i: usize) -> bool {
        if i == DRUMS { !self.project.drums.enabled } else { self.project.tracks[i].mute }
    }
    pub(crate) fn strip_solo(&self, i: usize) -> bool {
        if i == DRUMS { self.project.drums.solo } else { self.project.tracks[i].solo }
    }
    fn set_strip(&mut self, i: usize, gain: Option<f32>, pan: Option<f32>, mute: Option<bool>, solo: Option<bool>) {
        if i == DRUMS {
            let d = &mut self.project.drums;
            d.gain = gain.unwrap_or(d.gain);
            d.pan = pan.unwrap_or(d.pan);
            d.enabled = mute.map_or(d.enabled, |m| !m);
            d.solo = solo.unwrap_or(d.solo);
        } else {
            let t = &mut self.project.tracks[i];
            t.gain = gain.unwrap_or(t.gain);
            t.pan = pan.unwrap_or(t.pan);
            t.mute = mute.unwrap_or(t.mute);
            t.solo = solo.unwrap_or(t.solo);
        }
        self.apply_mixer();
    }

    // ---- tempo / transport -----------------------------------------------------------------------------------------

    fn set_bpm(&mut self, bpm: f64) {
        let bpm = bpm.clamp(MIN_BPM, MAX_BPM);
        // keep the musical position when the tempo changes while playing
        let beats = self.transport.pos as f64 / self.transport.frames_per_beat();
        self.project.bpm = bpm;
        self.transport.bpm = bpm;
        self.transport.pos = self.transport.beat_to_frame(beats);
        self.rebuild_click();
        self.rebuild_drums();
        self.toast.say(&format!("{:.0} BPM", bpm));
    }

    fn toggle_play(&mut self) {
        self.transport.playing = !self.transport.playing;
        if !self.transport.playing {
            // a stopped transport renders nothing; let the meters fall
            self.hold = [0.0; STRIPS];
            self.hold_master = 0.0;
        }
    }

    // ---- controls -----------------------------------------------------------------------------------------------------

    fn act(&mut self, b: Btn) {
        if self.held.is(Btn::Fn) {
            self.fn_act(b);
            return;
        }
        match self.view {
            View::Mixer => self.mixer_act(b),
            View::Arrange => self.arrange_act(b),
        }
    }

    fn fn_act(&mut self, b: Btn) {
        match b {
            Btn::Left => self.set_bpm(self.project.bpm - 1.0),
            Btn::Right => self.set_bpm(self.project.bpm + 1.0),
            Btn::Down => self.set_bpm(self.project.bpm - 5.0),
            Btn::Up => self.set_bpm(self.project.bpm + 5.0),
            Btn::L2 | Btn::R2 => {
                let d = if b == Btn::L2 { -0.01 } else { 0.01 };
                self.project.drums.swing = (self.project.drums.swing + d).clamp(0.0, 0.25);
                self.project.drums.swing = (self.project.drums.swing * 100.0).round() / 100.0;
                self.rebuild_drums();
                self.toast.say(&format!("Swing {}%", 50 + (self.project.drums.swing * 100.0).round() as i32));
            }
            Btn::A => {
                if let Some(bpm) = self.tap.tap(self.ms) {
                    self.set_bpm(bpm as f64);
                } else {
                    self.toast.say("Tap again");
                }
            }
            Btn::X => {
                self.click_on = !self.click_on;
                self.rebuild_click();
                self.toast.say(if self.click_on { "Click on" } else { "Click off" });
            }
            Btn::B => {
                self.transport.pos = 0;
                self.toast.say("Back to start");
            }
            _ => {}
        }
    }

    fn mixer_act(&mut self, b: Btn) {
        let i = self.sel;
        match b {
            Btn::Left => self.sel = i.saturating_sub(1),
            Btn::Right => self.sel = (i + 1).min(STRIPS - 1),
            Btn::Up | Btn::Down => {
                let g = step_gain(self.strip_gain(i), if b == Btn::Up { 1 } else { -1 });
                self.set_strip(i, Some(g), None, None, None);
            }
            Btn::L2 | Btn::R2 => {
                let p = self.strip_pan(i) + if b == Btn::L2 { -0.05 } else { 0.05 };
                let p = if p.abs() < 0.025 { 0.0 } else { p.clamp(-1.0, 1.0) };
                self.set_strip(i, None, Some((p * 100.0).round() / 100.0), None, None);
            }
            Btn::X => {
                if self.held.is(Btn::Y) {
                    let s = !self.strip_solo(i);
                    self.set_strip(i, None, None, None, Some(s));
                } else {
                    let m = !self.strip_muted(i);
                    self.set_strip(i, None, None, Some(m), None);
                }
            }
            Btn::A => {
                if i < MAX_TRACKS {
                    self.armed[i] = !self.armed[i];
                    self.toast.say(&format!("Track {} {}", i + 1, if self.armed[i] { "armed" } else { "disarmed" }));
                } else {
                    self.toast.say("The drum track can't be armed");
                }
            }
            Btn::B => self.set_strip(i, Some(1.0), Some(0.0), None, None),
            _ => {}
        }
    }

    fn arrange_act(&mut self, b: Btn) {
        let n = self.project.drums.sections.len();
        let all = &presets::builtin().patterns;
        let cur = self.project.drums.sections.get(self.arr_sel).and_then(|s| all.iter().position(|p| p.id == s.preset));
        match b {
            Btn::Up => self.arr_sel = self.arr_sel.saturating_sub(1),
            Btn::Down => self.arr_sel = (self.arr_sel + 1).min(n.saturating_sub(1)),
            Btn::Left | Btn::Right if self.held.is(Btn::Y) => {
                if let Some(s) = self.project.drums.sections.get_mut(self.arr_sel) {
                    let c = s.cycles as i64 + if b == Btn::Left { -1 } else { 1 };
                    s.cycles = c.clamp(1, MAX_SECTION_CYCLES as i64) as u32;
                    self.rebuild_drums();
                }
            }
            Btn::Left | Btn::Right => {
                if let Some(c) = cur {
                    let next = (c as i64 + if b == Btn::Left { -1 } else { 1 }).rem_euclid(all.len() as i64) as usize;
                    self.set_section_preset(next);
                }
            }
            Btn::L2 | Btn::R2 => {
                // jump to the first pattern of the next / previous style
                if let Some(c) = cur {
                    let styles = &presets::builtin().styles;
                    if let Some(si) = styles.iter().position(|st| *st == all[c].style) {
                        let ti = (si as i64 + if b == Btn::R2 { 1 } else { -1 }).rem_euclid(styles.len() as i64) as usize;
                        if let Some(t) = all.iter().position(|p| p.style == styles[ti]) {
                            self.set_section_preset(t);
                        }
                    }
                }
            }
            Btn::A => {
                if n >= MAX_DRUM_SECTIONS {
                    self.toast.say("Section limit reached");
                } else {
                    let new = self.project.drums.sections.get(self.arr_sel).cloned().or_else(|| {
                        all.first().map(|x| DrumSectionModel { name: x.name.clone(), preset: x.id.clone(), cycles: 4 })
                    });
                    if let Some(s) = new {
                        let at = if n == 0 { 0 } else { self.arr_sel + 1 };
                        self.project.drums.sections.insert(at, s);
                        self.arr_sel = at;
                        self.rebuild_drums();
                        self.toast.say("Section added");
                    }
                }
            }
            Btn::X if n > 0 => {
                self.project.drums.sections.remove(self.arr_sel);
                self.rebuild_drums();
                self.toast.say("Section deleted");
            }
            _ => {}
        }
    }

    fn set_section_preset(&mut self, idx: usize) {
        let all = &presets::builtin().patterns;
        if let (Some(s), Some(p)) = (self.project.drums.sections.get_mut(self.arr_sel), all.get(idx)) {
            s.preset = p.id.clone();
            s.name = p.name.clone();
            self.rebuild_drums();
        }
    }
}

/// Read-only views of the state, for the tests and for other front-ends.
impl HillDaw {
    pub fn view(&self) -> View {
        self.view
    }
    pub fn sel(&self) -> usize {
        self.sel
    }
    pub fn is_playing(&self) -> bool {
        self.transport.playing
    }
    pub fn bar_beat(&self) -> (u32, u32) {
        self.transport.bar_beat()
    }
    pub fn help_open(&self) -> bool {
        self.help.open
    }
    pub fn toast_text(&self) -> &str {
        &self.toast.text
    }
    pub fn status(&self) -> &Status {
        &self.status
    }
    /// Smoothed meter level of a strip, 0..1 linear.
    pub fn meter(&self, strip: usize) -> f32 {
        self.meter[strip]
    }
    /// Length of the drum arrangement in frames.
    pub fn drum_frames(&self) -> u64 {
        self.drum_total
    }
}

impl App for HillDaw {
    fn name(&self) -> &'static str {
        "hilldaw"
    }

    fn event(&mut self, b: Btn, down: bool) {
        self.held.set(b, down);
        if !down {
            self.rep.stop(b);
            if matches!(b, Btn::Fn | Btn::Y) {
                self.rep.clear(); // a held repeat must not carry on as a different action
            }
            match b {
                // START and SELECT act on release so that holding both (quit) does nothing else
                Btn::Start => {
                    if !self.combo && !self.held.is(Btn::Fn) {
                        self.toggle_play();
                    }
                    if !self.held.is(Btn::Select) {
                        self.combo = false;
                    }
                }
                Btn::Select => {
                    if !self.combo {
                        self.toast.say("Recording isn't wired yet");
                    }
                    if !self.held.is(Btn::Start) {
                        self.combo = false;
                    }
                }
                _ => {}
            }
            return;
        }
        if b == Btn::Select {
            if self.held.is(Btn::Fn) {
                self.combo = true;
                self.help.show();
                return;
            }
            if self.held.is(Btn::Start) {
                self.combo = true;
            }
            return;
        }
        if b == Btn::Start {
            if self.held.is(Btn::Select) {
                self.combo = true;
            }
            return;
        }
        if self.help.open {
            self.help.event(b, HELP.len());
            return;
        }
        match b {
            Btn::L1 | Btn::R1 => {
                self.view = if self.view == View::Mixer { View::Arrange } else { View::Mixer };
                self.rep.clear();
            }
            _ => {
                self.act(b);
                if matches!(b, Btn::Up | Btn::Down | Btn::Left | Btn::Right | Btn::L2 | Btn::R2) {
                    self.rep.start(b);
                }
            }
        }
    }

    fn audio(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let frames = out.len() / 2;
        let mut spans = std::mem::take(&mut self.spans);
        self.transport.advance(frames, &mut spans);
        let mut at = 0usize;
        for &(start, len) in &spans {
            self.mixer.render(start, &mut out[at * 2..(at + len) * 2]);
            for i in 0..STRIPS {
                self.hold[i] = self.hold[i].max(self.mixer.tracks[i].peak);
            }
            self.hold_master = self.hold_master.max(self.mixer.master_peak);
            at += len;
        }
        self.spans = spans;
    }

    fn render(&self, fb: &mut Fb) {
        self.draw(fb);
    }

    fn tick(&mut self, dt_ms: u32) {
        self.ms += dt_ms as u64;
        self.toast.tick(dt_ms);
        if self.held.is(Btn::Start) && self.held.is(Btn::Select) {
            self.quit_ms += dt_ms;
        } else {
            self.quit_ms = 0;
        }
        if !self.help.open {
            if let Some(b) = self.rep.tick(dt_ms) {
                self.act(b);
            }
        }
        // meters: take the peak since the last tick, fall smoothly
        let fall = (-(dt_ms as f32) / 300.0).exp();
        for i in 0..STRIPS {
            self.meter[i] = self.hold[i].max(self.meter[i] * fall);
            self.hold[i] = 0.0;
        }
        self.meter_master = self.hold_master.max(self.meter_master * fall);
        self.hold_master = 0.0;
        self.status_ms += dt_ms;
        if self.status_ms >= 500 {
            self.status_ms = 0;
            if let Ok(s) = self.status_src.lock() {
                self.status = s.clone();
            }
        }
    }

    fn set_rate(&mut self, rate: f32) {
        // Project positions are frames at the output rate; a device that refuses 44.1 kHz keeps working at its own rate.
        self.rate = rate as u32;
        self.transport.rate = self.rate;
        self.mixer.rate = self.rate;
        self.rebuild_drums();
    }

    fn wants_quit(&self) -> bool {
        self.quit_ms >= QUIT_HOLD_MS
    }

    fn save(&mut self) {
        let json = self.project.to_json();
        self.lib.host_mut().save_state(STATE_KEY, &json);
    }
}
