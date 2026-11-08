//! The Track screen (takes, effects, input meter, loop and punch) and FLAC/MP3 export, through the app with a fake recorder.

use hill_core::fb::Fb;
use hill_core::{App, Btn, Host, MemHost};
use hilldaw_app::recorder::{ExportFile, RecInfo, Recorder};
use hilldaw_app::status::{InputSummary, Status, StatusHandle};
use hilldaw_app::{HillDaw, TrackTab, View};
use hilldaw_core::project::{ExportFormat, TakeModel, TrackFxModel};
use hilldaw_core::wav::decode;
use std::collections::BTreeMap;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

const R: f64 = 44100.0;
const PAN_CENTRE: f32 = std::f32::consts::FRAC_1_SQRT_2;

#[derive(Clone, Default)]
struct MemFile(Arc<Mutex<Cursor<Vec<u8>>>>);

impl Write for MemFile {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for MemFile {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.0.lock().unwrap().seek(p)
    }
}

#[derive(Default)]
struct State {
    started: usize,
    files: BTreeMap<String, u64>,
    exports: BTreeMap<String, MemFile>,
    /// input meter
    monitoring: bool,
    monitor_calls: Vec<bool>,
    monitor_fail: Option<String>,
    level: f32,
    /// every `select_input` the app made
    selected: Vec<Option<String>>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<State>>);

impl Recorder for Fake {
    fn start(&mut self) -> Result<String, String> {
        let mut s = self.0.lock().unwrap();
        s.started += 1;
        s.monitoring = false; // like the real recorder: recording takes over from monitoring
        Ok(format!("audio/rec-{:03}.wav", s.started))
    }
    fn frames(&self) -> u64 {
        0
    }
    fn finish(&mut self, min_frames: u64) -> Result<RecInfo, String> {
        let mut s = self.0.lock().unwrap();
        let n = format!("audio/rec-{:03}.wav", s.started);
        s.files.insert(n, min_frames + 500);
        Ok(RecInfo { frames: min_frames + 500, overruns: 0 })
    }
    /// every recording is a ramp whose slope is its file number x 1e-5, so a sample says which file it came from
    fn load(&mut self, rel: &str) -> Option<Vec<f32>> {
        let n = *self.0.lock().unwrap().files.get(rel)?;
        let k: f32 = rel.trim_start_matches("audio/rec-").trim_end_matches(".wav").parse().ok()?;
        Some((0..n).map(|i| i as f32 * k * 1e-5).collect())
    }
    fn select_input(&mut self, id: Option<String>) {
        self.0.lock().unwrap().selected.push(id);
    }
    fn monitor(&mut self, on: bool) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.monitor_calls.push(on);
        if on {
            if let Some(e) = &s.monitor_fail {
                return Err(e.clone());
            }
        }
        s.monitoring = on;
        Ok(())
    }
    fn level(&mut self) -> f32 {
        std::mem::take(&mut self.0.lock().unwrap().level)
    }
    fn create_export(&mut self, stem: &str) -> Result<(Box<dyn ExportFile>, String), String> {
        let mut s = self.0.lock().unwrap();
        let mut path = format!("/card/export/{stem}.wav");
        let mut n = 2;
        while s.exports.contains_key(&path) {
            path = format!("/card/export/{stem}-{n}.wav");
            n += 1;
        }
        let f = MemFile::default();
        s.exports.insert(path.clone(), f.clone());
        Ok((Box::new(f), path))
    }
}

impl Fake {
    fn export_bytes(&self, suffix: &str) -> Vec<u8> {
        let s = self.0.lock().unwrap();
        let (_, f) = s.exports.iter().find(|(p, _)| p.ends_with(suffix)).unwrap_or_else(|| panic!("no export {suffix}"));
        let b = f.0.lock().unwrap().get_ref().clone();
        b
    }
    fn set_level(&self, l: f32) {
        self.0.lock().unwrap().level = l;
    }
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<MemHost>>);

impl Host for Shared {
    fn list_samples(&self) -> Vec<String> {
        self.0.lock().unwrap().list_samples()
    }
    fn read_sample(&self, n: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().read_sample(n)
    }
    fn load_state(&self, a: &str) -> Option<String> {
        self.0.lock().unwrap().load_state(a)
    }
    fn save_state(&mut self, a: &str, d: &str) {
        self.0.lock().unwrap().save_state(a, d)
    }
}

fn status() -> StatusHandle {
    Arc::new(Mutex::new(Status::default()))
}

fn press(a: &mut HillDaw, b: Btn) {
    a.event(b, true);
    a.event(b, false);
}

fn with(a: &mut HillDaw, hold: Btn, b: Btn) {
    a.event(hold, true);
    press(a, b);
    a.event(hold, false);
}

fn to_view(a: &mut HillDaw, v: View) {
    for _ in 0..8 {
        if a.view() == v {
            return;
        }
        press(a, Btn::R1);
    }
    panic!("never reached {v:?}");
}

fn to_tab(a: &mut HillDaw, t: TrackTab) {
    to_view(a, View::Track);
    for _ in 0..6 {
        if a.track_tab() == t {
            return;
        }
        press(a, if (a.track_tab() as usize) < (t as usize) { Btn::R2 } else { Btn::L2 });
    }
    panic!("never reached {t:?}");
}

fn new_app() -> (HillDaw, Fake) {
    let fake = Fake::default();
    let a = HillDaw::new(Box::new(MemHost::default()), status()).with_recorder(Box::new(fake.clone()));
    (a, fake)
}

fn remove_drums(a: &mut HillDaw) {
    to_view(a, View::Arrange);
    press(a, Btn::X);
    to_view(a, View::Mixer);
}

/// Audio time keeps running across recordings so every capture starts at 0 on its own clock.
struct Clock(u64);

impl Clock {
    fn run(&mut self, a: &mut HillDaw, frames: usize) -> Vec<f32> {
        let mut all = Vec::new();
        let (mut left, mut buf) = (frames, vec![0.0f32; 2048]);
        while left > 0 {
            let n = left.min(1024);
            a.audio_at(&mut buf[..n * 2], self.0 as f64 / R);
            all.extend_from_slice(&buf[..n * 2]);
            self.0 += n as u64;
            left -= n;
        }
        all
    }

    /// Rewind to the start, then record: a take is placed where the playhead is when recording begins.
    fn record(&mut self, a: &mut HillDaw, secs: f64) {
        with(a, Btn::Fn, Btn::B);
        a.set_test_clock(Some(self.0 as f64 / R));
        press(a, Btn::Select);
        self.run(a, (secs * R) as usize);
        press(a, Btn::Select);
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// An app with the drums removed, count-in off and track 1 armed, so only recorded audio sounds.
fn recording_app() -> (HillDaw, Fake, Clock) {
    let (mut a, fake) = new_app();
    remove_drums(&mut a);
    with(&mut a, Btn::Fn, Btn::L1); // count-in off
    press(&mut a, Btn::A);
    (a, fake, Clock(0))
}

// ---- takes ----

#[test]
fn a_take_remembers_where_it_was_placed_and_any_take_can_be_made_the_playing_one() {
    let (mut a, _f, mut clock) = recording_app();
    for s in [1.0, 2.0, 1.5] {
        clock.record(&mut a, s);
    }
    let t = &a.project.tracks[0];
    assert_eq!((t.takes.len(), t.active_take), (3, Some(2)));
    assert!(t.takes.iter().all(|k| k.placed && k.start == 0 && k.offset == 563), "{:?}", t.takes);

    to_view(&mut a, View::Track);
    assert_eq!((a.track_tab(), a.take_sel()), (TrackTab::Takes, 0));
    press(&mut a, Btn::A); // take 1
    assert_eq!(a.project.tracks[0].active_take, Some(0));
    assert_eq!(a.project.tracks[0].clips[0].file, "audio/rec-001.wav");
    assert!(a.toast_text().contains("Take 1 plays"), "{}", a.toast_text());
    // it really plays that take: file 1 has slope 1e-5, so frame p is (563 + p) * 1e-5 through the centre pan law
    with(&mut a, Btn::Fn, Btn::B);
    let out = clock.run(&mut a, 3000);
    for p in [0usize, 500, 2000] {
        let want = (563 + p) as f32 * 1e-5 * PAN_CENTRE;
        assert!((out[2 * p] - want).abs() < 2e-5, "frame {p}: {} vs {want}", out[2 * p]);
    }
    // and take 3 (slope 3e-5) once chosen
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::A);
    with(&mut a, Btn::Fn, Btn::B);
    let out = clock.run(&mut a, 3000);
    assert!((out[2 * 100] - (663.0 * 3e-5 * PAN_CENTRE)).abs() < 2e-5, "{}", out[2 * 100]);
}

#[test]
fn removing_a_take_needs_two_presses_and_the_newest_one_takes_over_if_it_was_playing() {
    let (mut a, _f, mut clock) = recording_app();
    for s in [1.0, 1.0, 1.0] {
        clock.record(&mut a, s);
    }
    to_view(&mut a, View::Track);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Down); // take 3 selected
    press(&mut a, Btn::X);
    assert_eq!(a.project.tracks[0].takes.len(), 3, "one press only asks");
    assert!(a.toast_text().contains("X again"), "{}", a.toast_text());
    // the question times out
    a.tick(3500);
    press(&mut a, Btn::X);
    assert_eq!(a.project.tracks[0].takes.len(), 3, "an old question does not count");
    press(&mut a, Btn::X);
    let t = &a.project.tracks[0];
    assert_eq!(t.takes.len(), 2, "removed on the second press");
    assert_eq!(t.active_take, Some(1), "take 3 was playing: the newest remaining (now index 1) takes over");
    assert_eq!(t.clips[0].file, "audio/rec-002.wav");
    assert_eq!(a.take_sel(), 1, "the selection stays on the list");
    // removing a take that is not playing leaves the playing one alone
    press(&mut a, Btn::Up);
    press(&mut a, Btn::X);
    press(&mut a, Btn::X);
    let t = &a.project.tracks[0];
    assert_eq!((t.takes.len(), t.active_take), (1, Some(0)));
    assert_eq!(t.clips[0].file, "audio/rec-002.wav", "still the same audio, now take 1");
    // removing the last one leaves the track silent
    press(&mut a, Btn::X);
    press(&mut a, Btn::X);
    assert!(a.project.tracks[0].takes.is_empty() && a.project.tracks[0].clips.is_empty() && a.project.tracks[0].active_take.is_none());
}

#[test]
fn an_older_take_without_a_saved_place_cannot_be_switched_to_and_the_drum_strip_has_no_takes() {
    let (mut a, _f, mut clock) = recording_app();
    clock.record(&mut a, 1.0);
    a.project.tracks[0].takes.insert(0, TakeModel { file: "audio/old.wav".into(), frames: 100, ..Default::default() });
    to_view(&mut a, View::Track);
    press(&mut a, Btn::A);
    assert!(a.toast_text().contains("older take"), "{}", a.toast_text());
    assert_eq!(a.project.tracks[0].active_take, Some(0), "unchanged (index shifted by the insert, still a valid take)");
    // the drum strip: nothing to do, nothing breaks
    to_view(&mut a, View::Mixer);
    for _ in 0..8 {
        press(&mut a, Btn::Right);
    }
    to_view(&mut a, View::Track);
    press(&mut a, Btn::A);
    press(&mut a, Btn::X);
    let mut fb = Fb::new();
    a.render(&mut fb);
}

// ---- effects ----

fn play_3s(a: &mut HillDaw) -> Vec<f32> {
    to_view(a, View::Mixer);
    press(a, Btn::Start);
    Clock(0).run(a, 3 * 44100)
}

fn drums_strip(a: &mut HillDaw) {
    to_view(a, View::Mixer);
    for _ in 0..8 {
        press(a, Btn::Right);
    }
}

/// Energy in the high frequencies: the RMS of the sample-to-sample difference (a low-pass takes this away, a bass drum has little of it).
fn highs(x: &[f32]) -> f32 {
    let left: Vec<f32> = x.iter().step_by(2).copied().collect();
    rms(&left.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>())
}

#[test]
fn filter_delay_and_reverb_edited_on_the_screen_change_the_sound_and_survive_a_restart() {
    let (mut plain, _f) = new_app();
    let base = play_3s(&mut plain);
    assert!(highs(&base) > 0.01, "the starter groove has highs to take out: {}", highs(&base));

    let host = Shared::default();
    let mut a = HillDaw::new(Box::new(host.clone()), status());
    drums_strip(&mut a);
    to_tab(&mut a, TrackTab::Fx);
    // row 0, low-pass: off -> Left comes in from the top (20 kHz) and walks down; about 33 steps to 500 Hz
    for _ in 0..33 {
        press(&mut a, Btn::Left);
    }
    let lp = a.project.drums.fx.lowpass_hz;
    assert!((400..700).contains(&lp), "{lp} Hz");
    let filtered = play_3s(&mut a);
    assert!(highs(&filtered) < highs(&base) * 0.5, "the low-pass takes the highs out: {} vs {}", highs(&filtered), highs(&base));

    // delay mix (row 4) and reverb send (row 5)
    to_tab(&mut a, TrackTab::Fx);
    for _ in 0..4 {
        press(&mut a, Btn::Down);
    }
    for _ in 0..6 {
        press(&mut a, Btn::Right);
    }
    press(&mut a, Btn::Down);
    for _ in 0..6 {
        press(&mut a, Btn::Right);
    }
    let f = a.project.drums.fx.clone();
    assert_eq!((f.delay_mix, f.reverb_send), (0.3, 0.3));
    a.save();

    // a restart: the same project plays the same sound every time, and it is not the dry sound
    let play_fresh = || {
        let mut b = HillDaw::new(Box::new(host.clone()), status());
        assert_eq!(b.project.drums.fx, f, "the effects were saved with the project");
        press(&mut b, Btn::Start);
        Clock(0).run(&mut b, 3 * 44100)
    };
    let (w1, w2) = (play_fresh(), play_fresh());
    assert!(w1 == w2, "deterministic after a restart");
    assert!(w1 != base && highs(&w1) < highs(&base) * 0.7, "and still filtered after the restart");
}

#[test]
fn cutoff_steps_come_in_from_the_ends_and_switch_off_past_them() {
    use hilldaw_app::step_cutoff;
    // a low-pass: off, Left comes in at the top; going up from the top switches it off again
    assert_eq!(step_cutoff(0, -1, 100, 20_000, false, true), 20_000);
    assert_eq!(step_cutoff(0, 1, 100, 20_000, false, true), 0);
    assert_eq!(step_cutoff(20_000, 1, 100, 20_000, false, true), 0);
    assert!(step_cutoff(1000, 1, 100, 20_000, false, true) > 1000 && step_cutoff(1000, -1, 100, 20_000, false, true) < 1000);
    assert_eq!(step_cutoff(100, -1, 100, 20_000, false, true), 100, "stops at the bottom");
    // a high-pass: off, Right comes in at the bottom; going down from the bottom switches it off
    assert_eq!(step_cutoff(0, 1, 20, 2_000, false, false), 20);
    assert_eq!(step_cutoff(20, -1, 20, 2_000, false, false), 0);
    assert_eq!(step_cutoff(2_000, 1, 20, 2_000, false, false), 2_000, "stops at the top");
    // bigger steps with Y, and tiny values still move
    assert!(step_cutoff(1000, 1, 100, 20_000, true, true) > step_cutoff(1000, 1, 100, 20_000, false, true));
    assert_eq!(step_cutoff(20, 1, 20, 2_000, false, false), 22);
}

#[test]
fn the_export_includes_each_tracks_effects_and_sounds_like_playback() {
    let (mut a, fake) = new_app();
    a.set_strip_fx(8, TrackFxModel { lowpass_hz: 3000, delay_ms: 250, delay_feedback: 0.4, delay_mix: 0.3, reverb_send: 0.3, ..Default::default() });
    to_view(&mut a, View::Export);
    press(&mut a, Btn::A);
    for _ in 0..4000 {
        a.tick(16);
        if !a.export_running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let wav = decode(&fake.export_bytes("song-mixdown.wav")).expect("a WAV");
    to_view(&mut a, View::Mixer);
    press(&mut a, Btn::Start);
    let live = Clock(0).run(&mut a, 3 * 44100);
    let worst = live.iter().zip(&wav.data).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
    assert!(worst < 1e-5, "effects included: the export and the live render differ by {worst}");
    assert!(rms(&live) > 0.01);
}

// ---- input meter ----

#[test]
fn the_meter_runs_while_a_track_is_armed_or_the_input_tab_is_open_and_stops_when_neither() {
    let (mut a, fake) = new_app();
    a.tick(100);
    assert!(fake.0.lock().unwrap().monitor_calls.is_empty(), "nothing wanted: the interface is not claimed");
    press(&mut a, Btn::A); // arm track 1
    a.tick(100);
    assert!(fake.0.lock().unwrap().monitoring);
    press(&mut a, Btn::A); // disarm
    a.tick(100);
    assert!(!fake.0.lock().unwrap().monitoring);
    to_tab(&mut a, TrackTab::Input);
    a.tick(100);
    assert!(fake.0.lock().unwrap().monitoring, "the Input tab listens even with nothing armed");
    press(&mut a, Btn::L2); // leave the tab
    a.tick(100);
    assert!(!fake.0.lock().unwrap().monitoring);
}

#[test]
fn input_level_hold_clip_and_decay() {
    let (mut a, fake) = new_app();
    to_tab(&mut a, TrackTab::Input);
    a.tick(100);
    fake.set_level(0.25);
    a.tick(20);
    assert!((a.input_level() - 0.25).abs() < 1e-6 && (a.input_hold() - 0.25).abs() < 1e-6 && !a.clip_flash());
    for _ in 0..20 {
        a.tick(20);
    }
    assert!(a.input_level() < 0.06 && a.input_level() < 0.25 / 3.0, "the bar falls: {}", a.input_level());
    assert!((a.input_hold() - 0.25).abs() < 1e-6, "the hold tick stays for a moment");
    for _ in 0..100 {
        a.tick(20);
    }
    assert!(a.input_hold() < 0.01, "and then lets go");
    fake.set_level(0.995);
    a.tick(20);
    assert!(a.clip_flash(), "within 0.2 dB of full scale warns");
    a.tick(2100);
    assert!(!a.clip_flash(), "the warning fades");
    let mut fb = Fb::new();
    a.render(&mut fb);
}

#[test]
fn a_missing_fm1_is_shown_not_retried_every_tick_and_recording_takes_over_from_the_meter() {
    let (mut a, fake) = new_app();
    fake.0.lock().unwrap().monitor_fail = Some("FM-1 not found".into());
    to_tab(&mut a, TrackTab::Input);
    for _ in 0..50 {
        a.tick(20); // one second
    }
    assert_eq!(a.monitor_error(), Some("FM-1 not found"));
    let tries = fake.0.lock().unwrap().monitor_calls.iter().filter(|c| **c).count();
    assert!(tries <= 2, "it backs off instead of retrying on every tick: {tries}");
    // plugged in now: it connects within a couple of seconds
    fake.0.lock().unwrap().monitor_fail = None;
    for _ in 0..150 {
        a.tick(20);
    }
    assert!(fake.0.lock().unwrap().monitoring && a.monitor_error().is_none());

    // arm and record: the recorder takes over; afterwards (still armed) the meter comes back
    let (mut a, fake, mut clock) = recording_app();
    a.tick(100);
    assert!(fake.0.lock().unwrap().monitoring);
    clock.record(&mut a, 1.0);
    a.tick(100);
    assert!(fake.0.lock().unwrap().monitoring, "monitoring resumed after the take");
    assert_eq!(a.project.tracks[0].takes.len(), 1);
}

#[test]
fn the_latency_offset_can_be_nudged_and_moves_where_the_next_take_is_placed() {
    let (mut a, _f, mut clock) = recording_app();
    to_tab(&mut a, TrackTab::Input);
    press(&mut a, Btn::Down); // row 0 is the input device; row 1 the latency offset
    assert_eq!(a.project.latency_offset_frames, 563);
    press(&mut a, Btn::Left);
    assert_eq!(a.project.latency_offset_frames, 519, "about 1 ms per step");
    a.event(Btn::Y, true);
    press(&mut a, Btn::Left);
    a.event(Btn::Y, false);
    assert_eq!(a.project.latency_offset_frames, 299, "5 ms with Y");
    for _ in 0..30 {
        press(&mut a, Btn::Left);
    }
    assert_eq!(a.project.latency_offset_frames, 0, "never before the capture itself");
    for _ in 0..200 {
        press(&mut a, Btn::Right);
    }
    assert_eq!(a.project.latency_offset_frames, 4410, "and never later than 100 ms");
    for _ in 0..200 {
        press(&mut a, Btn::Left);
    }
    to_view(&mut a, View::Mixer);
    clock.record(&mut a, 1.0);
    assert_eq!(a.project.tracks[0].clips[0].offset, 0, "with no offset the clip starts at the first captured frame");
    // the count-in switch on the same tab
    to_tab(&mut a, TrackTab::Input);
    press(&mut a, Btn::Down);
    let before = a.project.count_in;
    press(&mut a, Btn::Right);
    assert_ne!(a.project.count_in, before);
}

// ---- loop and punch ----

#[test]
fn the_loop_is_set_in_bars_remembered_when_switched_off_and_set_from_the_playhead() {
    let (mut a, _f) = new_app();
    let bar = 2 * 44100 + 44100 * 4 / 10; // 100 bpm, 4/4: 2.4 s
    to_tab(&mut a, TrackTab::Loop);
    press(&mut a, Btn::Right); // loop on: bars 1-4
    assert_eq!(a.transport_loop(), Some((0, 4 * bar as u64)));
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Right); // start bar 2
    assert_eq!(a.transport_loop(), Some((bar as u64, 5 * bar as u64)), "moving the start keeps the length");
    a.event(Btn::Y, true);
    press(&mut a, Btn::Right); // +4 bars
    a.event(Btn::Y, false);
    assert_eq!(a.transport_loop().unwrap().0, 5 * bar as u64);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Left); // length 3
    assert_eq!(a.transport_loop(), Some((5 * bar as u64, 8 * bar as u64)));
    // off and on again brings the same region back
    press(&mut a, Btn::Up);
    press(&mut a, Btn::Up);
    press(&mut a, Btn::Left);
    assert_eq!(a.transport_loop(), None);
    press(&mut a, Btn::Right);
    assert_eq!(a.transport_loop(), Some((5 * bar as u64, 8 * bar as u64)));
    // A on the start row = the bar the playhead is in (3.5 bars in = bar 4)
    to_view(&mut a, View::Mixer);
    press(&mut a, Btn::Start);
    Clock(0).run(&mut a, (3.5 * 2.4 * R) as usize);
    press(&mut a, Btn::Start);
    to_tab(&mut a, TrackTab::Loop);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::A);
    assert_eq!(a.transport_loop().unwrap().0, 3 * bar as u64, "bar 4 starts 3 bars in");
    assert_eq!(a.project.loop_region, a.transport_loop(), "stored in the project");
}

#[test]
fn recording_inside_a_punch_window_keeps_only_that_part() {
    let (mut a, _f, mut clock) = recording_app();
    let bar = (2.4 * R).round() as u64;
    to_tab(&mut a, TrackTab::Loop);
    for _ in 0..3 {
        press(&mut a, Btn::Down);
    }
    press(&mut a, Btn::Right); // punch on: bars 1-4
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Right); // from bar 2
    press(&mut a, Btn::Down);
    for _ in 0..3 {
        press(&mut a, Btn::Left); // length 1
    }
    assert_eq!(a.project.punch, Some((bar, 2 * bar)));
    to_view(&mut a, View::Mixer);
    clock.record(&mut a, 7.0); // bars 1-3 pass; only bar 2 is kept
    let t = &a.project.tracks[0];
    assert_eq!(t.takes.len(), 1);
    assert_eq!((t.clips[0].start, t.clips[0].len), (bar, Some(bar)), "just the punched bar, placed where it was played");
    // switching the punch off records everything again
    to_tab(&mut a, TrackTab::Loop);
    for _ in 0..2 {
        press(&mut a, Btn::Up); // from the punch length row back to the punch on/off row
    }
    assert_eq!(a.track_row(), 3);
    press(&mut a, Btn::Left);
    assert_eq!(a.project.punch, None);
}

#[test]
fn a_loop_set_on_the_screen_makes_each_pass_its_own_take() {
    let (mut a, _f, mut clock) = recording_app();
    to_tab(&mut a, TrackTab::Loop);
    press(&mut a, Btn::Right); // loop on, bars 1-4
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Down);
    for _ in 0..3 {
        press(&mut a, Btn::Left); // length 1 bar
    }
    to_view(&mut a, View::Mixer);
    clock.record(&mut a, 2.4 * 2.6);
    assert_eq!(a.project.tracks[0].takes.len(), 3, "two full passes and a part");
}

// ---- FLAC / MP3 ----

fn wait_export(a: &mut HillDaw) {
    for _ in 0..4000 {
        a.tick(16);
        if !a.export_running() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the export did not finish");
}

#[test]
fn the_format_row_cycles_through_wav_flac_and_mp3_and_mp3_is_16_bit() {
    let (mut a, _f) = new_app();
    to_view(&mut a, View::Export);
    let mut seen = Vec::new();
    for _ in 0..6 {
        seen.push((a.project.export.format, a.project.export.bits));
        press(&mut a, Btn::Right);
    }
    assert_eq!(
        seen,
        vec![(ExportFormat::Wav, 24), (ExportFormat::Wav, 16), (ExportFormat::Flac, 24), (ExportFormat::Flac, 16), (ExportFormat::Mp3, 16), (ExportFormat::Wav, 24)]
    );
    // six presses left it one step past the start (WAV 16); Left goes round the other way, through the start to MP3
    press(&mut a, Btn::Left);
    assert_eq!((a.project.export.format, a.project.export.bits), (ExportFormat::Wav, 24));
    press(&mut a, Btn::Left);
    assert_eq!((a.project.export.format, a.project.export.bits), (ExportFormat::Mp3, 16));
}

#[test]
fn flac_and_mp3_exports_hand_each_finished_wav_to_the_converter_and_report_failures() {
    let (mut a, _f) = new_app();
    let calls: Arc<Mutex<Vec<(String, ExportFormat)>>> = Arc::default();
    let c2 = calls.clone();
    a.set_converter(Arc::new(move |wav: &str, fmt| {
        c2.lock().unwrap().push((wav.to_string(), fmt));
        Ok(wav.replace(".wav", &format!(".{}", fmt.ext())))
    }));
    a.project.export.format = ExportFormat::Flac;
    a.project.export.stems = true;
    to_view(&mut a, View::Export);
    press(&mut a, Btn::A);
    wait_export(&mut a);
    let rep = a.export_report().unwrap().as_ref().unwrap().clone();
    assert_eq!(rep.files, vec!["/card/export/song-mixdown.flac", "/card/export/song-stem-drums.flac"], "the converted names are reported");
    assert_eq!(calls.lock().unwrap().len(), 2, "one conversion per file");
    assert!(calls.lock().unwrap().iter().all(|(_, f)| *f == ExportFormat::Flac));
    assert!(rep.warnings.is_empty());

    // no encoder on the device: the WAV is kept and the screen says why
    a.set_converter(Arc::new(|_: &str, _| Err("no MP3 encoder found: kept the WAV".to_string())));
    a.project.export.format = ExportFormat::Mp3;
    a.project.export.stems = false;
    press(&mut a, Btn::A);
    wait_export(&mut a);
    let rep = a.export_report().unwrap().as_ref().unwrap();
    assert_eq!(rep.warnings, vec!["no MP3 encoder found: kept the WAV".to_string()]);
    assert!(rep.files[0].ends_with(".wav"), "{:?}", rep.files);
    let mut fb = Fb::new();
    a.render(&mut fb);
}

// ---- drawing ----

#[test]
fn every_track_tab_draws_for_a_track_and_for_the_drums() {
    let (mut a, _f, mut clock) = recording_app();
    clock.record(&mut a, 1.0);
    for strip in [0usize, 8] {
        to_view(&mut a, View::Mixer);
        for _ in 0..8 {
            press(&mut a, if strip == 8 { Btn::Right } else { Btn::Left });
        }
        for t in [TrackTab::Takes, TrackTab::Fx, TrackTab::Input, TrackTab::Loop] {
            to_tab(&mut a, t);
            for _ in 0..3 {
                let mut fb = Fb::new();
                a.render(&mut fb);
                let lit = fb.px.iter().filter(|p| **p != hill_core::fb::BG).count();
                assert!(lit > 12_000, "strip {strip} tab {t:?}: {lit} px");
                press(&mut a, Btn::Down);
                press(&mut a, Btn::Right);
            }
        }
    }
}

// ---- generic USB audio input ----

const FM1: &str = "4c4a:c755";
const MIC: &str = "1234:00ab";

fn inputs(list: &[(&str, &str, &str, u32)]) -> Vec<InputSummary> {
    list.iter().map(|(id, name, summary, rate)| InputSummary { id: id.to_string(), name: name.to_string(), summary: summary.to_string(), rate: *rate }).collect()
}

fn both_attached() -> Vec<InputSummary> {
    inputs(&[(MIC, "Studio Mic", "1ch 16-bit 48 kHz", 48_000), (FM1, "FM-1", "2ch 24-bit 44.1 kHz", 44_100)])
}

/// An app on a shared host, with a status handle the test can fill in, and the Input tab open.
fn input_app(host: &Shared) -> (HillDaw, Fake, StatusHandle) {
    let fake = Fake::default();
    let st = status();
    let mut a = HillDaw::new(Box::new(host.clone()), st.clone()).with_recorder(Box::new(fake.clone()));
    to_tab(&mut a, TrackTab::Input);
    (a, fake, st)
}

#[test]
fn the_automatic_input_is_the_fm1_when_attached_otherwise_the_first_and_the_top_bar_says_which() {
    let (mut a, _f, st) = input_app(&Shared::default());
    a.tick(600);
    assert_eq!((a.input_name(), a.selected_input()), (None, None), "nothing attached");
    st.lock().unwrap().inputs = inputs(&[(MIC, "Studio Mic", "1ch 16-bit 48 kHz", 48_000)]);
    a.tick(600);
    assert_eq!(a.input_name(), Some("Studio Mic"), "no FM-1: the first input");
    st.lock().unwrap().inputs = both_attached();
    a.tick(600);
    assert_eq!(a.input_name(), Some("FM-1"), "the FM-1 wins automatically even though the mic is listed first");
    for view in [View::Mixer, View::Track, View::Export] {
        to_view(&mut a, view);
        let mut fb = Fb::new();
        a.render(&mut fb);
        assert!(fb.px.iter().filter(|p| **p != hill_core::fb::BG).count() > 12_000);
    }
}

#[test]
fn the_input_is_chosen_with_left_right_remembered_across_restarts_and_given_to_the_recorder() {
    let host = Shared::default();
    let (mut a, fake, st) = input_app(&host);
    st.lock().unwrap().inputs = both_attached();
    a.tick(600);
    assert_eq!(a.track_row(), 0);
    press(&mut a, Btn::Right); // Auto -> first device (the mic)
    assert_eq!((a.selected_input(), a.input_name()), (Some(MIC), Some("Studio Mic")));
    press(&mut a, Btn::Right);
    assert_eq!((a.selected_input(), a.input_name()), (Some(FM1), Some("FM-1")));
    press(&mut a, Btn::Right); // round to Auto
    assert_eq!(a.selected_input(), None);
    press(&mut a, Btn::Left); // and back the other way
    assert_eq!(a.selected_input(), Some(FM1));
    press(&mut a, Btn::Left);
    assert_eq!(a.selected_input(), Some(MIC));
    assert_eq!(fake.0.lock().unwrap().selected.last().unwrap().as_deref(), Some(MIC), "the recorder was told");
    assert!(a.toast_text().contains("Studio Mic"), "{}", a.toast_text());

    // a restart remembers it and tells the recorder before anything is opened
    let (b, fake2, st2) = input_app(&host);
    assert_eq!(b.selected_input(), Some(MIC));
    assert_eq!(fake2.0.lock().unwrap().selected.first().unwrap().as_deref(), Some(MIC));
    // the choice is the device's, not the song's: it is not in the project file
    let project_json = host.load_state("hilldaw").unwrap_or_default();
    assert!(!project_json.contains(MIC), "the project file does not carry the input choice");
    drop((b, st2));
}

#[test]
fn a_chosen_input_that_is_unplugged_is_shown_as_such_and_not_replaced_by_another() {
    let host = Shared::default();
    let (mut a, _f, st) = input_app(&host);
    st.lock().unwrap().inputs = both_attached();
    a.tick(600);
    press(&mut a, Btn::Right); // the mic
    st.lock().unwrap().inputs = inputs(&[(FM1, "FM-1", "2ch 24-bit 44.1 kHz", 44_100)]); // the mic is unplugged
    a.tick(600);
    assert_eq!((a.selected_input(), a.input_name()), (Some(MIC), None), "still chosen, but there is no input: the FM-1 is not substituted");
    let mut fb = Fb::new();
    a.render(&mut fb);
    // moving on from a missing choice lands on the first entry (Auto when going left, the first device going right)
    press(&mut a, Btn::Left);
    assert_eq!(a.selected_input(), None);
    press(&mut a, Btn::Right);
    assert_eq!(a.selected_input(), Some(FM1));
}

#[test]
fn the_meter_reopens_on_the_new_input_when_the_choice_or_the_automatic_pick_changes() {
    let settle = |a: &mut HillDaw| {
        a.tick(600);
        for _ in 0..3 {
            a.tick(100);
        }
    };
    let host = Shared::default();
    let (mut a, fake, st) = input_app(&host);
    st.lock().unwrap().inputs = inputs(&[(MIC, "Studio Mic", "1ch 16-bit 48 kHz", 48_000)]);
    settle(&mut a);
    assert!(fake.0.lock().unwrap().monitoring, "listening to the mic (the Input tab is open)");
    // the FM-1 is plugged in: the automatic pick moves to it, so the meter reopens on it
    let base = fake.0.lock().unwrap().monitor_calls.len();
    st.lock().unwrap().inputs = both_attached();
    settle(&mut a);
    assert_eq!(&fake.0.lock().unwrap().monitor_calls[base..], &[false, true]);
    assert!(fake.0.lock().unwrap().monitoring);
    // choosing a device by hand reopens it too, and nothing else changes while it just keeps running
    let base = fake.0.lock().unwrap().monitor_calls.len();
    press(&mut a, Btn::Right);
    settle(&mut a);
    assert_eq!(&fake.0.lock().unwrap().monitor_calls[base..], &[false, true]);
    let base = fake.0.lock().unwrap().monitor_calls.len();
    settle(&mut a);
    assert_eq!(fake.0.lock().unwrap().monitor_calls.len(), base, "no churn while nothing changes");
}

#[test]
fn the_input_cannot_be_changed_while_recording() {
    let host = Shared::default();
    let fake = Fake::default();
    let st = status();
    let mut a = HillDaw::new(Box::new(host), st.clone()).with_recorder(Box::new(fake.clone()));
    st.lock().unwrap().inputs = both_attached();
    a.tick(600);
    remove_drums(&mut a);
    with(&mut a, Btn::Fn, Btn::L1);
    press(&mut a, Btn::A);
    let mut clock = Clock(0);
    a.set_test_clock(Some(0.0));
    press(&mut a, Btn::Select);
    clock.run(&mut a, 4410);
    to_tab(&mut a, TrackTab::Input);
    press(&mut a, Btn::Right);
    assert!(a.toast_text().contains("Stop recording first"), "{}", a.toast_text());
    assert_eq!(a.selected_input(), None);
    press(&mut a, Btn::Select);
}

// ---- clip editing (the Clip tab) ----

/// A track with one 2 s take (file 1: ramp, slope 1e-5, offset 563, placed at frame 0) and the Clip tab open.
fn clip_app() -> (HillDaw, Fake, Clock) {
    let (mut a, f, mut clock) = recording_app();
    clock.record(&mut a, 2.0);
    to_tab(&mut a, TrackTab::Clip);
    (a, f, clock)
}

fn play_from_start(a: &mut HillDaw, clock: &mut Clock, frames: usize) -> Vec<f32> {
    with(a, Btn::Fn, Btn::B);
    clock.run(a, frames)
}

fn rows_down(a: &mut HillDaw, n: usize) {
    for _ in 0..n {
        press(a, Btn::Down);
    }
}

#[test]
fn trimming_the_start_keeps_the_sound_where_it_was_and_silences_what_was_cut() {
    let (mut a, _f, mut clock) = clip_app();
    rows_down(&mut a, 2);
    let before = a.project.tracks[0].clips[0].clone();
    press(&mut a, Btn::Right);
    press(&mut a, Btn::Right); // 2 x 441 frames
    let c = a.project.tracks[0].clips[0].clone();
    assert_eq!((c.offset, c.start, c.len), (before.offset + 882, 882, before.len.map(|l| l - 882)));
    let out = play_from_start(&mut a, &mut clock, 3000);
    assert!(out[..2 * 880].iter().all(|s| *s == 0.0), "the cut part is silent");
    for p in [882usize, 1500, 2500] {
        let want = (563 + p) as f32 * 1e-5 * PAN_CENTRE; // the same sample as before the trim: it did not slide
        assert!((out[2 * p] - want).abs() < 2e-5, "frame {p}: {} vs {want}", out[2 * p]);
    }
    // Left walks it back out, never before the start of the file
    for _ in 0..40 {
        press(&mut a, Btn::Left);
    }
    assert_eq!(a.project.tracks[0].clips[0].offset, 0);
}

#[test]
fn the_length_shortens_and_cannot_pass_the_end_of_the_recording() {
    let (mut a, _f, mut clock) = clip_app();
    rows_down(&mut a, 3);
    let full = a.project.tracks[0].clips[0].len.unwrap();
    for _ in 0..3 {
        press(&mut a, Btn::Left);
    }
    assert_eq!(a.project.tracks[0].clips[0].len, Some(full - 3 * 441));
    let out = play_from_start(&mut a, &mut clock, full as usize);
    let end = (full - 3 * 441) as usize;
    assert!(out[2 * (end - 10)] != 0.0 && out[2 * (end + 10)] == 0.0, "sound stops at the new end");
    for _ in 0..10 {
        press(&mut a, Btn::Right);
    }
    // the clip can grow back out to the end of the file (the fake's file holds 500 frames more than the take used) and no further
    let cap = a.project.tracks[0].clips[0].len.unwrap();
    assert_eq!(cap, full + 500, "cannot extend past the file");
    press(&mut a, Btn::Right);
    assert_eq!(a.project.tracks[0].clips[0].len, Some(cap));
    // Y steps 100 ms
    a.event(Btn::Y, true);
    press(&mut a, Btn::Left);
    a.event(Btn::Y, false);
    assert_eq!(a.project.tracks[0].clips[0].len, Some(cap - 4410));
}

#[test]
fn fades_shape_the_ends_and_reverse_plays_backwards() {
    let (mut a, _f, mut clock) = clip_app();
    rows_down(&mut a, 4);
    press(&mut a, Btn::Right);
    press(&mut a, Btn::Right); // fade in 882 frames
    assert_eq!(a.project.tracks[0].clips[0].fade_in, 882);
    let out = play_from_start(&mut a, &mut clock, 2000);
    for p in [100usize, 441, 800] {
        let want = (563 + p) as f32 * 1e-5 * PAN_CENTRE * (p as f32 / 882.0);
        assert!((out[2 * p] - want).abs() < 2e-5, "frame {p}: {} vs {want}", out[2 * p]);
    }
    rows_down(&mut a, 1);
    for _ in 0..3 {
        press(&mut a, Btn::Right);
    }
    assert_eq!(a.project.tracks[0].clips[0].fade_out, 1323);

    rows_down(&mut a, 2);
    press(&mut a, Btn::A); // reverse
    assert!(a.project.tracks[0].clips[0].reverse);
    let c = a.project.tracks[0].clips[0].clone();
    let len = c.len.unwrap();
    let out = play_from_start(&mut a, &mut clock, len as usize);
    let p = 3000usize;
    let src = c.offset + len - 1 - p as u64; // frame p of a reversed clip reads from the far end
    let g = (p as f32 / 882.0).min(1.0);
    assert!((out[2 * p] - src as f32 * 1e-5 * PAN_CENTRE * g).abs() < 3e-5, "{}", out[2 * p]);
}

#[test]
fn the_start_moves_by_a_sixteenth_or_a_bar_and_cannot_go_before_zero() {
    let (mut a, _f, _c) = clip_app();
    let bar = a.bar_frames_for_test();
    rows_down(&mut a, 1);
    press(&mut a, Btn::Right);
    assert_eq!(a.project.tracks[0].clips[0].start, bar / 16);
    a.event(Btn::Y, true);
    press(&mut a, Btn::Right);
    a.event(Btn::Y, false);
    assert_eq!(a.project.tracks[0].clips[0].start, bar / 16 + bar);
    for _ in 0..3 {
        a.event(Btn::Y, true);
        press(&mut a, Btn::Left);
        a.event(Btn::Y, false);
    }
    assert_eq!(a.project.tracks[0].clips[0].start, 0);
}

#[test]
fn clip_edits_stay_with_their_take_undo_removes_them_and_a_restart_keeps_them() {
    let (mut a, _f, mut clock) = recording_app();
    clock.record(&mut a, 1.0);
    clock.record(&mut a, 1.0);
    to_tab(&mut a, TrackTab::Clip);
    rows_down(&mut a, 4);
    press(&mut a, Btn::Right); // fade in on take 2 (playing)
    assert_eq!(a.project.tracks[0].takes[1].fade_in, 441);
    to_tab(&mut a, TrackTab::Takes);
    press(&mut a, Btn::A); // take 1: untouched
    assert_eq!(a.project.tracks[0].clips[0].fade_in, 0);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::A); // take 2 again: its fade is back
    assert_eq!(a.project.tracks[0].clips[0].fade_in, 441);
    // undo the take switch, then the edit
    with(&mut a, Btn::Y, Btn::B);
    with(&mut a, Btn::Y, Btn::B);
    with(&mut a, Btn::Y, Btn::B);
    assert_eq!(a.project.tracks[0].clips[0].fade_in, 0, "undo took the fade out again");
    assert_eq!(a.project.tracks[0].takes[1].fade_in, 0);
}

#[test]
fn the_clip_tab_says_so_when_there_is_nothing_to_edit_and_never_touches_the_drums() {
    let (mut a, _f) = new_app();
    to_tab(&mut a, TrackTab::Clip);
    press(&mut a, Btn::Right); // no take: a toast, no change
    assert!(a.toast_text().contains("No take"), "{}", a.toast_text());
    let mut fb = Fb::new();
    a.render(&mut fb);
    // the drums strip
    to_view(&mut a, View::Mixer);
    for _ in 0..8 {
        press(&mut a, Btn::Right);
    }
    to_tab(&mut a, TrackTab::Clip);
    press(&mut a, Btn::Right);
    a.render(&mut fb);
}

#[test]
fn splitting_at_the_playhead_makes_two_clips_that_sound_exactly_like_the_original() {
    let (mut a, _f, mut clock) = clip_app();
    let whole = play_from_start(&mut a, &mut clock, 20_000);
    // park the playhead at frame 10 000 and split
    with(&mut a, Btn::Fn, Btn::B);
    clock.run(&mut a, 10_000);
    press(&mut a, Btn::Start);
    rows_down(&mut a, 8);
    press(&mut a, Btn::A);
    let clips = a.project.tracks[0].clips.clone();
    assert_eq!(clips.len(), 2, "{clips:?}");
    assert_eq!((clips[0].start, clips[0].len), (0, Some(10_000)));
    assert_eq!((clips[1].start, clips[1].offset), (10_000, clips[0].offset + 10_000));
    assert_eq!(a.clip_sel(), 1, "the new right-hand clip is picked");
    press(&mut a, Btn::Start); // play again
    let split = play_from_start(&mut a, &mut clock, 20_000);
    for p in [0usize, 5_000, 9_999, 10_000, 15_000, 19_999] {
        assert!((split[2 * p] - whole[2 * p]).abs() < 1e-6, "frame {p}");
    }
    // the split is saved in the take, undoable, and the clip count shows on screen
    assert_eq!(a.project.tracks[0].takes[0].clips.len(), 2);
    with(&mut a, Btn::Y, Btn::B);
    assert_eq!(a.project.tracks[0].clips.len(), 1);
    let mut fb = Fb::new();
    a.render(&mut fb);
}

#[test]
fn a_split_outside_the_clip_is_refused_and_a_reversed_clip_splits_into_the_right_halves() {
    let (mut a, _f, mut clock) = clip_app();
    with(&mut a, Btn::Fn, Btn::B); // playhead at 0: on the clip's first frame, not inside it
    rows_down(&mut a, 8);
    press(&mut a, Btn::A);
    assert_eq!(a.project.tracks[0].clips.len(), 1);
    assert!(a.toast_text().contains("inside"), "{}", a.toast_text());
    // reverse, then split in the middle: played back it must equal the reversed whole
    for _ in 0..1 {
        press(&mut a, Btn::Up);
    }
    press(&mut a, Btn::A); // row 7: reverse
    let whole = play_from_start(&mut a, &mut clock, 20_000);
    with(&mut a, Btn::Fn, Btn::B);
    clock.run(&mut a, 30_000);
    press(&mut a, Btn::Start);
    rows_down(&mut a, 1);
    press(&mut a, Btn::A);
    assert_eq!(a.project.tracks[0].clips.len(), 2);
    press(&mut a, Btn::Start);
    let split = play_from_start(&mut a, &mut clock, 20_000);
    for p in [0usize, 10_000, 19_999] {
        assert!((split[2 * p] - whole[2 * p]).abs() < 1e-6, "frame {p}: {} vs {}", split[2 * p], whole[2 * p]);
    }
}

#[test]
fn clip_gain_steps_in_half_decibels_and_the_clips_are_picked_and_deleted_with_confirmation() {
    let (mut a, _f, mut clock) = clip_app();
    rows_down(&mut a, 7); // gain row is 6; check it first
    for _ in 0..1 {
        press(&mut a, Btn::Up);
    }
    for _ in 0..4 {
        press(&mut a, Btn::Right); // +2 dB
    }
    let g = a.project.tracks[0].clips[0].gain;
    assert!((g - 10f32.powf(2.0 / 20.0)).abs() < 1e-3, "{g}");
    let out = play_from_start(&mut a, &mut clock, 2000);
    let want = (563 + 1000) as f32 * 1e-5 * PAN_CENTRE * g;
    assert!((out[2 * 1000] - want).abs() < 3e-5);
    // two clips: pick with row 0, delete with A twice
    with(&mut a, Btn::Fn, Btn::B);
    clock.run(&mut a, 10_000);
    press(&mut a, Btn::Start);
    rows_down(&mut a, 2); // row 8
    press(&mut a, Btn::A);
    assert_eq!(a.project.tracks[0].clips.len(), 2);
    for _ in 0..8 {
        press(&mut a, Btn::Up);
    }
    assert_eq!(a.clip_sel(), 1);
    press(&mut a, Btn::Left);
    assert_eq!(a.clip_sel(), 0);
    for _ in 0..9 {
        press(&mut a, Btn::Down);
    }
    press(&mut a, Btn::A);
    assert_eq!(a.project.tracks[0].clips.len(), 2, "one press only asks");
    press(&mut a, Btn::A);
    assert_eq!(a.project.tracks[0].clips.len(), 1);
    press(&mut a, Btn::A);
    press(&mut a, Btn::A);
    assert_eq!(a.project.tracks[0].clips.len(), 1, "the last clip stays");
    assert!(a.toast_text().contains("Last clip"), "{}", a.toast_text());
}
