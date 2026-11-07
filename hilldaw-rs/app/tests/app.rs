//! Drives HillDaw the way the runner does: button events, ticks, audio buffers and frames.

use hill_core::fb::Fb;
use hill_core::{App, Btn, Host, MemHost};
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
    press(&mut a, Btn::R1);
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
    press(&mut a, Btn::R1);
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
