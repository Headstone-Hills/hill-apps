//! Recording through the app with a fake recorder that serves a known ramp, so the exact sample alignment can be checked.

use hill_core::{App, Btn, Host, MemHost};
use hilldaw_app::recorder::{RecInfo, Recorder};
use hilldaw_app::status::{Status, StatusHandle};
use hilldaw_app::{HillDaw, STATE_KEY};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const R: f64 = 44100.0;
/// ramp: file frame i has the value i * 1e-5, so any output sample says which file frame it came from
const SLOPE: f32 = 1e-5;
const PAN_CENTRE: f32 = std::f32::consts::FRAC_1_SQRT_2;

#[derive(Default)]
struct State {
    started: usize,
    finished_with: Vec<u64>,
    fail_start: Option<String>,
    overruns: u64,
    /// the capture stalls: never more than this many frames reach the disk
    cap_frames: Option<u64>,
    /// frames written, by file name
    files: HashMap<String, u64>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<State>>);

impl Recorder for Fake {
    fn start(&mut self) -> Result<String, String> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = &s.fail_start {
            return Err(e.clone());
        }
        s.started += 1;
        Ok(format!("audio/rec-{:03}.wav", s.started))
    }
    fn frames(&self) -> u64 {
        0
    }
    fn finish(&mut self, min_frames: u64) -> Result<RecInfo, String> {
        let mut s = self.0.lock().unwrap();
        s.finished_with.push(min_frames);
        let frames = s.cap_frames.unwrap_or(min_frames + 500);
        let name = format!("audio/rec-{:03}.wav", s.started);
        s.files.insert(name, frames);
        Ok(RecInfo { frames, overruns: s.overruns })
    }
    fn load(&mut self, rel: &str) -> Option<Vec<f32>> {
        let n = *self.0.lock().unwrap().files.get(rel)?;
        Some((0..n).map(|i| i as f32 * SLOPE).collect())
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

/// An app with the starter drums removed (so only recorded audio can sound), count-in as asked, track 1 armed.
fn ready(count_in: bool) -> (HillDaw, Fake) {
    let fake = Fake::default();
    let mut a = HillDaw::new(Box::new(MemHost::default()), status()).with_recorder(Box::new(fake.clone()));
    press(&mut a, Btn::R1);
    press(&mut a, Btn::X); // delete the starter drum section
    press(&mut a, Btn::L1);
    assert!(a.project.drums.sections.is_empty());
    if a.project.count_in != count_in {
        with(&mut a, Btn::Fn, Btn::L1);
    }
    assert_eq!(a.project.count_in, count_in);
    press(&mut a, Btn::A); // arm track 1
    a.set_test_clock(Some(0.0));
    (a, fake)
}

/// Run the audio callback for `frames` starting at `*done`, with the time the callback would really have.
fn run(a: &mut HillDaw, done: &mut u64, frames: usize, chunk: usize) -> Vec<f32> {
    let mut all = Vec::new();
    let mut left = frames;
    let mut buf = vec![0.0f32; chunk * 2];
    while left > 0 {
        let n = left.min(chunk);
        a.audio_at(&mut buf[..n * 2], *done as f64 / R);
        all.extend_from_slice(&buf[..n * 2]);
        *done += n as u64;
        left -= n;
    }
    all
}

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0, |m, s| m.max(s.abs()))
}

#[test]
fn nothing_starts_without_a_recorder_an_armed_track_or_a_device() {
    let mut a = HillDaw::new(Box::new(MemHost::default()), status());
    press(&mut a, Btn::Select);
    assert!(a.toast_text().contains("FM-1 recorder") && !a.is_recording());

    let fake = Fake::default();
    let mut a = HillDaw::new(Box::new(MemHost::default()), status()).with_recorder(Box::new(fake.clone()));
    press(&mut a, Btn::Select);
    assert!(a.toast_text().contains("Arm a track"), "{}", a.toast_text());
    press(&mut a, Btn::A);
    press(&mut a, Btn::Right);
    press(&mut a, Btn::A);
    press(&mut a, Btn::Select);
    assert!(a.toast_text().contains("only one"), "{}", a.toast_text());
    assert_eq!(fake.0.lock().unwrap().started, 0, "the capture was never started");

    let (mut a, fake) = ready(false);
    fake.0.lock().unwrap().fail_start = Some("FM-1 not found".into());
    press(&mut a, Btn::Select);
    assert!(a.toast_text().contains("FM-1 not found") && !a.is_recording());
}

#[test]
fn a_take_is_a_clip_placed_so_the_file_lines_up_with_the_timeline_including_the_latency() {
    let (mut a, fake) = ready(false);
    let mut done = 0;
    press(&mut a, Btn::Select);
    assert!(a.is_recording() && !a.is_counting_in() && a.is_playing());
    run(&mut a, &mut done, 2 * 44100, 1024);
    press(&mut a, Btn::Select);
    assert!(!a.is_recording());

    let t = &a.project.tracks[0];
    assert_eq!((t.takes.len(), t.active_take, t.clips.len()), (1, Some(0), 1));
    let c = &t.clips[0];
    // capture index 0 = the first recorded frame (no count-in, clock 0); audio arrives 563 frames late, so the clip starts 563 frames into the file
    assert_eq!((c.start, c.offset, c.len, c.file.as_str()), (0, 563, Some(88_200), "audio/rec-001.wav"));
    assert_eq!(t.takes[0].latency_comp_frames, 563);
    assert_eq!(fake.0.lock().unwrap().finished_with, vec![563 + 88_200], "the capture was kept running until the delayed audio arrived");

    // play it back from the start: output frame p is file frame 563 + p, through the centre pan law
    // (stopping a recording with SELECT leaves the transport playing, as in most DAWs; rewind and keep going)
    assert!(a.is_playing());
    with(&mut a, Btn::Fn, Btn::B);
    let mut done2 = 1_000_000;
    let out = run(&mut a, &mut done2, 3 * 44100, 1024);
    for p in [0usize, 1, 1000, 50_000, 88_199] {
        let want = (563 + p) as f32 * SLOPE * PAN_CENTRE;
        assert!((out[2 * p] - want).abs() < 2e-5 && (out[2 * p + 1] - want).abs() < 2e-5, "frame {p}: {} vs {want}", out[2 * p]);
    }
    assert_eq!(peak(&out[2 * 88_200..]), 0.0, "silent after the take ends (no loop, no drums)");
}

#[test]
fn the_count_in_clicks_without_moving_the_transport_and_recording_starts_exactly_where_it_ends() {
    let (mut a, _fake) = ready(true);
    let mut done = 0;
    press(&mut a, Btn::Select);
    assert!(a.is_counting_in());
    let ci = (4.0 * 44100.0 * 60.0 / 100.0) as usize; // 4 beats at 100 bpm = 105840 frames
    let during = run(&mut a, &mut done, ci - 3000, 1024);
    assert!(a.is_counting_in() && a.bar_beat() == (1, 1), "transport has not moved");
    assert!(peak(&during) > 0.1, "the click is audible");
    // finish the count-in in the middle of a chunk, then record 1 s
    run(&mut a, &mut done, 3000 + 44100, 1024);
    assert!(!a.is_counting_in() && a.is_recording());
    press(&mut a, Btn::Select);
    let c = &a.project.tracks[0].clips[0];
    // the capture ran during the count-in, so the first recorded frame is capture index 105840 (+563 latency)
    assert_eq!(c.offset, ci as u64 + 563, "count-in length is skipped in the file");
    assert_eq!(c.start, 0, "the take sits at the transport position where recording began");
    assert!(c.len.unwrap() >= 44100 - 1024 && c.len.unwrap() <= 44100 + 1024, "{:?}", c.len);
}

#[test]
fn stopping_during_the_count_in_records_nothing() {
    let (mut a, fake) = ready(true);
    let mut done = 0;
    press(&mut a, Btn::Select);
    run(&mut a, &mut done, 20_000, 1024);
    press(&mut a, Btn::Select);
    assert!(!a.is_recording() && a.toast_text().contains("Nothing recorded"), "{}", a.toast_text());
    assert!(a.project.tracks[0].clips.is_empty() && a.project.tracks[0].takes.is_empty());
    assert_eq!(fake.0.lock().unwrap().finished_with.len(), 1, "the capture was still closed");
}

#[test]
fn each_loop_pass_is_its_own_take_and_the_latest_plays() {
    let (mut a, _fake) = ready(false);
    // a 2-bar loop so three passes fit: add one drum section back (cycles 2) so there is something to loop; mute its strip
    press(&mut a, Btn::R1);
    press(&mut a, Btn::A);
    a.event(Btn::Y, true);
    for _ in 0..2 {
        press(&mut a, Btn::Left); // cycles 4 -> 2
    }
    a.event(Btn::Y, false);
    press(&mut a, Btn::L1);
    for _ in 0..8 {
        press(&mut a, Btn::Right);
    }
    press(&mut a, Btn::X); // mute the drum strip
    press(&mut a, Btn::Left);
    for _ in 0..7 {
        press(&mut a, Btn::Left);
    }
    let len = a.drum_frames() as usize;
    assert!(a.transport_loop().is_some() && len > 0);
    let mut done = 0;
    press(&mut a, Btn::Select);
    run(&mut a, &mut done, len * 2 + len / 2, 1024);
    press(&mut a, Btn::Select);
    let t = &a.project.tracks[0];
    assert_eq!(t.takes.len(), 3, "two full passes and a partial one");
    assert_eq!((t.active_take, t.clips.len()), (Some(2), 1), "only the latest take is on the track");
    assert_eq!(t.clips[0].start, 0);
    assert_eq!(t.clips[0].offset, 563 + 2 * len as u64, "third pass starts two loop lengths into the capture");
}

#[test]
fn a_take_is_never_longer_than_what_reached_the_disk() {
    let (mut a, fake) = ready(false);
    fake.0.lock().unwrap().cap_frames = Some(30_000); // the stream stalled after 30000 frames
    let mut done = 0;
    press(&mut a, Btn::Select);
    run(&mut a, &mut done, 2 * 44100, 1024);
    press(&mut a, Btn::Select);
    let c = &a.project.tracks[0].clips[0];
    assert_eq!(c.len, Some(30_000 - 563), "clamped to the file");
}

#[test]
fn dropped_chunks_are_reported_not_hidden() {
    let (mut a, fake) = ready(false);
    fake.0.lock().unwrap().overruns = 3;
    let mut done = 0;
    press(&mut a, Btn::Select);
    run(&mut a, &mut done, 44100, 1024);
    press(&mut a, Btn::Select);
    assert!(a.toast_text().contains("3 chunks dropped"), "{}", a.toast_text());
}

#[test]
fn start_stops_the_recording_and_quitting_mid_take_keeps_it() {
    let (mut a, _f) = ready(false);
    let mut done = 0;
    press(&mut a, Btn::Select);
    run(&mut a, &mut done, 44100, 1024);
    press(&mut a, Btn::Start);
    assert!(!a.is_recording() && !a.is_playing());
    assert_eq!(a.project.tracks[0].clips.len(), 1, "START ends the recording and keeps the take");

    let (mut a, _f) = ready(false);
    let mut done = 0;
    press(&mut a, Btn::Select);
    run(&mut a, &mut done, 44100, 1024);
    a.event(Btn::Start, true);
    a.event(Btn::Select, true);
    a.tick(1600);
    assert!(a.wants_quit() && !a.is_recording(), "the quit chord closes the take first");
    assert_eq!(a.project.tracks[0].clips.len(), 1);
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

#[test]
fn a_recorded_take_survives_quitting_and_plays_after_a_restart() {
    let host = Shared::default();
    let fake = Fake::default();
    let mut a = HillDaw::new(Box::new(host.clone()), status()).with_recorder(Box::new(fake.clone()));
    press(&mut a, Btn::R1);
    press(&mut a, Btn::X);
    press(&mut a, Btn::L1);
    with(&mut a, Btn::Fn, Btn::L1); // count-in off
    press(&mut a, Btn::A);
    a.set_test_clock(Some(0.0));
    let mut done = 0;
    press(&mut a, Btn::Select);
    run(&mut a, &mut done, 44100, 1024);
    press(&mut a, Btn::Select);
    assert!(host.load_state(STATE_KEY).unwrap().contains("rec-001.wav"), "saved the moment the take was made");

    // a new session with the same project and the same recordings
    let mut b = HillDaw::new(Box::new(host.clone()), status()).with_recorder(Box::new(fake.clone()));
    assert_eq!(b.project.tracks[0].clips.len(), 1);
    press(&mut b, Btn::Start);
    let mut d2 = 0;
    let out = run(&mut b, &mut d2, 20_000, 1024);
    assert!(peak(&out) > 0.001, "the recording plays after a restart");
    let want = (563 + 500) as f32 * SLOPE * PAN_CENTRE;
    assert!((out[2 * 500] - want).abs() < 2e-5, "{} vs {want}", out[2 * 500]);
    // a recording that is no longer on disk is reported, not a crash
    let gone = Fake::default();
    let mut c = HillDaw::new(Box::new(host), status()).with_recorder(Box::new(gone));
    assert!(c.toast_text().contains("missing"), "{}", c.toast_text());
    press(&mut c, Btn::Start);
    let mut d3 = 0;
    assert_eq!(peak(&run(&mut c, &mut d3, 5000, 1024)), 0.0);
}

#[test]
fn the_rec_lamp_shows_count_in_amber_then_recording_red_then_idle() {
    use hill_core::fb::{Fb, DIM, WARN};
    let count = |fb: &Fb, c: u32| (84..100).flat_map(|y| (246..420).map(move |x| (x, y))).filter(|(x, y)| fb.px[(y * 640 + x) as usize] == c).count();
    let (mut a, _f) = ready(true);
    let mut fb = Fb::new();
    a.render(&mut fb);
    assert!(count(&fb, DIM) > 30, "idle with a recorder: a dim REC");
    let mut done = 0;
    press(&mut a, Btn::Select);
    a.render(&mut fb);
    let amber = hill_core::fb::rgb(255, 200, 60);
    assert!(count(&fb, amber) > 30 && count(&fb, WARN) == 0, "count-in is amber");
    run(&mut a, &mut done, 105_840 + 44100 * 2, 1024);
    a.render(&mut fb);
    assert!(count(&fb, WARN) > 30, "recording is red");
    press(&mut a, Btn::Select);
    a.render(&mut fb);
    assert!(count(&fb, WARN) == 0 && count(&fb, DIM) > 30, "back to idle");
}
