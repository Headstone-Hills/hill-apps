//! Renders the Waves screen with some recordings: `cargo run -p hilldaw-app --example waves -- OUT_DIR`.

use hill_core::fb::Fb;
use hill_core::{App, Btn, MemHost};
use hilldaw_app::recorder::{RecInfo, Recorder};
use hilldaw_app::status::Status;
use hilldaw_app::{HillDaw, View};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// A pretend recorder: every recording is a swelling, decaying tone so the waveform has a shape.
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
        let k: f32 = rel.trim_start_matches("audio/rec-").trim_end_matches(".wav").parse().ok()?;
        Some(
            (0..n)
                .map(|i| {
                    let t = i as f32 / 44100.0;
                    let env = ((t * 0.8 * k).sin().abs() * 0.7 + 0.1) * (-t * 0.05).exp();
                    (t * 2.0 * std::f32::consts::PI * 220.0 * k).sin() * env
                })
                .collect(),
        )
    }
}

fn press(a: &mut HillDaw, b: Btn) {
    a.event(b, true);
    a.event(b, false);
}

fn save(a: &HillDaw, path: &str) {
    let mut fb = Fb::new();
    a.render(&mut fb);
    let mut out = b"P6\n640 480\n255\n".to_vec();
    for p in &fb.px {
        out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    std::fs::write(path, out).unwrap();
}

fn to_view(a: &mut HillDaw, v: View) {
    while a.view() != v {
        press(a, Btn::R1);
    }
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let st = Arc::new(Mutex::new(Status::default()));
    let mut a = HillDaw::new(Box::new(MemHost::default()), st).with_recorder(Box::new(Fake::default()));
    // record on tracks 1, 2 and 4 (3 and 5-8 stay empty); the clock is pretend
    a.event(Btn::Fn, true);
    press(&mut a, Btn::L1); // count-in off
    a.event(Btn::Fn, false);
    let mut buf = vec![0.0f32; 2048];
    let mut done = 0u64;
    for (track, secs, rewind) in [(0usize, 14.0, true), (1, 9.0, true), (3, 6.0, false)] {
        to_view(&mut a, View::Mixer);
        for _ in 0..8 {
            press(&mut a, Btn::Left);
        }
        for _ in 0..track {
            press(&mut a, Btn::Right);
        }
        press(&mut a, Btn::A);
        if rewind {
            a.event(Btn::Fn, true);
            press(&mut a, Btn::B);
            a.event(Btn::Fn, false);
        }
        a.set_test_clock(Some(done as f64 / 44100.0));
        press(&mut a, Btn::Select);
        let target = done + (secs * 44100.0) as u64;
        while done < target {
            a.audio_at(&mut buf, done as f64 / 44100.0);
            done += 1024;
        }
        press(&mut a, Btn::Select);
        press(&mut a, Btn::A); // disarm
    }
    // mute track 2, and a loop region
    to_view(&mut a, View::Mixer);
    for _ in 0..8 {
        press(&mut a, Btn::Left);
    }
    press(&mut a, Btn::Right);
    press(&mut a, Btn::X);
    a.set_loop_region(Some((2 * 105_840, 5 * 105_840)));
    a.event(Btn::Fn, true);
    press(&mut a, Btn::B);
    a.event(Btn::Fn, false);
    to_view(&mut a, View::Waves);
    for _ in 0..30 {
        a.audio_at(&mut buf, done as f64 / 44100.0);
        done += 1024;
        a.tick(23);
    }
    save(&a, &format!("{dir}/waves.ppm"));
    press(&mut a, Btn::L2); // zoom out
    press(&mut a, Btn::L2);
    save(&a, &format!("{dir}/waves_out.ppm"));
}
