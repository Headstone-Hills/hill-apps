//! Screenshot of the song browser: `cargo run -p hilldaw-app --example songs -- DIR`
use hill_core::{App, Btn, MemHost};
use hilldaw_app::status::Status;
use hilldaw_app::HillDaw;
use std::sync::{Arc, Mutex};

fn press(a: &mut HillDaw, b: Btn) {
    a.event(b, true);
    a.event(b, false);
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let mut a = HillDaw::new(Box::new(MemHost::default()), Arc::new(Mutex::new(Status::default())));
    a.event(Btn::Fn, true);
    press(&mut a, Btn::R1);
    a.event(Btn::Fn, false);
    for _ in 0..2 {
        press(&mut a, Btn::Down);
        press(&mut a, Btn::A);
        a.event(Btn::Fn, true);
        press(&mut a, Btn::R1);
        a.event(Btn::Fn, false);
    }
    let mut fb = hill_core::fb::Fb::new();
    a.render(&mut fb);
    let mut out = b"P6\n640 480\n255\n".to_vec();
    for p in &fb.px {
        out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    std::fs::write(format!("{dir}/songs.ppm"), out).unwrap();
}
