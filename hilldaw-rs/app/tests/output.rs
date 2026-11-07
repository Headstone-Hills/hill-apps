//! Playback through a USB audio device: choosing it, the built-in output going quiet while it streams, falling back when it goes away.

use hill_core::fb::Fb;
use hill_core::{App, Btn, Host, MemHost};
use hilldaw_app::status::{InputSummary, Status, StatusHandle};
use hilldaw_app::usbout::AudioOut;
use hilldaw_app::{HillDaw, TrackTab, View};
use std::sync::{Arc, Mutex};

const DAC: &str = "1234:00ab";
const FM1: &str = "4c4a:c755";

#[derive(Default)]
struct State {
    selected: Vec<Option<String>>,
    /// what the device does with `select`: Err = could not open
    fail: Option<String>,
    streaming: Option<String>,
    /// the device vanished (unplugged): push is refused
    gone: bool,
    got: Vec<f32>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<State>>);

impl AudioOut for Fake {
    fn select(&mut self, id: Option<String>) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.selected.push(id.clone());
        s.gone = false;
        if let (Some(_), Some(e)) = (&id, s.fail.clone()) {
            s.streaming = None;
            return Err(e);
        }
        s.streaming = id.map(|i| format!("dev {i}"));
        Ok(())
    }
    fn push(&mut self, frames: &[f32]) -> bool {
        let mut s = self.0.lock().unwrap();
        if s.streaming.is_none() || s.gone {
            return false;
        }
        s.got.extend_from_slice(frames);
        true
    }
    fn active(&self) -> Option<String> {
        let s = self.0.lock().unwrap();
        if s.gone { None } else { s.streaming.clone() }
    }
    fn stats(&self) -> (u64, u64) {
        (0, 0)
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

fn press(a: &mut HillDaw, b: Btn) {
    a.event(b, true);
    a.event(b, false);
}

fn outputs() -> Vec<InputSummary> {
    vec![
        InputSummary { id: DAC.into(), name: "USB DAC".into(), summary: "2ch 16-bit 48 kHz".into(), rate: 48_000 },
        InputSummary { id: FM1.into(), name: "FM-1".into(), summary: "2ch 24-bit 44.1 kHz".into(), rate: 44_100 },
    ]
}

/// An app with the fake output, on the Input tab's Output row.
fn app(host: &Shared) -> (HillDaw, Fake, StatusHandle) {
    let fake = Fake::default();
    let st: StatusHandle = Arc::new(Mutex::new(Status::default()));
    let mut a = HillDaw::new(Box::new(host.clone()), st.clone()).with_audio_out(Box::new(fake.clone()));
    for _ in 0..8 {
        if a.view() == View::Track {
            break;
        }
        press(&mut a, Btn::R1);
    }
    while a.track_tab() != TrackTab::Input {
        press(&mut a, Btn::R2);
    }
    for _ in 0..3 {
        press(&mut a, Btn::Down);
    }
    assert_eq!(a.track_row(), 3);
    (a, fake, st)
}

fn render(a: &mut HillDaw, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    a.audio_at(&mut out, 0.0);
    out
}

#[test]
fn the_built_in_output_is_used_until_a_usb_output_is_chosen_and_the_choice_is_remembered() {
    let host = Shared::default();
    let (mut a, fake, st) = app(&host);
    st.lock().unwrap().outputs = outputs();
    a.tick(600);
    assert_eq!(a.selected_output(), None);
    press(&mut a, Btn::Start); // play: the starting groove sounds
    let built_in = render(&mut a, 2048);
    assert!(built_in.iter().any(|s| s.abs() > 0.001), "sound comes out of the built-in output");
    assert!(fake.0.lock().unwrap().got.is_empty(), "nothing is sent to USB");

    press(&mut a, Btn::Right); // Built-in -> first device
    assert_eq!(a.selected_output(), Some(DAC));
    assert_eq!(a.output_name().as_deref(), Some("dev 1234:00ab"));
    assert!(a.toast_text().contains("USB DAC"), "{}", a.toast_text());
    let out = render(&mut a, 2048);
    assert!(out.iter().all(|s| *s == 0.0), "the built-in output is silent while USB plays");
    let got = fake.0.lock().unwrap().got.clone();
    assert_eq!(got.len(), 2048 * 2);
    assert!(got.iter().any(|s| s.abs() > 0.001), "and the audio went to the device");

    press(&mut a, Btn::Right);
    assert_eq!(a.selected_output(), Some(FM1));
    press(&mut a, Btn::Right); // round to the built-in output
    assert_eq!(a.selected_output(), None);
    assert!(a.output_name().is_none());
    assert_eq!(fake.0.lock().unwrap().selected.last().unwrap(), &None, "the stream was stopped");

    press(&mut a, Btn::Left);
    assert_eq!(a.selected_output(), Some(FM1));
    // a restart brings the saved choice back up (its device is attached)
    let (b, fake2, _st2) = app_with_status(&host, outputs());
    assert_eq!(b.selected_output(), Some(FM1));
    assert_eq!(fake2.0.lock().unwrap().selected.last().unwrap().as_deref(), Some(FM1));
}

fn app_with_status(host: &Shared, o: Vec<InputSummary>) -> (HillDaw, Fake, StatusHandle) {
    let fake = Fake::default();
    let st: StatusHandle = Arc::new(Mutex::new(Status::default()));
    st.lock().unwrap().outputs = o;
    let mut a = HillDaw::new(Box::new(host.clone()), st.clone());
    a.tick(600); // status arrives
    let a = a.with_audio_out(Box::new(fake.clone()));
    (a, fake, st)
}

#[test]
fn a_device_that_will_not_open_leaves_the_built_in_output_playing_and_says_why() {
    let (mut a, fake, st) = app(&Shared::default());
    st.lock().unwrap().outputs = outputs();
    a.tick(600);
    fake.0.lock().unwrap().fail = Some("busy".into());
    press(&mut a, Btn::Right);
    assert!(a.toast_text().contains("busy"), "{}", a.toast_text());
    assert!(a.output_name().is_none());
    press(&mut a, Btn::Start);
    let out = render(&mut a, 2048);
    assert!(out.iter().any(|s| s.abs() > 0.001), "the built-in output still plays");
    let mut fb = Fb::new();
    a.render(&mut fb);
}

#[test]
fn unplugging_the_device_falls_back_to_the_built_in_output_and_replugging_resumes() {
    let (mut a, fake, st) = app(&Shared::default());
    st.lock().unwrap().outputs = outputs();
    a.tick(600);
    press(&mut a, Btn::Right);
    press(&mut a, Btn::Start);
    assert!(render(&mut a, 1024).iter().all(|s| *s == 0.0));
    // unplug: the stream ends and the next block plays on the built-in output
    fake.0.lock().unwrap().gone = true;
    st.lock().unwrap().outputs.clear();
    a.tick(600);
    let out = render(&mut a, 1024);
    assert!(out.iter().any(|s| s.abs() > 0.001), "back on the built-in output");
    assert_eq!(a.selected_output(), Some(DAC), "the choice is kept");
    let mut fb = Fb::new();
    a.render(&mut fb);
    // plug in again: within a couple of seconds the app starts it
    st.lock().unwrap().outputs = outputs();
    for _ in 0..4 {
        a.tick(600);
    }
    assert_eq!(a.output_name().as_deref(), Some("dev 1234:00ab"));
    assert!(render(&mut a, 1024).iter().all(|s| *s == 0.0));
}

#[test]
fn a_saved_choice_whose_device_is_not_attached_waits_quietly() {
    let host = Shared::default();
    let (mut a, _f, st) = app(&host);
    st.lock().unwrap().outputs = outputs();
    a.tick(600);
    press(&mut a, Btn::Right);
    drop(a);
    let (mut b, fake, _st) = app_with_status(&host, vec![]);
    assert_eq!(b.selected_output(), Some(DAC));
    assert!(b.output_name().is_none());
    assert_eq!(fake.0.lock().unwrap().selected.last().unwrap(), &None, "nothing was opened");
    press(&mut b, Btn::Start);
    assert!(render(&mut b, 2048).iter().any(|s| s.abs() > 0.001), "the built-in output plays meanwhile");
}

#[test]
fn without_a_usb_output_connected_the_row_says_so_and_nothing_changes() {
    let host = Shared::default();
    let st: StatusHandle = Arc::new(Mutex::new(Status::default()));
    let mut a = HillDaw::new(Box::new(host), st);
    for _ in 0..8 {
        if a.view() == View::Track {
            break;
        }
        press(&mut a, Btn::R1);
    }
    while a.track_tab() != TrackTab::Input {
        press(&mut a, Btn::R2);
    }
    for _ in 0..3 {
        press(&mut a, Btn::Down);
    }
    press(&mut a, Btn::Right);
    assert_eq!(a.selected_output(), None);
    assert!(a.toast_text().contains("No USB output"), "{}", a.toast_text());
}
