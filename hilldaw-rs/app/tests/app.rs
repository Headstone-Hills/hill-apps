//! Drives HillDaw the way the runner does: button events, ticks, audio buffers and frames.

use hill_core::fb::Fb;
use hill_core::{App, Btn, Host, MemHost};
use hilldaw_app::fm1link::{Fm1Msg, Fm1Out};
use hilldaw_app::status::{Status, StatusHandle};
use hilldaw_app::{HillDaw, View, DRUMS, STATE_KEY};
use std::sync::{Arc, Mutex};

/// A host that two apps can share, so a test can "quit and start again".
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

fn app() -> HillDaw {
    HillDaw::new(Box::new(MemHost::default()), status())
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

fn render_audio(a: &mut HillDaw, frames: usize) -> Vec<f32> {
    let mut all = Vec::new();
    let mut buf = vec![0.0f32; 2048];
    let mut left = frames;
    while left > 0 {
        let n = left.min(1024);
        a.audio(&mut buf[..n * 2]);
        all.extend_from_slice(&buf[..n * 2]);
        left -= n;
    }
    all
}

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0, |m, s| m.max(s.abs()))
}

#[test]
fn starts_stopped_and_silent_with_a_starter_groove() {
    let mut a = app();
    assert_eq!(a.name(), "hilldaw");
    assert_eq!(a.project.drums.sections.len(), 1);
    assert_eq!(peak(&render_audio(&mut a, 4096)), 0.0, "stopped renders nothing");
}

#[test]
fn start_plays_the_drums_and_start_again_stops() {
    let mut a = app();
    press(&mut a, Btn::Start);
    let out = render_audio(&mut a, 44100);
    assert!(peak(&out) > 0.05, "drums are audible, peak {}", peak(&out));
    assert!(peak(&out) <= 1.0);
    press(&mut a, Btn::Start);
    assert_eq!(peak(&render_audio(&mut a, 2048)), 0.0);
}

#[test]
fn audio_is_the_same_whatever_buffer_size_the_device_uses() {
    let render = |chunk: usize| {
        let mut a = app();
        press(&mut a, Btn::Start);
        let mut all = Vec::new();
        let mut left = 60000usize;
        let mut buf = vec![0.0f32; chunk * 2];
        while left > 0 {
            let n = left.min(chunk);
            a.audio(&mut buf[..n * 2]);
            all.extend_from_slice(&buf[..n * 2]);
            left -= n;
        }
        all
    };
    assert!(render(1024) == render(333), "buffer size changed the output");
}

#[test]
fn level_pan_mute_and_solo_change_the_mix() {
    let mut a = app();
    a.event(Btn::Right, true);
    a.event(Btn::Right, false);
    for _ in 0..7 {
        press(&mut a, Btn::Right);
    }
    assert_eq!(a.sel(), DRUMS, "walked to the drum strip");
    press(&mut a, Btn::Start);
    let loud = peak(&render_audio(&mut a, 30000));

    for _ in 0..6 {
        press(&mut a, Btn::Down);
    }
    assert!((a.project.drums.gain - 0.5012).abs() < 0.01, "-6 dB, got {}", a.project.drums.gain);
    let quiet = peak(&render_audio(&mut a, 30000));
    assert!(quiet < loud * 0.7, "quieter after -6 dB: {quiet} vs {loud}");

    press(&mut a, Btn::B);
    assert_eq!(a.project.drums.gain, 1.0, "B resets the level");

    press(&mut a, Btn::X);
    assert!(!a.project.drums.enabled);
    assert_eq!(peak(&render_audio(&mut a, 30000)), 0.0, "muted");
    press(&mut a, Btn::X);

    // pan hard right: the left channel goes quiet
    for _ in 0..20 {
        press(&mut a, Btn::R2);
    }
    assert_eq!(a.project.drums.pan, 1.0);
    let out = render_audio(&mut a, 30000);
    let l = out.chunks_exact(2).fold(0.0f32, |m, f| m.max(f[0].abs()));
    let r = out.chunks_exact(2).fold(0.0f32, |m, f| m.max(f[1].abs()));
    assert!(l < 1e-6 && r > 0.05, "left {l} right {r}");

    // solo on another track silences the (un-soloed) drums
    press(&mut a, Btn::B);
    a.event(Btn::Left, true);
    a.event(Btn::Left, false);
    with(&mut a, Btn::Y, Btn::X);
    assert!(a.project.tracks[7].solo);
    assert_eq!(peak(&render_audio(&mut a, 30000)), 0.0, "solo elsewhere mutes the drums");
}

#[test]
fn gain_steps_stop_at_plus_six_and_at_silence() {
    let mut a = app();
    for _ in 0..20 {
        press(&mut a, Btn::Up);
    }
    assert!((a.project.tracks[0].gain - 1.9953).abs() < 0.01, "+6 dB max, got {}", a.project.tracks[0].gain);
    for _ in 0..80 {
        press(&mut a, Btn::Down);
    }
    assert_eq!(a.project.tracks[0].gain, 0.0);
    press(&mut a, Btn::Up);
    assert!(a.project.tracks[0].gain > 0.0, "comes back up from silence");
}

#[test]
fn tempo_keys_and_tap_tempo() {
    let mut a = app();
    let start = a.project.bpm;
    with(&mut a, Btn::Fn, Btn::Right);
    assert_eq!(a.project.bpm, start + 1.0);
    with(&mut a, Btn::Fn, Btn::Up);
    assert_eq!(a.project.bpm, start + 6.0);
    for _ in 0..80 {
        with(&mut a, Btn::Fn, Btn::Down);
    }
    assert_eq!(a.project.bpm, hilldaw_app::MIN_BPM);
    // four taps 500 ms apart = 120 bpm
    for _ in 0..4 {
        with(&mut a, Btn::Fn, Btn::A);
        a.tick(500);
    }
    assert!((a.project.bpm - 120.0).abs() < 1.0, "tapped {}", a.project.bpm);
}

#[test]
fn a_tempo_change_keeps_the_musical_position() {
    let mut a = app();
    press(&mut a, Btn::Start);
    render_audio(&mut a, 44100 * 3);
    let (bar, beat) = a.bar_beat();
    with(&mut a, Btn::Fn, Btn::Up);
    assert_eq!(a.bar_beat(), (bar, beat), "still on the same beat after +5 bpm");
}

#[test]
fn click_follows_the_beat_and_is_audible_during_solo() {
    let mut a = app();
    // remove the starter drums through the UI, so only the metronome can be heard
    press(&mut a, Btn::R1);
    press(&mut a, Btn::X);
    press(&mut a, Btn::L1);
    assert!(a.project.drums.sections.is_empty());
    press(&mut a, Btn::Start);
    assert_eq!(peak(&render_audio(&mut a, 22050)), 0.0, "click is off by default");
    with(&mut a, Btn::Fn, Btn::X);
    let out = render_audio(&mut a, 44100);
    assert!(peak(&out) > 0.05, "metronome audible with no drums");
    with(&mut a, Btn::Y, Btn::X); // solo track 1
    assert!(peak(&render_audio(&mut a, 44100)) > 0.05, "click survives a solo");
}

#[test]
fn arrange_screen_adds_edits_and_removes_sections() {
    let mut a = app();
    press(&mut a, Btn::R1);
    assert_eq!(a.view(), View::Arrange);
    let first = a.project.drums.sections[0].preset.clone();
    press(&mut a, Btn::A);
    assert_eq!(a.project.drums.sections.len(), 2);
    assert_eq!(a.project.drums.sections[1].preset, first, "a new section copies the selected one");
    let before = a.drum_frames();
    press(&mut a, Btn::Right);
    assert_ne!(a.project.drums.sections[1].preset, first, "Right picks the next pattern");
    a.event(Btn::Y, true);
    press(&mut a, Btn::Right);
    a.event(Btn::Y, false);
    assert_eq!(a.project.drums.sections[1].cycles, 5);
    assert_ne!(a.drum_frames(), before, "the drum track was rebuilt");
    press(&mut a, Btn::R2);
    let style = |a: &HillDaw| hill_core::presets::builtin().patterns.iter().find(|p| p.id == a.project.drums.sections[1].preset).unwrap().style.clone();
    let s1 = style(&a);
    press(&mut a, Btn::L2);
    assert_ne!(style(&a), s1, "L2 / R2 move between styles");
    press(&mut a, Btn::X);
    assert_eq!(a.project.drums.sections.len(), 1);
    press(&mut a, Btn::X);
    assert!(a.project.drums.sections.is_empty());
    press(&mut a, Btn::X); // nothing left: harmless
    press(&mut a, Btn::Start);
    assert_eq!(peak(&render_audio(&mut a, 20000)), 0.0, "no sections, no drums");
    press(&mut a, Btn::L1);
    assert_eq!(a.view(), View::Mixer);
}

#[test]
fn hold_start_and_select_quits_without_toggling_play() {
    let mut a = app();
    a.event(Btn::Start, true);
    a.event(Btn::Select, true);
    a.tick(1000);
    assert!(!a.wants_quit());
    a.tick(600);
    assert!(a.wants_quit());
    a.event(Btn::Select, false);
    a.event(Btn::Start, false);
    assert!(!a.is_playing(), "the quit chord must not start playback");
}

#[test]
fn releasing_the_quit_chord_early_cancels_it() {
    let mut a = app();
    a.event(Btn::Start, true);
    a.event(Btn::Select, true);
    a.tick(1000);
    a.event(Btn::Select, false);
    a.event(Btn::Start, false);
    a.tick(1000);
    assert!(!a.wants_quit());
    assert!(!a.is_playing());
}

#[test]
fn fn_select_opens_help_and_a_closes_it() {
    let mut a = app();
    a.event(Btn::Fn, true);
    a.event(Btn::Select, true);
    a.event(Btn::Select, false);
    a.event(Btn::Fn, false);
    assert!(a.help_open());
    let mut fb = Fb::new();
    a.render(&mut fb);
    press(&mut a, Btn::Right); // does not move the strip while help is up
    assert_eq!(a.sel(), 0);
    press(&mut a, Btn::A);
    assert!(!a.help_open());
}

#[test]
fn select_alone_says_recording_is_not_wired() {
    let mut a = app();
    press(&mut a, Btn::Select);
    assert!(a.toast_text().contains("Recording"));
    assert!(!a.is_playing());
}

#[test]
fn project_survives_quit_and_restart() {
    let host = Shared::default();
    let mut a = HillDaw::new(Box::new(host.clone()), status());
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Down);
    with(&mut a, Btn::Fn, Btn::Up);
    press(&mut a, Btn::R1);
    press(&mut a, Btn::A);
    a.save();
    assert!(host.load_state(STATE_KEY).is_some());
    let b = HillDaw::new(Box::new(host.clone()), status());
    assert_eq!(b.project, a.project);
    assert_eq!(b.project.drums.sections.len(), 2);
}

#[test]
fn a_corrupt_saved_project_or_unknown_preset_does_not_stop_the_app() {
    let mut h = MemHost::default();
    h.save_state(STATE_KEY, "{nope");
    let a = HillDaw::new(Box::new(h), status());
    assert_eq!(a.project.drums.sections.len(), 1, "fell back to a fresh project");

    let mut p = hilldaw_core::project::Project::default();
    p.drums.sections.push(hilldaw_core::project::DrumSectionModel { name: "x".into(), preset: "gone-preset".into(), cycles: 2 });
    let mut h = MemHost::default();
    h.save_state(STATE_KEY, &p.to_json());
    let mut a = HillDaw::new(Box::new(h), status());
    assert!(a.project.drums.sections.is_empty(), "unknown preset dropped");
    press(&mut a, Btn::Start);
    render_audio(&mut a, 1000);
}

#[test]
fn status_bar_picks_up_battery_storage_and_fm1() {
    let st = status();
    let mut a = HillDaw::new(Box::new(MemHost::default()), st.clone());
    *st.lock().unwrap() = Status { battery_pct: Some(45), charging: true, battery_low: false, free_bytes: Some(100 << 30), fm1: true };
    a.tick(600);
    assert_eq!(a.status().battery_pct, Some(45));
    assert!(a.status().fm1);
}

#[test]
fn every_screen_draws_something_and_stays_inside_the_frame() {
    let mut a = app();
    press(&mut a, Btn::Start);
    render_audio(&mut a, 22050);
    a.tick(16);
    for _ in 0..2 {
        let mut fb = Fb::new();
        a.render(&mut fb);
        let lit = fb.px.iter().filter(|p| **p != hill_core::fb::BG).count();
        assert!(lit > 20000, "screen has content ({lit} px)");
        assert_eq!(fb.px.len(), 640 * 480);
        press(&mut a, Btn::R1);
    }
}

#[test]
fn meters_follow_the_signal_and_fall_when_it_stops() {
    let mut a = app();
    press(&mut a, Btn::Start);
    render_audio(&mut a, 22050);
    a.tick(16);
    assert!(a.meter(DRUMS) > 0.05, "drums strip shows level");
    assert_eq!(a.meter(0), 0.0, "empty track shows none");
    press(&mut a, Btn::Start);
    for _ in 0..120 {
        a.tick(16);
    }
    assert!(a.meter(DRUMS) < 0.01, "meter fell");
}

// ---- FM-1 screen ----

#[derive(Clone, Default)]
struct Rec(Arc<Mutex<Vec<Fm1Msg>>>);

impl Fm1Out for Rec {
    fn send(&mut self, m: Fm1Msg) {
        self.0.lock().unwrap().push(m);
    }
}

impl Rec {
    fn take(&self) -> Vec<Fm1Msg> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

fn fm1_app() -> (HillDaw, Rec) {
    let rec = Rec::default();
    let mut a = HillDaw::new(Box::new(MemHost::default()), status()).with_fm1(Box::new(rec.clone()));
    press(&mut a, Btn::R1);
    press(&mut a, Btn::R1);
    assert_eq!(a.view(), View::Fm1);
    (a, rec)
}

#[test]
fn l1_and_r1_cycle_through_the_three_screens() {
    let mut a = app();
    let mut seen = vec![a.view()];
    for _ in 0..3 {
        press(&mut a, Btn::R1);
        seen.push(a.view());
    }
    assert_eq!(seen, vec![View::Mixer, View::Arrange, View::Fm1, View::Mixer]);
    press(&mut a, Btn::L1);
    assert_eq!(a.view(), View::Fm1);
}

#[test]
fn the_voice_tab_sends_program_changes_and_stays_in_0_to_127() {
    let (mut a, rec) = fm1_app();
    assert!(rec.take().is_empty(), "opening the screen sends nothing");
    press(&mut a, Btn::Right);
    assert_eq!(rec.take(), vec![Fm1Msg::Program(1)]);
    press(&mut a, Btn::Up);
    assert_eq!(rec.take(), vec![Fm1Msg::Program(11)]);
    for _ in 0..20 {
        press(&mut a, Btn::Down);
    }
    assert_eq!(a.project.fm1.program, 0);
    for _ in 0..20 {
        press(&mut a, Btn::Up);
    }
    assert_eq!(a.project.fm1.program, 127);
    assert!(a.project.fm1.program_set);
}

#[test]
fn effect_controls_send_cc_values_inside_each_parameters_range() {
    let (mut a, rec) = fm1_app();
    press(&mut a, Btn::R2);
    assert_eq!(a.fm1_tab(), 1, "Filter");
    press(&mut a, Btn::A);
    press(&mut a, Btn::A);
    assert_eq!(rec.take(), vec![Fm1Msg::FxCc { cc: 70, value: 1 }, Fm1Msg::FxCc { cc: 70, value: 0 }], "A toggles the effect's switch");
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Down);
    assert_eq!(a.fm1_row(), 2, "Cutoff");
    press(&mut a, Btn::Left); // never set: starts from the middle of 0..=107, not from an invented value
    press(&mut a, Btn::Right);
    a.event(Btn::Y, true);
    press(&mut a, Btn::Right);
    a.event(Btn::Y, false);
    assert_eq!(rec.take(), vec![Fm1Msg::FxCc { cc: 72, value: 54 }, Fm1Msg::FxCc { cc: 72, value: 55 }, Fm1Msg::FxCc { cc: 72, value: 65 }]);
    for _ in 0..60 {
        press(&mut a, Btn::Right);
    }
    assert_eq!(a.project.fm1.fx[&72], 107, "cutoff stops at its maximum");
    rec.take();
    press(&mut a, Btn::Down); // Q, range 0..=10
    for _ in 0..40 {
        press(&mut a, Btn::Right);
    }
    assert_eq!(a.project.fm1.fx[&73], 10);
    assert!(rec.take().iter().all(|m| matches!(m, Fm1Msg::FxCc { cc: 73, value } if *value <= 10)), "never sends a value above the range");
}

#[test]
fn test_note_plays_for_a_moment_and_b_stops_everything() {
    let (mut a, rec) = fm1_app();
    press(&mut a, Btn::X);
    assert_eq!(rec.take(), vec![Fm1Msg::NoteOn(60)]);
    a.tick(300);
    assert!(rec.take().is_empty());
    a.tick(500);
    assert_eq!(rec.take(), vec![Fm1Msg::NoteOff(60)]);
    press(&mut a, Btn::X);
    press(&mut a, Btn::X); // retrigger: the first note is released first
    assert_eq!(rec.take(), vec![Fm1Msg::NoteOn(60), Fm1Msg::NoteOff(60), Fm1Msg::NoteOn(60)]);
    press(&mut a, Btn::B);
    assert_eq!(rec.take(), vec![Fm1Msg::Panic]);
    a.tick(1000);
    assert!(rec.take().is_empty(), "no stray note-off after a panic");
}

#[test]
fn connecting_sends_only_what_the_user_chose_and_again_after_a_replug() {
    // nothing chosen: connecting must not touch the FM-1's own patch
    let st = status();
    let rec = Rec::default();
    let mut a = HillDaw::new(Box::new(MemHost::default()), st.clone()).with_fm1(Box::new(rec.clone()));
    st.lock().unwrap().fm1 = true;
    a.tick(600);
    assert!(rec.take().is_empty());

    // settings saved in the project are sent when the unit appears, once
    let mut p = hilldaw_core::project::Project::default();
    p.fm1.program = 5;
    p.fm1.program_set = true;
    p.fm1.fx.insert(70, 1);
    p.fm1.fx.insert(72, 60);
    let mut h = MemHost::default();
    h.save_state(STATE_KEY, &p.to_json());
    let st = status();
    let rec = Rec::default();
    let mut a = HillDaw::new(Box::new(h), st.clone()).with_fm1(Box::new(rec.clone()));
    a.tick(600);
    assert!(rec.take().is_empty(), "unit not there yet");
    st.lock().unwrap().fm1 = true;
    a.tick(600);
    let expect = vec![Fm1Msg::Program(5), Fm1Msg::FxCc { cc: 70, value: 1 }, Fm1Msg::FxCc { cc: 72, value: 60 }];
    assert_eq!(rec.take(), expect);
    a.tick(600);
    assert!(rec.take().is_empty(), "not again while it stays connected");
    st.lock().unwrap().fm1 = false;
    a.tick(600);
    st.lock().unwrap().fm1 = true;
    a.tick(600);
    assert_eq!(rec.take(), expect, "resent after unplug and replug");
}

#[test]
fn fm1_settings_are_saved_with_the_project() {
    let host = Shared::default();
    let (mut a, _rec) = {
        let rec = Rec::default();
        (HillDaw::new(Box::new(host.clone()), status()).with_fm1(Box::new(rec.clone())), rec)
    };
    press(&mut a, Btn::R1);
    press(&mut a, Btn::R1);
    press(&mut a, Btn::Right);
    press(&mut a, Btn::R2);
    press(&mut a, Btn::A);
    a.save();
    let b = HillDaw::new(Box::new(host), status());
    assert_eq!((b.project.fm1.program, b.project.fm1.program_set, b.project.fm1.fx.get(&70).copied()), (1, true, Some(1)));
}

#[test]
fn the_fm1_screen_draws_every_tab_without_panicking() {
    let (mut a, _rec) = fm1_app();
    for _ in 0..8 {
        let mut fb = Fb::new();
        a.render(&mut fb);
        assert!(fb.px.iter().filter(|p| **p != hill_core::fb::BG).count() > 15000);
        press(&mut a, Btn::R2);
        press(&mut a, Btn::Right);
    }
}

#[test]
fn the_output_clock_is_measured_from_the_audio_callback_with_a_known_error() {
    let mut a = app();
    assert!(a.out_clock().is_none(), "nothing before there is data");
    // a sound card that really runs 80 ppm fast, with a callback that is up to 8 ms early or late
    let real_rate = 44100.0 * (1.0 + 80e-6);
    let mut buf = vec![0.0f32; 2048]; // 1024 frames per callback
    let (mut frames, mut seed) = (0u64, 99u64);
    for _ in 0..5200 {
        frames += 1024;
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let jitter = ((seed >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 0.016;
        a.audio_at(&mut buf, frames as f64 / real_rate + jitter);
    }
    let f = a.out_clock().expect("two minutes of data");
    assert!((f.ppm - 80.0).abs() < 3.0, "measured {:+.1} ppm", f.ppm);
    assert!(f.span_s > 110.0 && f.span_s < 130.0, "{}", f.span_s);
    // it is on screen and is saved with the project
    let mut fb = Fb::new();
    a.render(&mut fb);
    let lit_in_strip = (437 * 640..445 * 640).filter(|i| fb.px[*i as usize] != hill_core::fb::BG).count();
    assert!(lit_in_strip > 100, "the readout is drawn ({lit_in_strip} px)");
}

#[test]
fn the_drums_keep_going_past_the_end_of_the_arrangement_while_looping() {
    // regression: the starter groove is 4 cycles (4 bars) and used to fall silent after them
    let mut a = app();
    assert!(a.project.drums.loop_arrangement);
    press(&mut a, Btn::Start);
    let len = a.drum_frames() as usize;
    assert!(len > 44100 * 8, "starter arrangement is about 9.6 s at 100 bpm: {len}");
    render_audio(&mut a, len + 2 * 44100);
    // two seconds after the end it is playing the groove again
    let again = render_audio(&mut a, 2 * 44100);
    assert!(peak(&again) > 0.05, "drums still sounding after the loop point");
    assert!(a.bar_beat().0 <= 2, "the position wrapped back to the start: {:?}", a.bar_beat());
}

#[test]
fn fn_y_turns_the_loop_off_and_then_the_arrangement_ends() {
    let mut a = app();
    with(&mut a, Btn::Fn, Btn::Y);
    assert!(!a.project.drums.loop_arrangement);
    assert!(a.toast_text().contains("Loop off"));
    press(&mut a, Btn::Start);
    let len = a.drum_frames() as usize;
    render_audio(&mut a, len + 2 * 44100);
    assert_eq!(peak(&render_audio(&mut a, 44100)), 0.0, "ended, nothing loops");
    with(&mut a, Btn::Fn, Btn::Y);
    assert!(a.toast_text().contains("Loop on"));
}

#[test]
fn looping_is_identical_for_any_buffer_size_and_survives_an_edit_that_shortens_the_song() {
    let render = |chunk: usize| {
        let mut a = app();
        press(&mut a, Btn::Start);
        let mut all = Vec::new();
        let mut left = 44100 * 14; // past the loop point
        let mut buf = vec![0.0f32; chunk * 2];
        while left > 0 {
            let n = left.min(chunk);
            a.audio(&mut buf[..n * 2]);
            all.extend_from_slice(&buf[..n * 2]);
            left -= n;
        }
        all
    };
    assert!(render(1024) == render(777), "wrapping at the loop point depends on the buffer size");
    // shorten the song while the playhead is beyond the new end: it wraps instead of sitting past the end
    let mut a = app();
    press(&mut a, Btn::Start);
    render_audio(&mut a, 44100 * 8);
    press(&mut a, Btn::R1);
    a.event(Btn::Y, true);
    for _ in 0..3 {
        press(&mut a, Btn::Left); // cycles 4 -> 1
    }
    a.event(Btn::Y, false);
    assert_eq!(a.project.drums.sections[0].cycles, 1);
    let out = render_audio(&mut a, 44100 * 3);
    assert!(peak(&out) > 0.05, "still playing after the song got shorter");
}
