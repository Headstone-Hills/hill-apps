//! Renders each screen to a PPM file: `cargo run -p hilldaw-app --example snapshot -- OUT_DIR`.
//! (Convert with any tool, e.g. `sips -s format png x.ppm --out x.png`.)

use hill_core::fb::Fb;
use hill_core::{App, Btn, MemHost};
use hilldaw_app::status::Status;
use hilldaw_app::HillDaw;
use std::sync::{Arc, Mutex};

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

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let st = Arc::new(Mutex::new(Status { battery_pct: Some(45), charging: true, battery_low: false, free_bytes: Some(100 << 30), fm1: true }));
    let mut a = HillDaw::new(Box::new(MemHost::default()), st);
    // mixer: some levels, a mute, a solo, an arm, then play so meters move
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::A);
    press(&mut a, Btn::Right);
    press(&mut a, Btn::X);
    press(&mut a, Btn::Right);
    for _ in 0..6 {
        press(&mut a, Btn::Right);
    }
    press(&mut a, Btn::L2);
    press(&mut a, Btn::L2);
    press(&mut a, Btn::Start);
    let mut buf = vec![0.0f32; 2048];
    for _ in 0..40 {
        a.audio(&mut buf);
        a.tick(23);
    }
    save(&a, &format!("{dir}/mixer.ppm"));
    press(&mut a, Btn::R1);
    press(&mut a, Btn::A);
    press(&mut a, Btn::Right);
    press(&mut a, Btn::Right);
    save(&a, &format!("{dir}/arrange.ppm"));
    a.event(Btn::Fn, true);
    a.event(Btn::Select, true);
    a.event(Btn::Select, false);
    a.event(Btn::Fn, false);
    save(&a, &format!("{dir}/help.ppm"));
    // FM-1 screens: close help, go to the third screen, choose a voice, then show the filter and the distortion
    press(&mut a, Btn::A);
    press(&mut a, Btn::R1);
    for _ in 0..4 {
        press(&mut a, Btn::Right);
    }
    save(&a, &format!("{dir}/fm1_voice.ppm"));
    press(&mut a, Btn::R2);
    press(&mut a, Btn::A);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Left);
    press(&mut a, Btn::Right);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Right);
    save(&a, &format!("{dir}/fm1_filter.ppm"));
    for _ in 0..2 {
        press(&mut a, Btn::R2);
    }
    save(&a, &format!("{dir}/fm1_delay.ppm"));
}
