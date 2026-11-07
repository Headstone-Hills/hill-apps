//! HillDAW as a `hill_core::App`.
//!
//! Skeleton scope (handoff step 2): the mixer/transport screen and an arrangement screen for the drum track, driven by
//! the Hill apps' controls, with the real mixer in the audio callback. Recording, the FM-1 keys, the sampler and export
//! are not wired yet; the screens say so rather than pretend.
//!
//! Threading: the runner calls `audio` from the SDL audio thread and `event/tick/render` from the UI thread under one
//! mutex, so `render` must stay cheap. Anything slow (battery, storage, USB presence) is gathered by a background
//! thread into a [`status::StatusHandle`] and only copied here.

pub mod encode;
pub mod export;
pub mod fm1link;
pub mod recorder;
pub mod status;
pub mod usbout;
mod view;

use hill_core::input::{Held, Repeat, TapTempo};
use hill_core::presets;
use hill_core::ui::{Help, Toast, QUIT_HOLD_MS};
use hill_core::{fb::Fb, App, Btn, Host};
use hilldaw_core::drift::{DriftEstimator, Fit};
use hilldaw_core::drums::DrumPattern;
use export::{ClipSnap, Converter, ExportReport, Job, OutFile, Snapshot, StripSnap};
use fm1link::{Fm1Msg, Fm1Out};
use hilldaw_core::fm1::fx_map;
use hilldaw_core::mixer::{Click, MonoClip, Mixer, Source, Track};
use hilldaw_core::session::{Phase, RecordConfig, RecordSession};
use recorder::Recorder;
use std::collections::HashMap;
use hilldaw_core::project::{ClipModel, DrumSectionModel, ExportFormat, Project, TakeModel, TrackFxModel, MAX_DRUM_SECTIONS, MAX_SECTION_CYCLES, MAX_TRACKS};
use hilldaw_core::transport::Transport;
use hilldaw_drums::{find_preset, DrumLibrary};
use hilldaw_core::settings::{Settings, SETTINGS_KEY};
use usbout::AudioOut;
use status::{effective_input, InputSummary, Status, StatusHandle};
use std::sync::Arc;

/// Key under which the project is kept in the host's state store.
pub const STATE_KEY: &str = "hilldaw";
/// Mixer track indices: 0..8 are the audio tracks, then the drums, then a hidden metronome track.
pub const DRUMS: usize = MAX_TRACKS;
const CLICK: usize = MAX_TRACKS + 1;
/// Strips on screen: 8 tracks + drums.
pub const STRIPS: usize = MAX_TRACKS + 1;
/// What the export format row cycles through: container, bit depth of the rendered WAV, label.
pub(crate) const FORMATS: [(ExportFormat, u16, &str); 5] = [
    (ExportFormat::Wav, 24, "WAV 24-bit"),
    (ExportFormat::Wav, 16, "WAV 16-bit"),
    (ExportFormat::Flac, 24, "FLAC 24-bit"),
    (ExportFormat::Flac, 16, "FLAC 16-bit"),
    (ExportFormat::Mp3, 16, "MP3"),
];
/// How many bars the Waves screen shows across, from zoomed in to zoomed out.
pub(crate) const ZOOM_BARS: [u64; 6] = [2, 4, 8, 16, 32, 64];
pub const MIN_BPM: f64 = 40.0;
pub const MAX_BPM: f64 = 240.0;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum View {
    Mixer,
    Arrange,
    /// voice (Program Change) and FX controls of the FM-1
    Fm1,
    /// mixdown and stems to WAV
    Export,
    /// the strip picked on the Mixer: takes, effects, input level, loop and punch
    Track,
    /// every track's audio drawn over time (waveforms), the drum arrangement, the playhead and the loop/punch regions
    Waves,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TrackTab {
    Takes,
    Fx,
    Input,
    Loop,
}

impl TrackTab {
    pub(crate) const ALL: [TrackTab; 4] = [TrackTab::Takes, TrackTab::Fx, TrackTab::Input, TrackTab::Loop];
}

pub(crate) const HELP: &[(&str, &str)] = &[
    ("Everywhere", ""),
    ("START", "play / stop"),
    ("SELECT", "record on the armed track / stop"),
    ("FN + L1", "count-in on / off"),
    ("Y + B", "undo the last edit"),
    ("Y + A", "redo"),
    ("L1 / R1", "previous / next screen"),
    ("FN + Left/Right", "tempo -1 / +1  (Up/Down: +-5)"),
    ("FN + A", "tap tempo"),
    ("FN + X", "click on / off"),
    ("FN + B", "back to the start"),
    ("FN + Y", "repeat the drum arrangement on / off"),
    ("FN + L2/R2", "drum swing -/+"),
    ("FN + SELECT", "this help"),
    ("START + SELECT hold", "quit"),
    ("Mixer", ""),
    ("Left / Right", "pick a strip"),
    ("Up / Down", "level -1 / +1 dB, up to +24  (hold to repeat)"),
    ("Y + Up / Down", "level in 6 dB steps"),
    ("L2 / R2", "pan left / right"),
    ("X", "mute"),
    ("Y + X", "solo"),
    ("A", "arm the track for recording"),
    ("B", "reset level and pan"),
    ("FM-1 screen", ""),
    ("L2 / R2", "previous / next section (voice, filter, reverb ...)"),
    ("Up / Down", "voice: +-10;  effects: pick a control"),
    ("Left / Right", "voice +-1;  effects: value (Y: +-10)"),
    ("A", "effect on / off;  voice: send it"),
    ("X", "play a test note (C4)"),
    ("B", "all notes off"),
    ("Waves screen (every track over time)", ""),
    ("Left / Right", "scroll a bar  (Y: a whole page)"),
    ("L2 / R2", "zoom out / in"),
    ("Up / Down", "pick a track row"),
    ("A", "move the playhead to the left edge"),
    ("X", "follow the playhead on / off"),
    ("Track screen (the strip picked on the Mixer)", ""),
    ("L2 / R2", "tab: takes, effects, input, loop"),
    ("Up / Down", "pick a row or a take"),
    ("Left / Right", "change the value  (Y: bigger steps)"),
    ("A", "take: play this one;  loop/punch: set to the playhead bar"),
    ("X", "take: remove from the list (press twice)"),
    ("B", "back to the Mixer"),
    ("Export screen", ""),
    ("Up / Down", "pick an option"),
    ("Left / Right", "change it"),
    ("A", "export now"),
    ("B", "cancel a running export"),
    ("Arrange (drums)", ""),
    ("Up / Down", "pick a section"),
    ("Left / Right", "previous / next pattern"),
    ("L2 / R2", "previous / next style"),
    ("Y + Left/Right", "cycles -/+"),
    ("A", "add a section after this one"),
    ("X", "delete the section"),
];

/// The project as it was before an edit, with a name for the toast.
#[derive(Clone)]
struct UndoEntry {
    project: Project,
    label: &'static str,
}

/// A snapshot taken before an edit, kept only if the edit really changed something.
struct Pending {
    project: Project,
    label: &'static str,
    key: u32,
}

/// Edits with this key never merge into the previous undo step.
const NO_COALESCE: u32 = u32::MAX;
/// Repeated edits with the same key inside this long form one undo step (holding a button to change a level).
const COALESCE_MS: u64 = 700;
const MAX_UNDO: usize = 100;

/// A recording in progress (count-in or recording).
pub(crate) struct ActiveRec {
    session: RecordSession,
    track: usize,
    /// the capture file, relative to the project folder
    file: String,
    /// the clock time (seconds) at which the capture stream started
    t_capture: f64,
    /// capture index that corresponds to the start of recording (known once the count-in is over)
    cap_base: Option<u64>,
    /// the count-in click, running on its own clock because the transport does not move during the count-in
    click: Click,
    click_pos: u64,
    /// transport frames recorded so far (for the display)
    pub(crate) elapsed: u64,
    latency: u64,
}

impl ActiveRec {
    pub(crate) fn counting_in(&self) -> bool {
        self.session.phase() == Phase::CountIn
    }
}

pub struct HillDaw {
    lib: DrumLibrary,
    status_src: StatusHandle,
    pub(crate) status: Status,
    pub(crate) settings: Settings,
    /// the input the meter is currently listening to (to notice when the automatic choice changes)
    monitored: Option<String>,
    pub project: Project,
    pub(crate) mixer: Mixer,
    pub(crate) transport: Transport,
    rate: u32,
    spans: Vec<(u64, usize)>,
    /// how fast the sound card really consumes frames, against the system clock (see `core/src/drift.rs`)
    out_clock: DriftEstimator,
    clock0: Option<std::time::Instant>,
    out_frames: u64,
    out_last_push: f64,
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
    fm1_out: Option<Box<dyn Fm1Out>>,
    recorder: Option<Box<dyn Recorder>>,
    audio_out: Option<Box<dyn AudioOut>>,
    /// the USB output is streaming right now (the built-in output is muted)
    out_usb: bool,
    /// the output the app tried to start and failed on (shown on screen), and when it last tried
    pub(crate) out_err: Option<String>,
    out_retry_ms: u64,
    export_job: Option<Job>,
    /// the Waves screen: the timeline frame at its left edge (always a whole bar), the zoom (index into `ZOOM_BARS`), and whether it follows the playhead
    pub(crate) waves_start: u64,
    pub(crate) waves_zoom: usize,
    pub(crate) waves_follow: bool,
    /// waveform overviews of the loaded recordings, by file
    pub(crate) peak_cache: HashMap<String, Arc<Vec<(f32, f32)>>>,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    pending: Option<Pending>,
    /// the key and time of the last committed edit, to merge a burst of the same edit
    last_commit: Option<(u32, u64)>,
    coalesce_touch: Option<u32>,
    pub(crate) track_tab: TrackTab,
    pub(crate) track_row: usize,
    pub(crate) take_sel: usize,
    /// a take waiting for a second X: (track, take, ms left)
    confirm_delete: Option<(usize, usize, u32)>,
    /// the FM-1 input meter (see `monitor_tick`)
    monitoring: bool,
    monitor_block_ms: u32,
    pub(crate) monitor_err: Option<String>,
    pub(crate) in_level: f32,
    pub(crate) in_hold: f32,
    in_hold_ms: u32,
    pub(crate) clip_ms: u32,
    /// the last loop region that was switched off, to bring back
    loop_memo: Option<(u64, u64)>,
    converter: Converter,
    pub(crate) export_report: Option<Result<ExportReport, String>>,
    pub(crate) export_row: usize,
    pub(crate) rec: Option<ActiveRec>,
    /// recordings already loaded for playback, by file
    audio_cache: HashMap<String, Arc<Vec<f32>>>,
    /// the clock the audio callback runs on (seconds); tests set it by hand
    test_clock: Option<f64>,
    last_audio_s: f64,
    /// 0 = voice, 1.. = the effects in the map's order
    pub(crate) fm1_tab: usize,
    pub(crate) fm1_row: usize,
    fm1_was_up: bool,
    /// a test note that is sounding: (note, ms left)
    audition: Option<(u8, u32)>,

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

/// One step of a filter cutoff in Hz (0 = off). Steps are multiplicative so they feel even across the range; a low-pass turns off above
/// `hi`, a high-pass below `lo`.
pub fn step_cutoff(cur: u32, dir: i32, lo: u32, hi: u32, big: bool, lowpass: bool) -> u32 {
    let mult = if big { 1.5 } else { 1.12 };
    if cur == 0 {
        // off: a low-pass comes in from the top (turning it down), a high-pass from the bottom (turning it up)
        return match (lowpass, dir < 0) {
            (true, true) => hi,
            (false, false) => lo,
            _ => 0,
        };
    }
    let next = (cur as f64 * if dir > 0 { mult } else { 1.0 / mult }).round() as u32;
    let next = if next == cur { (cur as i32 + dir) as u32 } else { next };
    // past the end of its range a filter switches off
    if (lowpass && next > hi) || (!lowpass && next < lo) { 0 } else { next.clamp(lo, hi) }
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub(crate) fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-6).log10()
}

/// Top of a strip's level range in dB (see `MAX_STRIP_GAIN`).
pub(crate) const MAX_DB: f32 = 24.0;

/// One step of `db` decibels up or down from `g`; the bottom of the range is silence.
fn step_gain(g: f32, dir: i32, db: i32) -> f32 {
    if g <= 0.0 {
        return if dir > 0 { db_to_gain(-60.0 + db as f32) } else { 0.0 };
    }
    let next = (gain_to_db(g).round() + (dir * db) as f32).clamp(-60.0, MAX_DB);
    if next <= -60.0 { 0.0 } else { db_to_gain(next) }
}

impl HillDaw {
    pub fn new(host: Box<dyn Host>, status: StatusHandle) -> Self {
        let mut lib = DrumLibrary::new(host);
        let mut toast = Toast::default();
        let settings = lib.host_mut().load_state(SETTINGS_KEY).map(|t| Settings::from_json(&t)).unwrap_or_default();
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
            settings,
            monitored: None,
            project,
            mixer: Mixer::new(rate, 1024),
            transport: Transport::new(rate),
            rate,
            spans: Vec::new(),
            out_clock: DriftEstimator::new(rate as f64),
            clock0: None,
            out_frames: 0,
            out_last_push: 0.0,
            hold: [0.0; STRIPS],
            hold_master: 0.0,
            meter: [0.0; STRIPS],
            meter_master: 0.0,
            view: View::Mixer,
            sel: 0,
            armed: [false; MAX_TRACKS],
            click_on: false,
            arr_sel: 0,
            fm1_out: None,
            recorder: None,
            audio_out: None,
            out_usb: false,
            out_err: None,
            out_retry_ms: 0,
            export_job: None,
            waves_start: 0,
            waves_zoom: 2,
            waves_follow: true,
            peak_cache: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
            last_commit: None,
            coalesce_touch: None,
            track_tab: TrackTab::Takes,
            track_row: 0,
            take_sel: 0,
            confirm_delete: None,
            monitoring: false,
            monitor_block_ms: 0,
            monitor_err: None,
            in_level: 0.0,
            in_hold: 0.0,
            in_hold_ms: 0,
            clip_ms: 0,
            loop_memo: None,
            converter: Arc::new(encode::convert),
            export_report: None,
            export_row: 0,
            rec: None,
            audio_cache: HashMap::new(),
            test_clock: None,
            last_audio_s: 0.0,
            fm1_tab: 0,
            fm1_row: 0,
            fm1_was_up: false,
            audition: None,
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
        app.apply_all_fx();
        app
    }

    /// Connect the FM-1 screen to a MIDI link (the SDL program gives the USB one, tests a recording one).
    pub fn with_fm1(mut self, out: Box<dyn Fm1Out>) -> Self {
        self.fm1_out = Some(out);
        self
    }

    /// Connect recording: the FM-1 capture and the place recordings are stored and read back from.
    pub fn with_recorder(mut self, r: Box<dyn Recorder>) -> Self {
        self.recorder = Some(r);
        if let Some(r) = self.recorder.as_mut() {
            r.select_input(self.settings.input_device.clone());
        }
        for i in 0..MAX_TRACKS {
            self.rebuild_track(i);
        }
        self
    }

    /// Connect USB audio playback (the SDL program gives the real one, tests a fake). Starts the saved choice if its device is attached.
    pub fn with_audio_out(mut self, o: Box<dyn AudioOut>) -> Self {
        self.audio_out = Some(o);
        self.start_output();
        self
    }

    /// (Re)start the chosen USB output, or stop it when the choice is the built-in output.
    fn start_output(&mut self) {
        let id = self.settings.output_device.clone();
        let attached = id.as_ref().is_none_or(|i| self.status.outputs.iter().any(|o| o.id == *i));
        let Some(o) = self.audio_out.as_mut() else { return };
        self.out_retry_ms = self.ms;
        if id.is_some() && !attached {
            let _ = o.select(None);
            self.out_usb = false;
            self.out_err = None; // not plugged in: the screen says so
            return;
        }
        match o.select(id) {
            Ok(()) => self.out_err = None,
            Err(e) => {
                self.out_err = Some(e);
                let _ = o.select(None);
            }
        }
        self.out_usb = false;
    }

    fn fresh_project() -> Project {
        let mut p = Project { name: "SONG".into(), bpm: 100.0, latency_offset_frames: 563, ..Project::default() };
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
        self.apply_loop();
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
        match (gain, pan, mute, solo) {
            (Some(_), Some(_), _, _) => self.checkpoint("reset", 10_000 + i as u32),
            (Some(_), ..) => self.checkpoint("level", 1000 + i as u32),
            (_, Some(_), ..) => self.checkpoint("pan", 1100 + i as u32),
            (_, _, Some(_), _) => self.checkpoint("mute", 1200 + i as u32),
            _ => self.checkpoint("solo", 1300 + i as u32),
        }
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
        self.checkpoint("tempo", 2000);
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
        if self.rec.is_some() {
            self.stop_recording(); // START stops everything
        }
        self.transport.playing = !self.transport.playing;
        if !self.transport.playing {
            // a stopped transport renders nothing; let the meters fall
            self.hold = [0.0; STRIPS];
            self.hold_master = 0.0;
        }
    }

    // ---- undo / redo --------------------------------------------------------------------------------------------------
    //
    // Whole-project snapshots: every edit calls `checkpoint` before it changes anything; the snapshot is kept only if the project really
    // changed by the end of the button press (`settle`). Edits with the same key within `COALESCE_MS` share one step.

    fn checkpoint(&mut self, label: &'static str, key: u32) {
        if self.pending.is_some() {
            return; // one step per button press: the first snapshot is the one that matters
        }
        if key != NO_COALESCE && self.last_commit.is_some_and(|(k, t)| k == key && self.ms.saturating_sub(t) <= COALESCE_MS) {
            self.coalesce_touch = Some(key);
            return;
        }
        self.pending = Some(Pending { project: self.project.clone(), label, key });
    }

    /// Keep or drop the snapshot of the edit that just happened.
    fn settle(&mut self) {
        if let Some(p) = self.pending.take() {
            if p.project != self.project {
                self.undo.push(UndoEntry { project: p.project, label: p.label });
                if self.undo.len() > MAX_UNDO {
                    self.undo.remove(0);
                }
                self.redo.clear();
                self.last_commit = Some((p.key, self.ms));
            }
        } else if let Some(k) = self.coalesce_touch.take() {
            self.last_commit = Some((k, self.ms));
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.pending.as_ref().is_some_and(|p| p.project != self.project)
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// Go back one edit.
    pub fn undo(&mut self) {
        self.settle();
        if self.rec.is_some() {
            self.toast.say("Stop recording first");
            return;
        }
        let Some(e) = self.undo.pop() else {
            self.toast.say("Nothing to undo");
            return;
        };
        self.redo.push(UndoEntry { project: self.project.clone(), label: e.label });
        self.toast.say(&format!("Undo: {}", e.label));
        self.restore(e.project);
        self.last_commit = None;
    }

    /// Go forward again after an undo.
    pub fn redo(&mut self) {
        self.settle();
        if self.rec.is_some() {
            self.toast.say("Stop recording first");
            return;
        }
        let Some(e) = self.redo.pop() else {
            self.toast.say("Nothing to redo");
            return;
        };
        self.undo.push(UndoEntry { project: self.project.clone(), label: e.label });
        self.toast.say(&format!("Redo: {}", e.label));
        self.restore(e.project);
        self.last_commit = None;
    }

    /// Make `new` the project and bring everything that is built from it (the click, the drums, the mixer, effects, clips) into line.
    fn restore(&mut self, mut new: Project) {
        new.export = self.project.export.clone(); // export options are settings, not edits: undo leaves them as they are
        let old = std::mem::replace(&mut self.project, new);
        if old.bpm != self.project.bpm || old.beats_per_bar != self.project.beats_per_bar {
            // keep the musical position when the tempo comes back
            let beats = self.transport.pos as f64 / self.transport.frames_per_beat();
            self.transport.bpm = self.project.bpm;
            self.transport.beats_per_bar = self.project.beats_per_bar;
            self.transport.pos = self.transport.beat_to_frame(beats);
        }
        self.rebuild_click();
        self.rebuild_drums();
        self.apply_mixer();
        self.apply_all_fx();
        self.apply_loop();
        for i in 0..MAX_TRACKS {
            self.rebuild_track(i);
        }
        let strip_takes = self.project.tracks.get(self.sel).map_or(0, |t| t.takes.len());
        self.take_sel = self.take_sel.min(strip_takes.saturating_sub(1));
        // the FM-1 keeps whatever it was last told: send what this change put back
        let n = self.project.fm1.clone();
        let o = &old.fm1;
        if n.program_set && (!o.program_set || o.program != n.program) {
            self.fm1_send(Fm1Msg::Program(n.program));
        }
        let changed: Vec<(u8, u8)> = n.fx.iter().filter(|(cc, v)| o.fx.get(cc) != Some(v)).map(|(a, b)| (*a, *b)).collect();
        for (cc, value) in changed {
            self.fm1_send(Fm1Msg::FxCc { cc, value });
        }
    }

    // ---- recording ----------------------------------------------------------------------------------------------------

    /// The clock the audio callback runs on, in seconds (tests set it by hand).
    fn now_s(&self) -> f64 {
        self.test_clock.unwrap_or_else(|| self.clock0.map_or(0.0, |c| c.elapsed().as_secs_f64()))
    }

    /// SELECT: stop the recording in progress, or start one on the armed track.
    fn toggle_record(&mut self) {
        if self.rec.is_some() {
            self.stop_recording();
            return;
        }
        if self.recorder.is_none() {
            self.toast.say("Recording needs the FM-1 recorder");
            return;
        }
        let armed: Vec<usize> = (0..MAX_TRACKS).filter(|i| self.armed[*i]).collect();
        let track = match armed.as_slice() {
            [t] => *t,
            [] => {
                self.toast.say("Arm a track first (A on the mixer)");
                return;
            }
            _ => {
                self.toast.say("Arm only one track");
                return;
            }
        };
        let file = match self.recorder.as_mut().map(|r| r.start()) {
            Some(Ok(f)) => f,
            Some(Err(e)) => {
                self.toast.say(&hill_core::fb::clip(&e, 28));
                return;
            }
            None => return,
        };
        // the capture stream is running from about now: every capture index is counted from this moment
        let t_capture = self.now_s();
        let bpb = self.project.beats_per_bar.max(1);
        let count_in = if self.project.count_in { (bpb as f64 * self.transport.frames_per_beat()).round() as u64 } else { 0 };
        let latency = self.project.latency_offset_frames.max(0) as u64;
        let mut session = RecordSession::new(RecordConfig { count_in_frames: count_in, latency_frames: latency, punch: self.project.punch, ..Default::default() });
        session.begin();
        self.rec = Some(ActiveRec {
            session,
            track,
            file,
            t_capture,
            cap_base: None,
            click: Click { bpm: self.project.bpm, beats_per_bar: bpb as u64, amp: 0.6, enabled: true },
            click_pos: 0,
            elapsed: 0,
            latency,
        });
        self.monitoring = false; // starting the recording took over from monitoring
        self.transport.playing = true;
        self.toast.say(if count_in > 0 { "Count-in" } else { "Recording" });
    }

    /// Finish the capture and turn what was recorded into clips on the track. The capture file itself is never changed: each take is a
    /// trimmed view of it (offset + length) placed on the timeline, so a bad take can be dropped or re-trimmed later.
    fn stop_recording(&mut self) {
        let Some(mut rec) = self.rec.take() else { return };
        let (takes, need) = rec.session.stop();
        let base = rec.cap_base.unwrap_or(0);
        let Some(recorder) = self.recorder.as_mut() else { return };
        let info = match recorder.finish(base + need) {
            Ok(i) => i,
            Err(e) => {
                self.toast.say(&hill_core::fb::clip(&e, 28));
                return;
            }
        };
        let track = rec.track;
        self.checkpoint("recording", NO_COALESCE);
        let mut added = 0;
        for t in &takes {
            let offset = base + t.cap_start;
            // a take can never be longer than what actually reached the disk
            let frames = t.frames.min(info.frames.saturating_sub(offset));
            if frames == 0 {
                continue;
            }
            let tm = &mut self.project.tracks[track];
            tm.takes.push(TakeModel { file: rec.file.clone(), frames, latency_comp_frames: rec.latency as i64, start: t.timeline_start, offset, placed: true });
            tm.active_take = Some(tm.takes.len() - 1);
            // only the active (latest) take plays; the others stay in the take history
            tm.clips = vec![ClipModel { file: rec.file.clone(), start: t.timeline_start, offset, len: Some(frames), ..Default::default() }];
            added += 1;
        }
        self.audio_cache.remove(&rec.file);
        self.peak_cache.remove(&rec.file);
        self.rebuild_track(track);
        self.save(); // the take is in the project before anything can go wrong
        if added == 0 {
            self.toast.say("Nothing recorded");
        } else if info.overruns > 0 {
            self.toast.say(&format!("Recorded, {} chunks dropped", info.overruns));
        } else {
            self.toast.say(&format!("Recorded {added} take{}", if added == 1 { "" } else { "s" }));
        }
    }

    /// A recording's audio, loaded once and shared.
    fn clip_data(&mut self, file: &str) -> Option<Arc<Vec<f32>>> {
        if let Some(d) = self.audio_cache.get(file) {
            return Some(d.clone());
        }
        let a = Arc::new(self.recorder.as_mut()?.load(file)?);
        self.peak_cache.insert(file.to_string(), Arc::new(hilldaw_core::waveform::peaks(&a)));
        self.audio_cache.insert(file.to_string(), a.clone());
        Some(a)
    }

    /// Rebuild a track's playback from the clips in the project (loading each recording once).
    fn rebuild_track(&mut self, i: usize) {
        let clips = self.project.tracks[i].clips.clone();
        let mut sources: Vec<Box<dyn Source>> = Vec::new();
        for c in &clips {
            let Some(data) = self.clip_data(&c.file) else {
                self.toast.say("A recording is missing");
                continue;
            };
            let len = c.len.unwrap_or_else(|| (data.len() as u64).saturating_sub(c.offset));
            sources.push(Box::new(MonoClip { data, start: c.start, offset: c.offset, len, gain: c.gain, fade_in: c.fade_in, fade_out: c.fade_out, reverse: c.reverse }));
        }
        self.mixer.tracks[i].sources = sources;
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
            View::Fm1 => self.fm1_act(b),
            View::Export => self.export_act(b),
            View::Track => self.track_act(b),
            View::Waves => self.waves_act(b),
        }
    }

    fn fn_act(&mut self, b: Btn) {
        match b {
            Btn::Left => self.set_bpm(self.project.bpm - 1.0),
            Btn::Right => self.set_bpm(self.project.bpm + 1.0),
            Btn::Down => self.set_bpm(self.project.bpm - 5.0),
            Btn::Up => self.set_bpm(self.project.bpm + 5.0),
            Btn::L2 | Btn::R2 => {
                self.checkpoint("swing", 2100);
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
            Btn::Y => {
                self.checkpoint("drum loop", NO_COALESCE);
                self.project.drums.loop_arrangement = !self.project.drums.loop_arrangement;
                self.rebuild_drums();
                self.toast.say(if self.project.drums.loop_arrangement { "Drums loop on" } else { "Drums loop off" });
            }
            _ => {}
        }
    }

    /// The transport only wraps inside a loop region the user has set (none by default); the drum arrangement repeats on its own
    /// (`DrumTrack::looping`), so the bar counter keeps counting while the groove goes round and a long take stays one take.
    pub(crate) fn apply_loop(&mut self) {
        self.transport.loop_region = self.project.loop_region.filter(|(s, e)| e > s);
    }

    /// Set or clear the loop region (frames). Recording over a loop region makes each pass its own take.
    pub fn set_loop_region(&mut self, r: Option<(u64, u64)>) {
        self.checkpoint("loop", 6000 + self.track_row as u32);
        self.project.loop_region = r;
        self.apply_loop();
    }

    fn mixer_act(&mut self, b: Btn) {
        let i = self.sel;
        match b {
            Btn::Left => self.sel = i.saturating_sub(1),
            Btn::Right => self.sel = (i + 1).min(STRIPS - 1),
            Btn::Up | Btn::Down => {
                // Y held: 6 dB at a time, to get across the +-24 dB range quickly
                let g = step_gain(self.strip_gain(i), if b == Btn::Up { 1 } else { -1 }, if self.held.is(Btn::Y) { 6 } else { 1 });
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
                self.checkpoint("cycles", 3100 + self.arr_sel as u32);
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
                    self.checkpoint("add section", NO_COALESCE);
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
                self.checkpoint("delete section", NO_COALESCE);
                self.project.drums.sections.remove(self.arr_sel);
                self.rebuild_drums();
                self.toast.say("Section deleted");
            }
            _ => {}
        }
    }

    // ---- effects ----------------------------------------------------------------------------------------------------

    fn strip_fx(&self, i: usize) -> &TrackFxModel {
        if i == DRUMS { &self.project.drums.fx } else { &self.project.tracks[i].fx }
    }

    fn strip_fx_mut(&mut self, i: usize) -> &mut TrackFxModel {
        if i == DRUMS { &mut self.project.drums.fx } else { &mut self.project.tracks[i].fx }
    }

    /// Rebuild one strip's effect chain from the project (a filter's memory restarts; fine for an edit) and the shared reverb.
    fn apply_fx(&mut self, i: usize) {
        let rate = self.rate;
        let m = self.strip_fx(i).clone();
        m.apply(&mut self.mixer.tracks[i].fx, rate);
        hilldaw_core::effects::sync_reverb(&mut self.mixer, self.project.reverb_room);
    }

    fn apply_all_fx(&mut self) {
        for i in 0..STRIPS {
            self.apply_fx(i);
        }
    }

    // ---- the Waves screen ----------------------------------------------------------------------------------------------

    pub(crate) fn waves_span(&self) -> u64 {
        ZOOM_BARS[self.waves_zoom.min(ZOOM_BARS.len() - 1)] * self.bar_frames()
    }

    /// The last frame anything starts or sounds on the timeline: the longest clip, or one pass of the drum arrangement.
    pub(crate) fn song_end(&self) -> u64 {
        let clips = self.project.tracks.iter().flat_map(|t| t.clips.iter()).map(|c| c.start + c.len.unwrap_or(0)).max().unwrap_or(0);
        let d = &self.project.drums;
        let drums = if d.enabled && !d.sections.is_empty() { self.drum_total } else { 0 };
        clips.max(drums)
    }

    fn waves_act(&mut self, b: Btn) {
        let bar = self.bar_frames();
        let span = self.waves_span();
        match b {
            Btn::Up => self.sel = self.sel.saturating_sub(1),
            Btn::Down => self.sel = (self.sel + 1).min(STRIPS - 1),
            Btn::Left | Btn::Right => {
                let step = if self.held.is(Btn::Y) { span } else { bar };
                let last = (self.song_end() / bar) * bar; // no scrolling off into nothing
                self.waves_start = if b == Btn::Left { self.waves_start.saturating_sub(step) } else { (self.waves_start + step).min(last.max(self.waves_start)) };
                self.waves_start = (self.waves_start / bar) * bar;
                if self.waves_follow {
                    self.waves_follow = false;
                    self.toast.say("Follow off (X)");
                }
            }
            Btn::L2 => self.waves_zoom = (self.waves_zoom + 1).min(ZOOM_BARS.len() - 1),
            Btn::R2 => self.waves_zoom = self.waves_zoom.saturating_sub(1),
            Btn::A => {
                self.transport.pos = self.waves_start;
                self.toast.say("Playhead moved here");
            }
            Btn::X => {
                self.waves_follow = !self.waves_follow;
                self.toast.say(if self.waves_follow { "Follow on" } else { "Follow off" });
            }
            Btn::B => self.view = View::Mixer,
            _ => {}
        }
    }

    /// While following, turn the page when the playhead leaves the window (the window starts on a multiple of its own width).
    fn waves_follow_tick(&mut self) {
        if self.waves_follow && self.transport.playing {
            let span = self.waves_span();
            if span > 0 && (self.transport.pos < self.waves_start || self.transport.pos >= self.waves_start + span) {
                self.waves_start = (self.transport.pos / span) * span;
            }
        }
    }

    // ---- the Track screen -----------------------------------------------------------------------------------------------

    pub(crate) fn bar_frames(&self) -> u64 {
        ((self.project.beats_per_bar.max(1) as f64) * self.transport.frames_per_beat()).round().max(1.0) as u64
    }

    /// Make a take from the history the one that plays (its clip is rebuilt from where it was recorded).
    pub fn set_active_take(&mut self, track: usize, idx: usize) -> Result<(), String> {
        let t = self.project.tracks.get(track).ok_or("no such track")?.takes.get(idx).ok_or("no such take")?.clone();
        if !t.placed {
            return Err("older take: its place was not saved".into());
        }
        self.checkpoint("take", NO_COALESCE);
        let tm = &mut self.project.tracks[track];
        tm.active_take = Some(idx);
        tm.clips = vec![ClipModel { file: t.file, start: t.start, offset: t.offset, len: Some(t.frames), ..Default::default() }];
        self.rebuild_track(track);
        Ok(())
    }

    /// Take `idx` leaves the list (its audio stays on the card). If it was playing, the newest remaining take plays instead.
    pub fn remove_take(&mut self, track: usize, idx: usize) {
        if self.project.tracks.get(track).is_none_or(|t| idx >= t.takes.len()) {
            return;
        }
        self.checkpoint("remove take", NO_COALESCE);
        let tm = &mut self.project.tracks[track];
        tm.takes.remove(idx);
        match tm.active_take {
            Some(a) if a == idx => {
                tm.active_take = None;
                tm.clips.clear();
                if let Some(last) = tm.takes.iter().rposition(|t| t.placed) {
                    let _ = self.set_active_take(track, last);
                } else {
                    self.rebuild_track(track);
                }
            }
            Some(a) if a > idx => tm.active_take = Some(a - 1),
            _ => {}
        }
        self.take_sel = self.take_sel.min(self.project.tracks[track].takes.len().saturating_sub(1));
    }

    fn track_act(&mut self, b: Btn) {
        let big = self.held.is(Btn::Y);
        match b {
            Btn::B => {
                self.view = View::Mixer;
                return;
            }
            Btn::L2 | Btn::R2 => {
                let i = TrackTab::ALL.iter().position(|t| *t == self.track_tab).unwrap_or(0);
                let n = if b == Btn::R2 { (i + 1).min(3) } else { i.saturating_sub(1) };
                self.track_tab = TrackTab::ALL[n];
                self.track_row = 0;
                self.confirm_delete = None;
                return;
            }
            _ => {}
        }
        let strip = self.sel;
        match self.track_tab {
            TrackTab::Takes => self.takes_act(b, strip),
            TrackTab::Fx => {
                self.track_row = self.track_row.min(6);
                match b {
                    Btn::Up => self.track_row = self.track_row.saturating_sub(1),
                    Btn::Down => self.track_row = (self.track_row + 1).min(6),
                    Btn::Left | Btn::Right => self.fx_edit(strip, if b == Btn::Right { 1 } else { -1 }, big),
                    _ => {}
                }
            }
            TrackTab::Input => {
                self.track_row = self.track_row.min(3);
                match b {
                    Btn::Up => self.track_row = self.track_row.saturating_sub(1),
                    Btn::Down => self.track_row = (self.track_row + 1).min(3),
                    Btn::Left | Btn::Right if self.track_row == 0 => self.change_input(if b == Btn::Right { 1 } else { -1 }),
                    Btn::Left | Btn::Right if self.track_row == 3 => self.change_output(if b == Btn::Right { 1 } else { -1 }),
                    Btn::Left | Btn::Right => {
                        let dir = if b == Btn::Right { 1i64 } else { -1 };
                        if self.track_row == 1 {
                            // about 1 ms (44 frames) a step, 5 ms with Y; never earlier than the capture (0) or later than 100 ms
                            self.checkpoint("latency", 7000);
                            let step = if big { 220 } else { 44 };
                            self.project.latency_offset_frames = (self.project.latency_offset_frames + dir * step).clamp(0, 4410);
                        } else {
                            self.checkpoint("count-in", 7001);
                            self.project.count_in = !self.project.count_in;
                        }
                    }
                    Btn::A if self.track_row == 2 => {
                        self.checkpoint("count-in", 7001);
                        self.project.count_in = !self.project.count_in;
                    }
                    _ => {}
                }
            }
            TrackTab::Loop => self.loop_act(b, big),
        }
    }

    /// The input in use right now (the chosen one, or the automatic pick), if it is attached.
    pub(crate) fn current_input(&self) -> Option<&InputSummary> {
        effective_input(&self.status.inputs, &self.settings.input_device)
    }

    /// Step the input choice through Auto and every attached device, remember it, tell the recorder, and restart the meter on the new one.
    fn change_input(&mut self, dir: i32) {
        if self.rec.is_some() {
            self.toast.say("Stop recording first");
            return;
        }
        let mut opts: Vec<Option<String>> = vec![None];
        opts.extend(self.status.inputs.iter().map(|i| Some(i.id.clone())));
        // a chosen device that is unplugged is not in the list: Right goes to the first device, Left to Auto
        let cur = opts.iter().position(|o| *o == self.settings.input_device).unwrap_or(if dir > 0 { 0 } else { 1 });
        let next = (cur as i32 + dir).rem_euclid(opts.len() as i32) as usize;
        self.settings.input_device = opts[next].clone();
        let json = self.settings.to_json();
        self.lib.host_mut().save_state(SETTINGS_KEY, &json);
        if let Some(r) = self.recorder.as_mut() {
            r.select_input(self.settings.input_device.clone());
            if self.monitoring {
                let _ = r.monitor(false); // the meter reopens on the new input
                self.monitoring = false;
            }
        }
        self.monitor_err = None;
        self.monitor_block_ms = 0;
        self.in_hold = 0.0;
        let name = self.current_input().map_or_else(|| "none found".to_string(), |i| i.name.clone());
        self.toast.say(&hill_core::fb::clip(&format!("Input: {}", if self.settings.input_device.is_none() { format!("auto ({name})") } else { name }), 30));
    }

    /// The output in use: the chosen USB device if it is attached and streaming, else the built-in output.
    pub fn output_name(&self) -> Option<String> {
        self.audio_out.as_ref().and_then(|o| o.active())
    }

    /// The chosen output id, or `None` for the built-in output.
    pub fn selected_output(&self) -> Option<&str> {
        self.settings.output_device.as_deref()
    }

    /// Step the output through the built-in output and every attached USB output, remember it, and start it.
    fn change_output(&mut self, dir: i32) {
        if self.audio_out.is_none() {
            self.toast.say("No USB output here");
            return;
        }
        let mut opts: Vec<Option<String>> = vec![None];
        opts.extend(self.status.outputs.iter().map(|i| Some(i.id.clone())));
        let cur = opts.iter().position(|o| *o == self.settings.output_device).unwrap_or(if dir > 0 { 0 } else { 1 });
        let next = (cur as i32 + dir).rem_euclid(opts.len() as i32) as usize;
        self.settings.output_device = opts[next].clone();
        let json = self.settings.to_json();
        self.lib.host_mut().save_state(SETTINGS_KEY, &json);
        self.start_output();
        let msg = match (&self.settings.output_device, &self.out_err) {
            (None, _) => "Output: built-in".to_string(),
            (Some(_), Some(e)) => format!("Output failed: {e}"),
            (Some(id), None) => format!("Output: {}", self.status.outputs.iter().find(|o| o.id == *id).map_or("USB", |o| o.name.as_str())),
        };
        self.toast.say(&hill_core::fb::clip(&msg, 30));
    }

    /// If the chosen USB output has appeared (plugged in) or its stream ended (unplugged), bring it up or let the built-in output take over.
    fn output_tick(&mut self) {
        if self.audio_out.is_none() || self.settings.output_device.is_none() {
            return;
        }
        let streaming = self.audio_out.as_ref().is_some_and(|o| o.active().is_some());
        if streaming || self.ms.saturating_sub(self.out_retry_ms) < 2000 {
            return;
        }
        let id = self.settings.output_device.clone().unwrap_or_default();
        if self.status.outputs.iter().any(|o| o.id == id) {
            self.start_output();
        } else {
            self.out_retry_ms = self.ms;
        }
    }

    fn takes_act(&mut self, b: Btn, strip: usize) {
        if strip >= MAX_TRACKS {
            return; // the drums are not recorded
        }
        let n = self.project.tracks[strip].takes.len();
        self.take_sel = self.take_sel.min(n.saturating_sub(1));
        match b {
            Btn::Up => self.take_sel = self.take_sel.saturating_sub(1),
            Btn::Down => self.take_sel = (self.take_sel + 1).min(n.saturating_sub(1)),
            Btn::A if n > 0 => match self.set_active_take(strip, self.take_sel) {
                Ok(()) => self.toast.say(&format!("Take {} plays", self.take_sel + 1)),
                Err(e) => self.toast.say(&hill_core::fb::clip(&e, 28)),
            },
            Btn::X if n > 0 => {
                // two presses, so a slip does not drop a take from the list
                if self.confirm_delete.is_some_and(|(t, i, _)| t == strip && i == self.take_sel) {
                    self.confirm_delete = None;
                    self.remove_take(strip, self.take_sel);
                    self.toast.say("Take removed from the list");
                } else {
                    self.confirm_delete = Some((strip, self.take_sel, 3000));
                    self.toast.say("X again to remove (audio stays)");
                }
            }
            _ => {}
        }
    }

    /// One edit step of the effect row `self.track_row` on strip `i`.
    fn fx_edit(&mut self, i: usize, dir: i32, big: bool) {
        let row = self.track_row;
        self.checkpoint("effect", 5000 + i as u32 * 16 + row as u32);
        let q = |v: f32| (v * 100.0).round() / 100.0;
        if row == 6 {
            self.project.reverb_room = q((self.project.reverb_room + dir as f32 * 0.05).clamp(0.0, 1.0));
        } else {
            let fx = self.strip_fx_mut(i);
            match row {
                0 => fx.lowpass_hz = step_cutoff(fx.lowpass_hz, dir, 100, 20_000, big, true),
                1 => fx.highpass_hz = step_cutoff(fx.highpass_hz, dir, 20, 2_000, big, false),
                2 => fx.delay_ms = (fx.delay_ms as i32 + dir * if big { 50 } else { 10 }).clamp(10, 1000) as u32,
                3 => fx.delay_feedback = q((fx.delay_feedback + dir as f32 * 0.05).clamp(0.0, 0.9)),
                4 => fx.delay_mix = q((fx.delay_mix + dir as f32 * 0.05).clamp(0.0, 1.0)),
                _ => fx.reverb_send = q((fx.reverb_send + dir as f32 * 0.05).clamp(0.0, 1.0)),
            }
        }
        self.apply_fx(i);
    }

    fn loop_act(&mut self, b: Btn, big: bool) {
        let bar = self.bar_frames();
        self.track_row = self.track_row.min(5);
        let step = if big { 4 } else { 1 };
        // rows 0-2 are the loop (on, start bar, length in bars), rows 3-5 the punch
        let (is_loop, base) = if self.track_row < 3 { (true, 0) } else { (false, 3) };
        let cur = if is_loop { self.project.loop_region } else { self.project.punch };
        match b {
            Btn::Up => self.track_row = self.track_row.saturating_sub(1),
            Btn::Down => self.track_row = (self.track_row + 1).min(5),
            Btn::Left | Btn::Right | Btn::A => {
                let dir = match b {
                    Btn::Left => -1i64,
                    Btn::Right => 1,
                    _ => 0,
                };
                let new = match (self.track_row - base, cur) {
                    // on / off
                    (0, Some(r)) => {
                        if is_loop {
                            self.loop_memo = Some(r);
                        }
                        None
                    }
                    (0, None) => Some(if is_loop { self.loop_memo.unwrap_or((0, 4 * bar)) } else { (0, 4 * bar) }),
                    // start bar
                    (1, Some((s, e))) => {
                        let len = e - s;
                        let cur_bar = (s / bar) as i64;
                        let nb = if b == Btn::A { (self.transport.pos / bar) as i64 } else { cur_bar + dir * step };
                        let ns = (nb.max(0) as u64) * bar;
                        Some((ns, ns + len))
                    }
                    // length in bars
                    (2, Some((s, e))) => {
                        let cur_len = ((e - s) / bar).max(1) as i64;
                        let nl = if b == Btn::A { ((self.transport.pos / bar) as i64 + 1 - (s / bar) as i64).max(1) } else { (cur_len + dir * step).max(1) };
                        Some((s, s + nl as u64 * bar))
                    }
                    (_, None) => None, // the start / length rows do nothing while it is off
                    _ => cur,
                };
                if is_loop {
                    self.set_loop_region(new);
                } else {
                    self.checkpoint("punch", 6200 + self.track_row as u32);
                    self.project.punch = new;
                }
            }
            _ => {}
        }
    }

    // ---- input meter ----------------------------------------------------------------------------------------------------

    /// Keep the FM-1 capture running for the meter while a track is armed or the Input tab is open (and read its level).
    fn monitor_tick(&mut self, dt_ms: u32) {
        self.monitor_block_ms = self.monitor_block_ms.saturating_sub(dt_ms);
        self.clip_ms = self.clip_ms.saturating_sub(dt_ms);
        let wanted = self.recorder.is_some() && self.rec.is_none() && (self.armed.iter().any(|a| *a) || (self.view == View::Track && self.track_tab == TrackTab::Input));
        // the automatic choice can change while the meter runs (the FM-1 is plugged in): reopen on the new one
        let eff = self.current_input().map(|i| i.id.clone());
        if self.monitoring && self.monitored != eff {
            if let Some(r) = self.recorder.as_mut() {
                let _ = r.monitor(false);
            }
            self.monitoring = false;
        }
        if wanted != self.monitoring && (!wanted || self.monitor_block_ms == 0) {
            if let Some(r) = self.recorder.as_mut() {
                match r.monitor(wanted) {
                    Ok(()) => {
                        self.monitoring = wanted;
                        self.monitored = if wanted { eff } else { None };
                        self.monitor_err = None;
                    }
                    Err(e) => {
                        // not plugged in: say so on the Input tab and try again in a couple of seconds
                        self.monitoring = false;
                        self.monitor_err = Some(e);
                        self.monitor_block_ms = 2000;
                    }
                }
            }
        }
        let live = self.monitoring || self.rec.is_some();
        let l = if live { self.recorder.as_mut().map_or(0.0, |r| r.level()) } else { 0.0 };
        self.in_level = l.max(self.in_level * (-(dt_ms as f32) / 250.0).exp());
        if l >= self.in_hold {
            self.in_hold = l;
            self.in_hold_ms = 1500;
        } else if self.in_hold_ms > dt_ms {
            self.in_hold_ms -= dt_ms;
        } else {
            self.in_hold = l;
            self.in_hold_ms = 0;
        }
        if l >= 0.98 {
            self.clip_ms = 2000; // within 0.2 dB of full scale: the FM-1 is clipping into the recording
        }
        if !live {
            self.in_level = 0.0;
        }
        if let Some((t, i, left)) = self.confirm_delete {
            self.confirm_delete = if left > dt_ms { Some((t, i, left - dt_ms)) } else { None };
        }
    }

    // ---- export -------------------------------------------------------------------------------------------------------

    /// What an export would render now, or `None` when there is nothing to export.
    fn export_snapshot(&mut self) -> Option<Snapshot> {
        let rate = self.rate;
        let mut strips = Vec::new();
        let mut end = 0u64;
        for i in 0..MAX_TRACKS {
            let tm = self.project.tracks[i].clone();
            let mut clips = Vec::new();
            for c in &tm.clips {
                let Some(data) = self.clip_data(&c.file) else { continue };
                let len = c.len.unwrap_or_else(|| (data.len() as u64).saturating_sub(c.offset));
                end = end.max(c.start + len);
                clips.push(ClipSnap { data, start: c.start, offset: c.offset, len, gain: c.gain, fade_in: c.fade_in, fade_out: c.fade_out, reverse: c.reverse });
            }
            strips.push(StripSnap { name: tm.name.clone(), gain: tm.gain, pan: tm.pan, mute: tm.mute, solo: tm.solo, fx: tm.fx.clone(), clips });
        }
        let d = self.project.drums.clone();
        let mut drums = None;
        // the drums are left out of the export entirely when the option is off: no stem, no length, and a soloed drum strip must not
        // silence everything else
        let with_drums = self.project.export.drums;
        if with_drums && d.enabled && !d.sections.is_empty() {
            if let Ok(mut t) = self.lib.build_track(&d, self.project.bpm) {
                t.gain = 1.0; // the strip's level is the mixer track's gain
                end = end.max(t.total_frames(rate));
                drums = Some(t);
            }
        }
        strips.push(StripSnap { name: "DRUMS".into(), gain: d.gain, pan: d.pan, mute: false, solo: d.solo && with_drums, fx: d.fx.clone(), clips: vec![] });
        if end == 0 {
            return None;
        }
        // the drums repeat to cover the longest clip, then stop on the last frame; hits already sounding ring into the tail
        if let Some(t) = drums.as_mut() {
            t.end_frame = Some(end);
        }
        Some(Snapshot { rate, strips, drums, reverb_room: self.project.reverb_room, end })
    }

    /// Start exporting the mixdown (and stems if asked) on a background thread.
    pub fn start_export(&mut self) {
        if self.export_job.is_some() {
            self.toast.say("An export is already running");
            return;
        }
        if self.recorder.is_none() {
            self.toast.say("Export needs the project storage");
            return;
        }
        let Some(snap) = self.export_snapshot() else {
            self.toast.say("Nothing to export yet");
            return;
        };
        let opts = self.project.export.clone();
        let base = export::slug(&self.project.name);
        let mut names: Vec<(String, Option<usize>)> = vec![(format!("{base}-mixdown"), None)];
        if opts.stems {
            for i in 0..snap.strips.len() {
                if snap.has_audio(i) {
                    let n = if i == 8 { format!("{base}-stem-drums") } else { format!("{base}-stem-{}-{}", i + 1, export::slug(&snap.strips[i].name)) };
                    names.push((n, Some(i)));
                }
            }
        }
        let mut files = Vec::new();
        for (name, strip) in names {
            match self.recorder.as_mut().map(|r| r.create_export(&name)) {
                Some(Ok((file, path))) => files.push(OutFile { file, path, strip }),
                Some(Err(e)) => {
                    self.toast.say(&hill_core::fb::clip(&e, 28));
                    return;
                }
                None => return,
            }
        }
        self.export_report = None;
        self.export_job = Some(export::spawn(snap, opts, files, self.converter.clone()));
        self.toast.say("Exporting");
    }

    /// Replace the FLAC/MP3 converter (tests give a fake; the SP uses the encoders on the device).
    pub fn set_converter(&mut self, c: Converter) {
        self.converter = c;
    }

    /// Set a strip's effect chain from a model (strip 8 is the drums); values are pulled into range.
    pub fn set_strip_fx(&mut self, strip: usize, mut m: TrackFxModel) {
        if strip < STRIPS {
            self.checkpoint("effect", 5000 + strip as u32 * 16);
            m.sanitize();
            *self.strip_fx_mut(strip) = m;
            self.apply_fx(strip);
        }
    }

    pub fn export_running(&self) -> bool {
        self.export_job.is_some()
    }

    pub fn export_progress(&self) -> Option<f32> {
        self.export_job.as_ref().map(|j| j.fraction())
    }

    pub fn export_report(&self) -> Option<&Result<ExportReport, String>> {
        self.export_report.as_ref()
    }

    /// Collect a finished export (called every UI tick).
    fn export_poll(&mut self) {
        if !self.export_job.as_ref().is_some_and(|j| j.finished()) {
            return;
        }
        let Some(mut job) = self.export_job.take() else { return };
        let r = job.take_result();
        match &r {
            Ok(rep) if rep.cancelled => self.toast.say("Export cancelled"),
            Ok(rep) if rep.clipped > 0 => self.toast.say("Exported, but some peaks clipped"),
            Ok(_) => self.toast.say("Export finished"),
            Err(e) => self.toast.say(&hill_core::fb::clip(e, 28)),
        }
        self.export_report = Some(r);
    }

    fn export_act(&mut self, b: Btn) {
        const ROWS: usize = 6;
        match b {
            Btn::Up => self.export_row = self.export_row.saturating_sub(1),
            Btn::Down => self.export_row = (self.export_row + 1).min(ROWS - 1),
            Btn::A => self.start_export(),
            Btn::B => {
                if let Some(j) = &self.export_job {
                    j.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Btn::Left | Btn::Right => {
                let e = &mut self.project.export;
                match self.export_row {
                    0 => {
                        let i = FORMATS.iter().position(|f| (f.0, f.1) == (e.format, e.bits)).unwrap_or(0) as i32;
                        let n = FORMATS.len() as i32;
                        let (format, bits, _) = FORMATS[(i + if b == Btn::Right { 1 } else { n - 1 }).rem_euclid(n) as usize];
                        e.format = format;
                        e.bits = bits;
                    }
                    1 => e.dither = !e.dither,
                    2 => e.normalize = !e.normalize,
                    3 => e.stems = !e.stems,
                    4 => e.drums = !e.drums,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // ---- FM-1 -------------------------------------------------------------------------------------------------------

    fn fm1_send(&mut self, m: Fm1Msg) {
        if let Some(o) = self.fm1_out.as_mut() {
            o.send(m);
        }
    }

    fn fm1_set_program(&mut self, p: u8) {
        self.checkpoint("FM-1 voice", 4000);
        self.project.fm1.program = p.min(127);
        self.project.fm1.program_set = true;
        self.fm1_send(Fm1Msg::Program(self.project.fm1.program));
    }

    fn fm1_set_fx(&mut self, cc: u8, v: u8) {
        let Some(p) = fx_map().by_cc(cc) else { return };
        self.checkpoint("FM-1 effect", 4100 + cc as u32);
        let v = p.clamp(v);
        self.project.fm1.fx.insert(cc, v);
        self.fm1_send(Fm1Msg::FxCc { cc, value: v });
    }

    /// Send what the user has chosen (and nothing else) after the FM-1 appears, so connecting never overrides its own patch
    /// with invented defaults.
    fn fm1_resync(&mut self) {
        if self.project.fm1.program_set {
            self.fm1_send(Fm1Msg::Program(self.project.fm1.program));
        }
        let fx: Vec<(u8, u8)> = self.project.fm1.fx.iter().map(|(a, b)| (*a, *b)).collect();
        for (cc, value) in fx {
            self.fm1_send(Fm1Msg::FxCc { cc, value });
        }
    }

    fn fm1_audition(&mut self) {
        if let Some((n, _)) = self.audition.take() {
            self.fm1_send(Fm1Msg::NoteOff(n));
        }
        self.fm1_send(Fm1Msg::NoteOn(60));
        self.audition = Some((60, 700));
    }

    fn fm1_act(&mut self, b: Btn) {
        let map = fx_map();
        let effects = map.effects();
        match b {
            Btn::L2 => {
                self.fm1_tab = self.fm1_tab.saturating_sub(1);
                self.fm1_row = 0;
                return;
            }
            Btn::R2 => {
                self.fm1_tab = (self.fm1_tab + 1).min(effects.len());
                self.fm1_row = 0;
                return;
            }
            Btn::X => {
                self.fm1_audition();
                return;
            }
            Btn::B => {
                self.audition = None;
                self.fm1_send(Fm1Msg::Panic);
                self.toast.say("All notes off");
                return;
            }
            _ => {}
        }
        if self.fm1_tab == 0 {
            let cur = self.project.fm1.program as i32;
            let p = match b {
                Btn::Left => Some(cur - 1),
                Btn::Right => Some(cur + 1),
                Btn::Up => Some(cur + 10),
                Btn::Down => Some(cur - 10),
                Btn::A => Some(cur),
                _ => None,
            };
            if let Some(p) = p {
                self.fm1_set_program(p.clamp(0, 127) as u8);
            }
            return;
        }
        let ps = &effects[self.fm1_tab - 1].1;
        self.fm1_row = self.fm1_row.min(ps.len() - 1);
        let sel = ps[self.fm1_row];
        match b {
            Btn::Up => self.fm1_row = self.fm1_row.saturating_sub(1),
            Btn::Down => self.fm1_row = (self.fm1_row + 1).min(ps.len() - 1),
            Btn::A => {
                let on = self.project.fm1.fx.get(&ps[0].cc).copied().unwrap_or(0) > 0;
                self.fm1_set_fx(ps[0].cc, if on { 0 } else { 1 });
            }
            Btn::Left | Btn::Right => {
                let dir = if b == Btn::Left { -1 } else { 1 };
                let step = if self.held.is(Btn::Y) && sel.max > 10 { 10 } else { 1 };
                let v = match self.project.fm1.fx.get(&sel.cc).copied() {
                    // never set: a switch goes straight to its side, a parameter starts from the middle of its range
                    None if sel.is_switch() => if dir > 0 { 1 } else { 0 },
                    None => sel.from_fraction(0.5),
                    Some(cur) => (cur as i32 + dir * step).clamp(sel.min as i32, sel.max as i32) as u8,
                };
                self.fm1_set_fx(sel.cc, v);
            }
            _ => {}
        }
    }

    fn set_section_preset(&mut self, idx: usize) {
        self.checkpoint("pattern", 3000 + self.arr_sel as u32);
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
    /// The name of the input in use (`None` when nothing is attached).
    pub fn input_name(&self) -> Option<&str> {
        self.current_input().map(|i| i.name.as_str())
    }
    /// The chosen input id, or `None` for automatic.
    pub fn selected_input(&self) -> Option<&str> {
        self.settings.input_device.as_deref()
    }
    /// The last frame anything starts or sounds (the longest clip, or one pass of the drum arrangement).
    pub fn song_end_for_test(&self) -> u64 {
        self.song_end()
    }
    pub fn waves_zoom(&self) -> usize {
        self.waves_zoom
    }
    pub fn waves_start(&self) -> u64 {
        self.waves_start
    }
    pub fn waves_follow(&self) -> bool {
        self.waves_follow
    }
    pub fn track_tab(&self) -> TrackTab {
        self.track_tab
    }
    pub fn track_row(&self) -> usize {
        self.track_row
    }
    pub fn take_sel(&self) -> usize {
        self.take_sel
    }
    pub fn clip_flash(&self) -> bool {
        self.clip_ms > 0
    }
    pub fn input_level(&self) -> f32 {
        self.in_level
    }
    pub fn input_hold(&self) -> f32 {
        self.in_hold
    }
    pub fn monitor_error(&self) -> Option<&str> {
        self.monitor_err.as_deref()
    }
    pub fn fm1_tab(&self) -> usize {
        self.fm1_tab
    }
    pub fn fm1_row(&self) -> usize {
        self.fm1_row
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

impl HillDaw {
    /// The audio callback with the time given by the caller (tests drive it with a pretend clock).
    pub fn audio_at(&mut self, out: &mut [f32], t_s: f64) {
        self.last_audio_s = t_s;
        self.out_frames += (out.len() / 2) as u64;
        if t_s - self.out_last_push >= 0.2 {
            self.out_last_push = t_s;
            self.out_clock.push(t_s, self.out_frames);
        }
        out.fill(0.0);
        let frames = out.len() / 2;
        let rate = self.rate;
        // count-in: only the click sounds and the transport does not move; recording begins where it ends, possibly mid-chunk
        let mut lead = 0usize;
        if let Some(rec) = self.rec.as_mut() {
            if rec.session.phase() == Phase::CountIn {
                let used = rec.session.tick_count_in(frames as u64) as usize;
                rec.click.render_add(rec.click_pos, &mut out[..used * 2], rate);
                rec.click_pos += used as u64;
                lead = used;
            }
            if rec.session.phase() == Phase::Recording && rec.cap_base.is_none() {
                // the capture index that lines up with the first recorded frame, from the two streams' common clock
                let t = t_s + lead as f64 / rate as f64;
                rec.cap_base = Some(((t - rec.t_capture).max(0.0) * recorder::RATE as f64).round() as u64);
            }
        }
        let mut spans = std::mem::take(&mut self.spans);
        self.transport.advance(frames - lead, &mut spans);
        let mut at = lead;
        for &(start, len) in &spans {
            self.mixer.render(start, &mut out[at * 2..(at + len) * 2]);
            for i in 0..STRIPS {
                self.hold[i] = self.hold[i].max(self.mixer.tracks[i].peak);
            }
            self.hold_master = self.hold_master.max(self.mixer.master_peak);
            if let Some(rec) = self.rec.as_mut() {
                rec.session.on_span(start, len as u64);
                rec.elapsed += len as u64;
            }
            at += len;
        }
        self.spans = spans;
        // a USB output takes the finished block and the built-in output goes quiet; if it is gone the block plays here as usual
        if let Some(o) = self.audio_out.as_mut() {
            self.out_usb = o.push(out);
            if self.out_usb {
                out.fill(0.0);
            }
        }
    }

    /// Test hook: the clock recordings are aligned on, instead of the real one.
    pub fn set_test_clock(&mut self, t: Option<f64>) {
        self.test_clock = t;
    }

    /// Whether the FM-1 input is being measured right now (armed, on the Input tab, or recording).
    pub(crate) fn input_live(&self) -> bool {
        self.monitoring || self.rec.is_some()
    }

    pub(crate) fn has_recorder(&self) -> bool {
        self.recorder.is_some()
    }

    pub fn transport_loop(&self) -> Option<(u64, u64)> {
        self.transport.loop_region
    }

    pub fn is_recording(&self) -> bool {
        self.rec.is_some()
    }

    pub fn is_counting_in(&self) -> bool {
        self.rec.as_ref().is_some_and(|r| r.counting_in())
    }

    /// Measured output clock after at least 30 s of running (`None` before).
    pub fn out_clock(&self) -> Option<Fit> {
        self.out_clock.fit(30.0)
    }

    fn tick_inner(&mut self, dt_ms: u32) {
        self.ms += dt_ms as u64;
        self.toast.tick(dt_ms);
        if self.held.is(Btn::Start) && self.held.is(Btn::Select) {
            self.quit_ms += dt_ms;
        } else {
            self.quit_ms = 0;
        }
        if self.quit_ms >= QUIT_HOLD_MS && self.rec.is_some() {
            self.stop_recording(); // quitting mid-take keeps the take
        }
        if self.quit_ms >= QUIT_HOLD_MS {
            if let Some(j) = &self.export_job {
                j.cancel.store(true, std::sync::atomic::Ordering::Relaxed); // quitting stops an export
            }
        }
        self.export_poll();
        self.waves_follow_tick();
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
        if let Some((n, left)) = self.audition {
            if left <= dt_ms {
                self.audition = None;
                self.fm1_send(Fm1Msg::NoteOff(n));
            } else {
                self.audition = Some((n, left - dt_ms));
            }
        }
        self.status_ms += dt_ms;
        if self.status_ms >= 500 {
            self.status_ms = 0;
            if let Ok(s) = self.status_src.lock() {
                self.status = s.clone();
            }
            // the FM-1 just appeared (or was there at startup): send what the user chose
            if self.status.fm1 && !self.fm1_was_up {
                self.fm1_resync();
            }
            self.fm1_was_up = self.status.fm1;
        }
        // after the status refresh, so the meter reacts to the device list of this very tick
        self.monitor_tick(dt_ms);
        self.output_tick();
    }

}

impl HillDaw {
    fn event_body(&mut self, b: Btn, down: bool) {
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
                        self.toggle_record();
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
            Btn::B if self.held.is(Btn::Y) => self.undo(),
            Btn::A if self.held.is(Btn::Y) => self.redo(),
            Btn::L1 if self.held.is(Btn::Fn) => {
                self.checkpoint("count-in", 7001);
                self.project.count_in = !self.project.count_in;
                self.toast.say(if self.project.count_in { "Count-in on" } else { "Count-in off" });
            }
            Btn::L1 | Btn::R1 => {
                const ORDER: [View; 6] = [View::Mixer, View::Track, View::Waves, View::Arrange, View::Fm1, View::Export];
                let i = ORDER.iter().position(|v| *v == self.view).unwrap_or(0);
                self.view = ORDER[(i + if b == Btn::R1 { 1 } else { ORDER.len() - 1 }) % ORDER.len()];
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
}

impl App for HillDaw {
    fn name(&self) -> &'static str {
        "hilldaw"
    }

    fn event(&mut self, b: Btn, down: bool) {
        self.event_body(b, down);
        self.settle(); // keep the undo snapshot of this press only if it changed something
    }

    fn audio(&mut self, out: &mut [f32]) {
        let now = std::time::Instant::now();
        let t0 = *self.clock0.get_or_insert(now);
        self.audio_at(out, now.duration_since(t0).as_secs_f64());
    }

    fn render(&self, fb: &mut Fb) {
        self.draw(fb);
    }

    fn tick(&mut self, dt_ms: u32) {
        self.tick_inner(dt_ms);
        self.settle();
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
        // the measured output clock, for the drift test (read it from hilldaw-drift_state.txt next to the program)
        if let Some(f) = self.out_clock() {
            let text = format!("output clock {:.3} Hz = {:+.1} ppm over {:.0} s, {} samples, timestamp jitter {:.1} ms\n", f.rate_hz, f.ppm, f.span_s, f.samples, f.jitter_ms);
            self.lib.host_mut().save_state("hilldaw-drift", &text);
        }
    }
}
