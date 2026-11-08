//! The Waves screen: waveforms over time for every track, the drum blocks, playhead, loop/punch bands, zoom, scroll and follow.

use hill_core::fb::{rgb, scale, Fb, ACCENT, BG, DRUM, MELODIC, WARN, WHITE};
use hill_core::{App, Btn, MemHost};
use hilldaw_app::recorder::{RecInfo, Recorder};
use hilldaw_app::status::{Status, StatusHandle};
use hilldaw_app::{HillDaw, View};
use hilldaw_core::waveform::frame_to_x;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const R: f64 = 44100.0;
const BAR: u64 = 105_840; // 100 bpm, 4/4
const X0: i32 = 62;
const W: i32 = 560;
const ROW_Y: i32 = 158;
const ROW_H: i32 = 26;

/// Every recording is a ramp rising from 0 (slope 2e-6 per frame), so a taller column means later in the file.
#[derive(Clone, Default)]
struct Fake(Arc<Mutex<(BTreeMap<String, u64>, usize)>>);

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
        Some((0..n).map(|i| (i as f32 * 2e-6) * if i % 2 == 0 { 1.0 } else { -1.0 }).collect())
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

fn new_app() -> HillDaw {
    HillDaw::new(Box::new(MemHost::default()), status()).with_recorder(Box::new(Fake::default()))
}

/// Run the audio callback for `frames` (the transport must be playing) starting at pretend time `*t`.
fn run(a: &mut HillDaw, t: &mut u64, frames: usize) {
    let mut buf = vec![0.0f32; 2048];
    let mut left = frames;
    while left > 0 {
        let n = left.min(1024);
        a.audio_at(&mut buf[..n * 2], *t as f64 / R);
        *t += n as u64;
        left -= n;
    }
}

/// Record `secs` seconds on the given track (0-based), from bar 1, with count-in off; leaves the track disarmed.
fn record(a: &mut HillDaw, track: usize, secs: f64, t: &mut u64) {
    to_view(a, View::Mixer);
    for _ in 0..8 {
        press(a, Btn::Left);
    }
    for _ in 0..track {
        press(a, Btn::Right);
    }
    if a.project.count_in {
        with(a, Btn::Fn, Btn::L1);
    }
    press(a, Btn::A); // arm
    with(a, Btn::Fn, Btn::B);
    a.set_test_clock(Some(*t as f64 / R));
    press(a, Btn::Select);
    run(a, t, (secs * R) as usize);
    press(a, Btn::Select);
    press(a, Btn::A); // disarm
}

fn render(a: &HillDaw) -> Fb {
    let mut fb = Fb::new();
    a.render(&mut fb);
    fb
}

fn px(fb: &Fb, x: i32, y: i32) -> u32 {
    fb.px[(y * 640 + x) as usize]
}

/// How many pixels of colour `c` there are in a rectangle.
fn count(fb: &Fb, c: u32, x0: i32, y0: i32, x1: i32, y1: i32) -> usize {
    (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|(x, y)| px(fb, *x, *y) == c).count()
}

fn row(i: i32) -> (i32, i32) {
    (ROW_Y + i * ROW_H, ROW_Y + (i + 1) * ROW_H)
}

#[test]
fn tracks_with_audio_show_a_waveform_and_empty_tracks_do_not() {
    let mut a = new_app();
    let mut t = 0;
    record(&mut a, 0, 6.0, &mut t);
    record(&mut a, 2, 4.0, &mut t);
    to_view(&mut a, View::Waves);
    let fb = render(&a);
    let wave = |i: i32| {
        let (y0, y1) = row(i);
        count(&fb, MELODIC, X0, y0, X0 + W, y1)
    };
    assert!(wave(0) > 200 && wave(2) > 200, "tracks 1 and 3 are drawn ({} and {} px)", wave(0), wave(2));
    for i in [1, 3, 4, 5, 6, 7] {
        assert_eq!(wave(i), 0, "track {} is empty and shows no waveform", i + 1);
    }
    // the label of a used track is bright, an empty one dim
    let label = |i: i32| count(&fb, MELODIC, 14, row(i).0, 34, row(i).1);
    assert!(label(0) > 0 && label(2) > 0 && label(1) == 0 && label(3) == 0, "labels: used tracks are bright");
    // an empty track says so
    let (y0, y1) = row(1);
    assert!((X0..X0 + 60).flat_map(|x| (y0..y1).map(move |y| (x, y))).any(|(x, y)| px(&fb, x, y) == rgb(48, 56, 72)), "\"empty\" is written on an empty row");
}

#[test]
fn the_waveform_follows_the_audio_a_rising_ramp_gets_taller_to_the_right() {
    let mut a = new_app();
    let mut t = 0;
    record(&mut a, 0, 10.0, &mut t);
    to_view(&mut a, View::Waves);
    let fb = render(&a);
    let (y0, y1) = row(0);
    let height = |x: i32| count(&fb, MELODIC, x, y0, x + 1, y1);
    let (early, late) = (height(X0 + 20), height(X0 + 200));
    assert!(late > early + 4, "later in the take the signal is bigger: {early} px at 20, {late} px at 200");
    // it stops where the take stops: 10 s is about 4.2 bars of 8 shown, so the right part of the row is bare
    assert_eq!(height(X0 + W - 20), 0, "nothing after the end of the take");
}

#[test]
fn a_boosted_level_makes_the_waveform_taller_and_a_muted_track_is_grey() {
    let mut a = new_app();
    let mut t = 0;
    record(&mut a, 0, 8.0, &mut t);
    to_view(&mut a, View::Waves);
    let before = {
        let fb = render(&a);
        let (y0, y1) = row(0);
        count(&fb, MELODIC, X0, y0, X0 + W, y1)
    };
    to_view(&mut a, View::Mixer);
    a.event(Btn::Y, true);
    press(&mut a, Btn::Up);
    a.event(Btn::Y, false); // +6 dB
    to_view(&mut a, View::Waves);
    let boosted = {
        let fb = render(&a);
        let (y0, y1) = row(0);
        count(&fb, MELODIC, X0, y0, X0 + W, y1)
    };
    assert!(boosted > before, "the picture shows the level as it is heard: {before} -> {boosted}");
    to_view(&mut a, View::Mixer);
    press(&mut a, Btn::X); // mute
    to_view(&mut a, View::Waves);
    let fb = render(&a);
    let (y0, y1) = row(0);
    assert_eq!(count(&fb, MELODIC, X0, y0, X0 + W, y1), 0, "a muted track is not drawn in its colour");
    assert!(count(&fb, rgb(96, 102, 116), X0, y0, X0 + W, y1) > 100, "it is grey");
    // its "M" flag is shown
    assert!(count(&fb, rgb(255, 200, 60), 30, y0, 50, y1) > 0, "an M is drawn next to the label");
}

#[test]
fn the_drum_arrangement_is_blocks_that_repeat_while_the_drums_loop_and_stop_when_they_do_not() {
    let mut a = new_app();
    to_view(&mut a, View::Waves);
    let (y0, y1) = row(8);
    let first = scale(DRUM, 0.55);
    let again = scale(DRUM, 0.3);
    let fb = render(&a);
    assert!(count(&fb, first, X0, y0, X0 + W, y1) > 500, "the arrangement is drawn");
    // the starter groove is 4 bars: at 8 bars across, the second half is the repeat, in the lighter shade
    assert!(count(&fb, again, X0 + W / 2 + 20, y0, X0 + W, y1) > 500, "and repeats while the drums loop");
    with(&mut a, Btn::Fn, Btn::Y); // drums once
    let fb = render(&a);
    assert_eq!(count(&fb, again, X0, y0, X0 + W, y1), 0, "no repeat when the drums play once");
    assert!(count(&fb, first, X0, y0, X0 + W / 2, y1) > 200);
    // no sections: nothing
    to_view(&mut a, View::Arrange);
    press(&mut a, Btn::X);
    to_view(&mut a, View::Waves);
    let fb = render(&a);
    assert_eq!(count(&fb, first, X0, y0, X0 + W, y1), 0, "no sections, no blocks");
}

#[test]
fn the_playhead_loop_and_punch_are_drawn_where_they_are() {
    let mut a = new_app();
    to_view(&mut a, View::Waves);
    let span = 8 * BAR;
    // the playhead at bar 3 (two bars in)
    a.event(Btn::Fn, true);
    press(&mut a, Btn::B);
    a.event(Btn::Fn, false);
    let mut t = 0;
    press(&mut a, Btn::Start);
    run(&mut a, &mut t, (2 * BAR) as usize);
    press(&mut a, Btn::Start);
    let pos = a.bar_beat();
    assert_eq!(pos.0, 3);
    let fb = render(&a);
    let (x_expect, _) = (frame_to_x(2 * BAR, 0, span, X0, W).unwrap(), ());
    assert!((x_expect - 62 - 140).abs() <= 1, "two bars of eight across 560 px = 140 px");
    let found = (X0..X0 + W).filter(|x| px(&fb, *x, 145) == WHITE).count();
    assert_eq!(found, 2, "the playhead is a 2 px line");
    assert_eq!(px(&fb, x_expect + 1, 145), WHITE);
    assert_eq!(px(&fb, x_expect + 1, 300), WHITE, "and runs down through the rows");
    // a loop region (bars 3-5) and a punch region (bar 6) as bands on the ruler
    a.set_loop_region(Some((2 * BAR, 5 * BAR)));
    a.project.punch = Some((5 * BAR, 6 * BAR));
    let fb = render(&a);
    let (xl0, xl1) = (frame_to_x(2 * BAR, 0, span, X0, W).unwrap(), frame_to_x(5 * BAR, 0, span, X0, W).unwrap());
    assert_eq!(px(&fb, xl0 + 10, 141), scale(ACCENT, 0.7), "the loop is a green band");
    assert_eq!(px(&fb, xl1 - 10, 141), scale(ACCENT, 0.7));
    assert_ne!(px(&fb, xl1 + 20, 141), scale(ACCENT, 0.7), "which ends where the loop ends");
    let xp = frame_to_x(5 * BAR, 0, span, X0, W).unwrap();
    assert_eq!(px(&fb, xp + 20, 152), scale(WARN, 0.8), "the punch region is a red band");
}

#[test]
fn zoom_scroll_seek_follow_and_row_selection() {
    let mut a = new_app();
    to_view(&mut a, View::Waves);
    assert_eq!((a.waves_zoom(), a.waves_start(), a.waves_follow()), (2, 0, true), "8 bars, from the start, following");
    press(&mut a, Btn::R2); // zoom in
    assert_eq!(a.waves_zoom(), 1);
    for _ in 0..5 {
        press(&mut a, Btn::R2);
    }
    assert_eq!(a.waves_zoom(), 0, "stops at the closest zoom");
    for _ in 0..9 {
        press(&mut a, Btn::L2);
    }
    assert_eq!(a.waves_zoom(), 5, "and at the widest");

    // scrolling: a bar at a time, a page with Y, never before the start and never off into nothing past the end of the song
    press(&mut a, Btn::Right);
    assert_eq!(a.waves_start(), BAR);
    assert!(!a.waves_follow(), "scrolling by hand stops following");
    assert!(a.toast_text().contains("Follow off"), "{}", a.toast_text());
    for _ in 0..30 {
        press(&mut a, Btn::Right);
    }
    let end = (a.song_end_for_test() / BAR) * BAR;
    assert_eq!(a.waves_start(), end.max(BAR), "the last bar that has anything in it");
    a.event(Btn::Y, true);
    for _ in 0..4 {
        press(&mut a, Btn::Left); // a whole page back (64 bars) each time: lands on 0
    }
    a.event(Btn::Y, false);
    assert_eq!(a.waves_start(), 0);
    press(&mut a, Btn::Left);
    assert_eq!(a.waves_start(), 0, "not before the start");
    assert_eq!(a.waves_start() % BAR, 0, "always a whole bar");

    // follow on again, seek, selection and leaving
    press(&mut a, Btn::X);
    assert!(a.waves_follow());
    press(&mut a, Btn::Right);
    press(&mut a, Btn::Right);
    press(&mut a, Btn::A);
    assert_eq!(a.bar_beat(), (3, 1), "A puts the playhead on the bar at the left edge");
    assert_eq!(a.sel(), 0);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Down);
    assert_eq!(a.sel(), 2);
    for _ in 0..20 {
        press(&mut a, Btn::Down);
    }
    assert_eq!(a.sel(), 8, "the drums row is the last");
    press(&mut a, Btn::B);
    assert_eq!(a.view(), View::Mixer);
}

#[test]
fn the_view_turns_the_page_when_the_playhead_runs_off_the_end_while_following() {
    let mut a = new_app();
    to_view(&mut a, View::Waves);
    for _ in 0..2 {
        press(&mut a, Btn::R2); // 2 bars across
    }
    assert_eq!(a.waves_zoom(), 0);
    a.event(Btn::Fn, true);
    press(&mut a, Btn::B);
    a.event(Btn::Fn, false);
    press(&mut a, Btn::Start);
    let mut t = 0;
    run(&mut a, &mut t, (1.5 * BAR as f64) as usize);
    a.tick(20);
    assert_eq!(a.waves_start(), 0, "still on the first page");
    run(&mut a, &mut t, BAR as usize);
    a.tick(20);
    assert_eq!(a.waves_start(), 2 * BAR, "the page turned to bars 3-4");
    run(&mut a, &mut t, (4.5 * BAR as f64) as usize);
    a.tick(20);
    assert_eq!(a.waves_start(), 6 * BAR);
    // following off: the view stays where it is
    press(&mut a, Btn::X);
    run(&mut a, &mut t, (3.0 * BAR as f64) as usize);
    a.tick(20);
    assert_eq!(a.waves_start(), 6 * BAR);
    assert!(!a.waves_follow());
}

#[test]
fn the_selected_row_is_framed_and_every_zoom_draws_without_trouble() {
    let mut a = new_app();
    let mut t = 0;
    record(&mut a, 1, 5.0, &mut t);
    to_view(&mut a, View::Waves);
    let accent_in_row = |a: &HillDaw, i: i32| {
        let fb = render(a);
        let (y0, _) = row(i);
        (12..620).filter(|x| px(&fb, *x, y0) == ACCENT).count()
    };
    let picked = a.sel() as i32;
    assert!(accent_in_row(&a, picked) > 400, "the picked row has an outline");
    assert_eq!(accent_in_row(&a, (picked + 3) % 9), 0, "and the others do not");
    press(&mut a, Btn::Down);
    assert!(accent_in_row(&a, picked + 1) > 400, "the outline moves with Down");
    for _ in 0..6 {
        press(&mut a, Btn::L2);
        let fb = render(&a);
        assert!(fb.px.iter().filter(|p| **p != BG).count() > 12_000);
    }
}

#[test]
fn the_picture_shows_clip_gain_fades_and_reversal() {
    let mut a = new_app();
    let mut t = 0;
    record(&mut a, 0, 10.0, &mut t);
    to_view(&mut a, View::Waves);
    let (y0, y1) = row(0);
    let height = |a: &HillDaw, x: i32| count(&render(a), MELODIC, x, y0, x + 1, y1);
    let plain = height(&a, X0 + 200);
    a.project.tracks[0].clips[0].gain = 0.25;
    assert!(height(&a, X0 + 200) < plain, "a quieter clip is drawn smaller");
    a.project.tracks[0].clips[0].gain = 1.0;
    // a 2-bar fade in: early pixels shrink, late ones do not
    let early = height(&a, X0 + 60);
    a.project.tracks[0].clips[0].fade_in = 2 * BAR;
    assert!(height(&a, X0 + 60) < early, "inside the fade the picture is lower");
    assert_eq!(height(&a, X0 + 200), plain, "after the fade it is unchanged");
    a.project.tracks[0].clips[0].fade_in = 0;
    // reversed, the rising ramp falls
    a.project.tracks[0].clips[0].reverse = true;
    let (e, l) = (height(&a, X0 + 20), height(&a, X0 + 200));
    assert!(e > l + 4, "a reversed ramp is taller at the start: {e} vs {l}");
}
