//! Recording through a USB output that echoes: the app keeps a copy of what it plays into the device while recording, and asks the
//! recorder to take that echo out of the finished file.

use hill_core::{App, Btn, MemHost};
use hilldaw_app::recorder::{RecInfo, Recorder};
use hilldaw_app::status::{InputSummary, Status, StatusHandle};
use hilldaw_app::usbout::AudioOut;
use hilldaw_app::{HillDaw, TrackTab, View};
use hilldaw_core::echo::EchoReport;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Out {
    streaming: bool,
    tap_on: bool,
    tapped: Vec<f32>,
    tap_calls: Vec<bool>,
}

#[derive(Clone, Default)]
struct FakeOut(Arc<Mutex<Out>>);

impl AudioOut for FakeOut {
    fn select(&mut self, id: Option<String>) -> Result<(), String> {
        self.0.lock().unwrap().streaming = id.is_some();
        Ok(())
    }
    fn push(&mut self, frames: &[f32]) -> bool {
        let mut s = self.0.lock().unwrap();
        if !s.streaming {
            return false;
        }
        if s.tap_on {
            let mono: Vec<f32> = frames.chunks_exact(2).map(|f| (f[0] + f[1]) * 0.5).collect();
            s.tapped.extend(mono);
        }
        true
    }
    fn active(&self) -> Option<String> {
        self.0.lock().unwrap().streaming.then(|| "dev".into())
    }
    fn stats(&self) -> (u64, u64) {
        (0, 0)
    }
    fn tap(&mut self, on: bool) {
        let mut s = self.0.lock().unwrap();
        s.tap_calls.push(on);
        s.tap_on = on;
        if on {
            s.tapped.clear();
        }
    }
    fn take_tap(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.0.lock().unwrap().tapped)
    }
}

#[derive(Default)]
struct Rec {
    started: usize,
    /// (file, reference length) of every echo removal asked for
    cleaned: Vec<(String, usize)>,
    echo: bool,
}

#[derive(Clone, Default)]
struct FakeRec(Arc<Mutex<Rec>>);

impl Recorder for FakeRec {
    fn start(&mut self) -> Result<String, String> {
        let mut s = self.0.lock().unwrap();
        s.started += 1;
        Ok(format!("audio/rec-{:03}.wav", s.started))
    }
    fn frames(&self) -> u64 {
        0
    }
    fn finish(&mut self, min_frames: u64) -> Result<RecInfo, String> {
        Ok(RecInfo { frames: min_frames + 500, overruns: 0 })
    }
    fn load(&mut self, _rel: &str) -> Option<Vec<f32>> {
        Some(vec![0.1; 200_000])
    }
    fn remove_echo(&mut self, rel: &str, reference: &[f32]) -> Option<EchoReport> {
        let mut s = self.0.lock().unwrap();
        s.cleaned.push((rel.to_string(), reference.len()));
        s.echo.then_some(EchoReport { gain: 0.75, lag: 4000.0, corr: 0.9, removed_db: -14.0 })
    }
}

fn press(a: &mut HillDaw, b: Btn) {
    a.event(b, true);
    a.event(b, false);
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

/// An app recording to a fake recorder and playing through a fake USB output (chosen on the Input tab), with track 1 armed.
fn ready(usb: bool, echo: bool) -> (HillDaw, FakeOut, FakeRec) {
    let (out, rec) = (FakeOut::default(), FakeRec::default());
    rec.0.lock().unwrap().echo = echo;
    let st: StatusHandle = Arc::new(Mutex::new(Status::default()));
    st.lock().unwrap().outputs = vec![InputSummary { id: "4c4a:c755".into(), name: "FM-1".into(), summary: "2ch 24-bit 44.1 kHz".into(), rate: 44_100 }];
    let mut a = HillDaw::new(Box::new(MemHost::default()), st).with_recorder(Box::new(rec.clone())).with_audio_out(Box::new(out.clone()));
    a.tick(600);
    if usb {
        to_view(&mut a, View::Track);
        while a.track_tab() != TrackTab::Input {
            press(&mut a, Btn::R2);
        }
        for _ in 0..3 {
            press(&mut a, Btn::Down);
        }
        press(&mut a, Btn::Right);
        assert_eq!(a.selected_output(), Some("4c4a:c755"));
    }
    to_view(&mut a, View::Mixer);
    if a.project.count_in {
        a.event(Btn::Fn, true);
        press(&mut a, Btn::L1); // no count-in
        a.event(Btn::Fn, false);
    }
    press(&mut a, Btn::A); // arm track 1
    a.set_test_clock(Some(0.0));
    (a, out, rec)
}

fn record_one_second(a: &mut HillDaw) {
    press(a, Btn::Select);
    assert!(a.is_recording());
    let mut buf = vec![0.0f32; 2048 * 2];
    for i in 0..22 {
        a.audio_at(&mut buf, i as f64 * 2048.0 / 44_100.0);
    }
    press(a, Btn::Select);
    assert!(!a.is_recording());
}

#[test]
fn the_copy_of_what_was_played_is_kept_only_while_recording_and_handed_to_the_recorder() {
    let (mut a, out, rec) = ready(true, true);
    record_one_second(&mut a);
    let calls = out.0.lock().unwrap().tap_calls.clone();
    assert_eq!(calls, vec![true, false], "started with the recording, stopped when it ended");
    {
        let r = rec.0.lock().unwrap();
        assert_eq!(r.cleaned.len(), 1, "one echo removal for the one recording");
        assert_eq!(r.cleaned[0].0, "audio/rec-001.wav");
        assert!(r.cleaned[0].1 >= 20 * 2048, "the reference covers what was played ({})", r.cleaned[0].1);
    }
    assert!(a.toast_text().contains("echo -14 dB"), "{}", a.toast_text());
}

#[test]
fn no_echo_found_is_a_plain_recording() {
    let (mut a, _out, rec) = ready(true, false);
    record_one_second(&mut a);
    assert_eq!(rec.0.lock().unwrap().cleaned.len(), 1);
    assert!(a.toast_text().starts_with("Recorded 1 take") && !a.toast_text().contains("echo"), "{}", a.toast_text());
}

#[test]
fn with_the_built_in_output_nothing_is_tapped_or_cleaned() {
    let (mut a, out, rec) = ready(false, true);
    record_one_second(&mut a);
    assert!(out.0.lock().unwrap().tap_calls.iter().all(|on| !on), "never switched on");
    assert!(rec.0.lock().unwrap().cleaned.is_empty());
}
