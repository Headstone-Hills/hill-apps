//! Undo and redo: every kind of edit comes back exactly, bursts merge, no-ops leave no step, and what is built from the project follows.

use hill_core::{App, Btn, MemHost};
use hilldaw_app::fm1link::{Fm1Msg, Fm1Out};
use hilldaw_app::recorder::{RecInfo, Recorder};
use hilldaw_app::status::{Status, StatusHandle};
use hilldaw_app::{HillDaw, TrackTab, View};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const R: f64 = 44100.0;

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
    for _ in 0..4 {
        if a.track_tab() == t {
            return;
        }
        press(a, if (a.track_tab() as usize) < (t as usize) { Btn::R2 } else { Btn::L2 });
    }
    panic!("never reached {t:?}");
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn play_3s(a: &mut HillDaw) -> Vec<f32> {
    to_view(a, View::Mixer);
    press(a, Btn::Start);
    render_3s(a)
}

/// Render 3 s of whatever the transport is doing (no START press).
fn render_3s(a: &mut HillDaw) -> Vec<f32> {
    let mut all = Vec::new();
    let mut buf = vec![0.0f32; 2048];
    for k in 0..(3 * 44100 / 1024) {
        a.audio_at(&mut buf, k as f64 * 1024.0 / R);
        all.extend_from_slice(&buf);
    }
    all
}

#[derive(Clone, Default)]
struct Rec(Arc<Mutex<Vec<Fm1Msg>>>);

impl Fm1Out for Rec {
    fn send(&mut self, m: Fm1Msg) {
        self.0.lock().unwrap().push(m);
    }
}

fn app() -> HillDaw {
    HillDaw::new(Box::new(MemHost::default()), status())
}

/// One edit: after it the project differs, undo gives back exactly the old project (export options aside), redo exactly the new one.
fn step(a: &mut HillDaw, name: &str, act: impl FnOnce(&mut HillDaw)) {
    a.tick(1000); // outside the merge window of whatever came before
    let before = a.project.clone();
    act(a);
    a.tick(20);
    let after = a.project.clone();
    assert!(after != before, "{name}: the action changed nothing");
    a.undo();
    let mut want = before.clone();
    want.export = a.project.export.clone();
    assert!(a.project == want, "{name}: undo did not restore the project");
    a.redo();
    assert!(a.project == after, "{name}: redo did not bring the edit back");
}

#[test]
fn every_kind_of_edit_can_be_undone_and_redone_exactly() {
    let mut a = HillDaw::new(Box::new(MemHost::default()), status()).with_fm1(Box::new(Rec::default()));
    step(&mut a, "level", |a| {
        to_view(a, View::Mixer);
        press(a, Btn::Down);
    });
    step(&mut a, "pan", |a| press(a, Btn::R2));
    step(&mut a, "mute", |a| press(a, Btn::X));
    step(&mut a, "solo", |a| with(a, Btn::Y, Btn::X));
    step(&mut a, "tempo", |a| with(a, Btn::Fn, Btn::Right));
    step(&mut a, "swing", |a| with(a, Btn::Fn, Btn::R2));
    step(&mut a, "drum loop", |a| with(a, Btn::Fn, Btn::Y));
    step(&mut a, "count-in", |a| with(a, Btn::Fn, Btn::L1));
    step(&mut a, "drum pattern", |a| {
        to_view(a, View::Arrange);
        press(a, Btn::Right);
    });
    step(&mut a, "drum style", |a| press(a, Btn::R2));
    step(&mut a, "cycles", |a| with(a, Btn::Y, Btn::Right));
    step(&mut a, "add section", |a| press(a, Btn::A));
    step(&mut a, "delete section", |a| press(a, Btn::X));
    step(&mut a, "FM-1 voice", |a| {
        to_view(a, View::Fm1);
        press(a, Btn::Right);
    });
    step(&mut a, "FM-1 effect", |a| {
        press(a, Btn::R2);
        press(a, Btn::A);
    });
    step(&mut a, "track effect", |a| {
        to_tab(a, TrackTab::Fx);
        press(a, Btn::Left);
    });
    step(&mut a, "reverb room", |a| {
        for _ in 0..6 {
            press(a, Btn::Down);
        }
        press(a, Btn::Right);
    });
    step(&mut a, "loop", |a| {
        to_tab(a, TrackTab::Loop);
        while a.track_row() > 0 {
            press(a, Btn::Up);
        }
        press(a, Btn::Right);
    });
    step(&mut a, "punch", |a| {
        for _ in 0..3 {
            press(a, Btn::Down);
        }
        press(a, Btn::Right);
    });
    step(&mut a, "latency", |a| {
        to_tab(a, TrackTab::Input);
        while a.track_row() > 1 {
            press(a, Btn::Up);
        }
        while a.track_row() < 1 {
            press(a, Btn::Down);
        }
        press(a, Btn::Right);
    });
    step(&mut a, "input count-in", |a| {
        press(a, Btn::Down);
        press(a, Btn::Right);
    });
    assert!(a.undo_depth() >= 19, "{}", a.undo_depth());
}

#[test]
fn a_burst_of_the_same_edit_is_one_step_and_a_later_one_is_another() {
    let mut a = app();
    for _ in 0..6 {
        press(&mut a, Btn::Down); // hold-to-repeat on a level
        a.tick(70);
    }
    assert!((20.0 * a.project.tracks[0].gain.log10() + 6.0).abs() < 0.1, "six -1 dB steps");
    a.tick(1000);
    for _ in 0..3 {
        press(&mut a, Btn::Down);
        a.tick(70);
    }
    assert_eq!(a.undo_depth(), 2, "two bursts, two steps");
    a.undo();
    assert!((20.0 * a.project.tracks[0].gain.log10() + 6.0).abs() < 0.1, "back to -6 dB, not -1");
    a.undo();
    assert_eq!(a.project.tracks[0].gain, 1.0, "back to the start in one more step");
    a.redo();
    a.redo();
    assert!((20.0 * a.project.tracks[0].gain.log10() + 9.0).abs() < 0.1, "both bursts redone");
    // different strips or different kinds of edit never merge
    a.tick(1000);
    let depth = a.undo_depth();
    press(&mut a, Btn::Right);
    press(&mut a, Btn::Down);
    a.tick(70);
    press(&mut a, Btn::R2); // a pan right after a level: separate
    assert_eq!(a.undo_depth(), depth + 2);
}

#[test]
fn presses_that_change_nothing_leave_no_step_and_a_new_edit_clears_redo() {
    let mut a = app();
    press(&mut a, Btn::B); // reset a level that is already at its default
    press(&mut a, Btn::Up);
    for _ in 0..40 {
        press(&mut a, Btn::Up); // up against the +24 dB limit: the extra presses change nothing
    }
    a.tick(1000);
    let after_limit = a.undo_depth();
    for _ in 0..5 {
        press(&mut a, Btn::Up);
    }
    assert_eq!(a.undo_depth(), after_limit, "pressing against the limit adds no step");
    assert!(!a.can_redo());
    a.undo();
    assert!(a.can_redo());
    a.tick(1000);
    press(&mut a, Btn::X); // a new edit
    assert!(!a.can_redo(), "a new edit ends the redo history");
    a.redo();
    assert!(a.toast_text().contains("Nothing to redo"), "{}", a.toast_text());
    let mut empty = app();
    empty.undo();
    assert!(empty.toast_text().contains("Nothing to undo") && !empty.can_undo());
}

#[test]
fn y_plus_b_undoes_and_y_plus_a_redoes_without_also_doing_what_b_and_a_normally_do() {
    let mut a = app();
    a.event(Btn::Y, true);
    press(&mut a, Btn::Up); // Y+Up: +6 dB
    a.event(Btn::Y, false);
    assert!(a.project.tracks[0].gain > 1.9);
    a.event(Btn::Y, true);
    press(&mut a, Btn::B); // normally "reset level", here "undo"
    a.event(Btn::Y, false);
    assert_eq!(a.project.tracks[0].gain, 1.0);
    assert!(a.toast_text().contains("Undo: level"), "{}", a.toast_text());
    assert!(a.can_redo(), "it was an undo, not a reset");
    a.event(Btn::Y, true);
    press(&mut a, Btn::A); // normally "arm", here "redo"
    a.event(Btn::Y, false);
    assert!(a.project.tracks[0].gain > 1.9);
    assert!(a.toast_text().contains("Redo: level"), "{}", a.toast_text());
    // A and B on their own still do their usual things
    press(&mut a, Btn::B);
    assert_eq!(a.project.tracks[0].gain, 1.0, "B alone resets");
    press(&mut a, Btn::A);
    assert!(a.toast_text().contains("armed"), "{}", a.toast_text());
}

#[test]
fn what_is_built_from_the_project_follows_an_undo_the_sound_comes_back_exactly() {
    let base = play_3s(&mut app());
    assert!(rms(&base) > 0.01);

    // a drum pattern change and its undo: the same audio as never having touched it
    let mut a = app();
    to_view(&mut a, View::Arrange);
    press(&mut a, Btn::Right);
    a.tick(100);
    a.undo();
    assert!(play_3s(&mut a) == base, "the drums are the original pattern again after the undo");

    // a tempo change and its undo
    let mut a = app();
    with(&mut a, Btn::Fn, Btn::Up);
    assert!(a.project.bpm > 100.0);
    a.tick(100);
    a.undo();
    assert_eq!(a.project.bpm, 100.0);
    assert!(play_3s(&mut a) == base, "the tempo is back everywhere");

    // an effect and its undo: filter state and all
    let mut a = app();
    for _ in 0..8 {
        press(&mut a, Btn::Right);
    }
    to_tab(&mut a, TrackTab::Fx);
    for _ in 0..30 {
        press(&mut a, Btn::Left);
    }
    assert!(a.project.drums.fx.lowpass_hz > 0);
    a.tick(100);
    let filtered = play_3s(&mut a);
    assert!(filtered != base);
    press(&mut a, Btn::Start);
    let mut b = app();
    for _ in 0..8 {
        press(&mut b, Btn::Right);
    }
    to_tab(&mut b, TrackTab::Fx);
    for _ in 0..30 {
        press(&mut b, Btn::Left);
    }
    b.tick(100);
    b.undo();
    assert!(b.project.drums.fx.is_off());
    assert!(play_3s(&mut b) == base, "no filter after the undo");

    // mute and its undo
    let mut a = app();
    for _ in 0..8 {
        press(&mut a, Btn::Right);
    }
    press(&mut a, Btn::X);
    assert!(rms(&play_3s(&mut a)) == 0.0, "muted drums are silent");
    a.undo(); // still playing: the drums come back at once
    assert!(a.project.drums.enabled);
    with(&mut a, Btn::Fn, Btn::B);
    assert!(rms(&render_3s(&mut a)) > 0.01, "unmuted by the undo, and audible");
}

#[test]
fn undo_resends_the_fm1_settings_it_puts_back() {
    let out = Rec::default();
    let mut a = HillDaw::new(Box::new(MemHost::default()), status()).with_fm1(Box::new(out.clone()));
    to_view(&mut a, View::Fm1);
    press(&mut a, Btn::Right); // voice 2 (program 1)
    a.tick(1000);
    for _ in 0..4 {
        press(&mut a, Btn::Right);
    }
    assert_eq!(a.project.fm1.program, 5);
    out.0.lock().unwrap().clear();
    a.undo();
    assert_eq!(a.project.fm1.program, 1);
    assert_eq!(*out.0.lock().unwrap(), vec![Fm1Msg::Program(1)], "the unit is told to go back");
    out.0.lock().unwrap().clear();
    a.redo();
    assert_eq!(*out.0.lock().unwrap(), vec![Fm1Msg::Program(5)]);

    // an effect control: from 54 (the middle) to 55 and back
    a.tick(1000);
    press(&mut a, Btn::R2);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Right); // Cutoff: first touch sets the middle
    a.tick(1000);
    press(&mut a, Btn::Right);
    out.0.lock().unwrap().clear();
    a.undo();
    assert_eq!(*out.0.lock().unwrap(), vec![Fm1Msg::FxCc { cc: 72, value: 54 }]);
}

#[derive(Default)]
struct Files(BTreeMap<String, u64>, usize);

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Files>>);

impl Recorder for Fake {
    fn start(&mut self) -> Result<String, String> {
        let mut s = self.0.lock().unwrap();
        s.1 += 1;
        Ok(format!("audio/rec-{:03}.wav", s.1))
    }
    fn frames(&self) -> u64 {
        0
    }
    fn finish(&mut self, min: u64) -> Result<RecInfo, String> {
        let mut s = self.0.lock().unwrap();
        let n = format!("audio/rec-{:03}.wav", s.1);
        s.0.insert(n, min + 500);
        Ok(RecInfo { frames: min + 500, overruns: 0 })
    }
    fn load(&mut self, rel: &str) -> Option<Vec<f32>> {
        let n = *self.0.lock().unwrap().0.get(rel)?;
        Some((0..n).map(|i| i as f32 * 1e-5).collect())
    }
}

fn recording_app() -> (HillDaw, Fake) {
    let fake = Fake::default();
    let mut a = HillDaw::new(Box::new(MemHost::default()), status()).with_recorder(Box::new(fake.clone()));
    to_view(&mut a, View::Arrange);
    press(&mut a, Btn::X); // no drums: only the recording sounds
    to_view(&mut a, View::Mixer);
    with(&mut a, Btn::Fn, Btn::L1); // count-in off
    press(&mut a, Btn::A);
    a.set_test_clock(Some(0.0));
    (a, fake)
}

fn record(a: &mut HillDaw, secs: f64) {
    with(a, Btn::Fn, Btn::B);
    press(a, Btn::Select);
    let mut buf = vec![0.0f32; 2048];
    let mut done = 0u64;
    while (done as f64) < secs * R {
        a.audio_at(&mut buf, done as f64 / R);
        done += 1024;
    }
    press(a, Btn::Select);
}

fn play_clip(a: &mut HillDaw) -> f32 {
    to_view(a, View::Mixer);
    with(a, Btn::Fn, Btn::B);
    let mut buf = vec![0.0f32; 2048];
    for k in 0..4 {
        a.audio_at(&mut buf, 100.0 + k as f64);
    }
    rms(&buf)
}

#[test]
fn a_recording_is_one_undo_step_that_removes_the_take_and_redo_brings_it_back() {
    let (mut a, _f) = recording_app();
    record(&mut a, 1.0);
    assert_eq!(a.project.tracks[0].takes.len(), 1);
    assert!(play_clip(&mut a) > 0.0001, "the take plays");
    a.tick(1000);
    record(&mut a, 1.0);
    assert_eq!(a.project.tracks[0].takes.len(), 2);

    a.undo();
    assert_eq!((a.project.tracks[0].takes.len(), a.project.tracks[0].active_take), (1, Some(0)), "the second take is gone, the first plays");
    assert_eq!(a.project.tracks[0].clips[0].file, "audio/rec-001.wav");
    a.undo();
    assert!(a.project.tracks[0].takes.is_empty() && a.project.tracks[0].clips.is_empty());
    assert_eq!(play_clip(&mut a), 0.0, "an undone recording is silent");
    a.redo();
    a.redo();
    assert_eq!(a.project.tracks[0].takes.len(), 2, "both takes back");
    assert!(play_clip(&mut a) > 0.0001, "and playing again: the audio files were never touched");
}

#[test]
fn choosing_and_removing_takes_are_steps_too() {
    let (mut a, _f) = recording_app();
    record(&mut a, 1.0);
    a.tick(1000);
    record(&mut a, 1.0);
    a.tick(1000);
    to_tab(&mut a, TrackTab::Takes);
    press(&mut a, Btn::A); // take 1 plays
    assert_eq!(a.project.tracks[0].active_take, Some(0));
    a.undo();
    assert_eq!(a.project.tracks[0].active_take, Some(1), "back to the take that was playing");
    a.tick(1000);
    press(&mut a, Btn::X);
    press(&mut a, Btn::X); // remove take 1
    assert_eq!(a.project.tracks[0].takes.len(), 1);
    a.undo();
    assert_eq!(a.project.tracks[0].takes.len(), 2, "removed take back in the list");
    assert!(a.take_sel() < 2);
}

#[test]
fn undo_is_refused_while_recording_and_the_history_is_capped_at_100() {
    let (mut a, _f) = recording_app();
    press(&mut a, Btn::Down);
    a.tick(1000);
    press(&mut a, Btn::Select); // recording now
    press(&mut a, Btn::Right);
    a.event(Btn::Y, true);
    press(&mut a, Btn::B);
    a.event(Btn::Y, false);
    assert!(a.toast_text().contains("Stop recording first"), "{}", a.toast_text());
    press(&mut a, Btn::Select);

    let mut b = app();
    for i in 0..150 {
        b.tick(1000);
        press(&mut b, if i % 2 == 0 { Btn::Down } else { Btn::Up });
    }
    assert_eq!(b.undo_depth(), 100, "the oldest steps fall off");
    for _ in 0..100 {
        b.undo();
    }
    b.undo();
    assert!(b.toast_text().contains("Nothing to undo"));
}

#[test]
fn export_options_are_settings_not_edits_so_undo_leaves_them_alone() {
    let mut a = app();
    press(&mut a, Btn::Down);
    a.tick(1000);
    to_view(&mut a, View::Export);
    press(&mut a, Btn::Right); // format
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Right); // normalize on
    let (format, norm) = (a.project.export.format, a.project.export.normalize);
    assert!(norm);
    assert_eq!(a.undo_depth(), 1, "changing export options added no step");
    a.undo();
    assert_eq!((a.project.export.format, a.project.export.normalize), (format, norm), "undoing the level did not undo the export options");
    assert_eq!(a.project.tracks[0].gain, 1.0);
}

#[test]
fn selections_stay_valid_after_an_undo_and_the_saved_project_is_the_undone_one() {
    let mut a = app();
    to_view(&mut a, View::Arrange);
    press(&mut a, Btn::A);
    press(&mut a, Btn::A);
    press(&mut a, Btn::A);
    assert_eq!(a.project.drums.sections.len(), 4);
    a.tick(100);
    for _ in 0..3 {
        a.undo();
    }
    assert_eq!(a.project.drums.sections.len(), 1);
    let mut fb = hill_core::fb::Fb::new();
    a.render(&mut fb); // the selected section is clamped: this must not panic
    press(&mut a, Btn::Left);
    press(&mut a, Btn::Down);
}
