//! Screenshot of the Input/Output tab with a USB output streaming: `cargo run -p hilldaw-app --example outtab -- DIR`
use hill_core::{App, Btn, MemHost};
use hilldaw_app::status::{InputSummary, Status};
use hilldaw_app::usbout::AudioOut;
use hilldaw_app::HillDaw;
use std::sync::{Arc, Mutex};

struct F(Option<String>);
impl AudioOut for F {
    fn select(&mut self, id: Option<String>) -> Result<(), String> {
        self.0 = id.map(|_| "FM-1".to_string());
        Ok(())
    }
    fn push(&mut self, _: &[f32]) -> bool {
        self.0.is_some()
    }
    fn active(&self) -> Option<String> {
        self.0.clone()
    }
    fn stats(&self) -> (u64, u64) {
        (0, 0)
    }
}

fn press(a: &mut HillDaw, b: Btn) {
    a.event(b, true);
    a.event(b, false);
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let io = |id: &str, n: &str| InputSummary { id: id.into(), name: n.into(), summary: "2ch 24-bit 44.1 kHz".into(), rate: 44_100 };
    let st = Arc::new(Mutex::new(Status { fm1: true, inputs: vec![io("4c4a:c755", "FM-1")], outputs: vec![io("4c4a:c755", "FM-1")], ..Status::default() }));
    let mut a = HillDaw::new(Box::new(MemHost::default()), st).with_audio_out(Box::new(F(None)));
    a.tick(600);
    press(&mut a, Btn::R1);
    press(&mut a, Btn::R2);
    press(&mut a, Btn::R2);
    for _ in 0..3 {
        press(&mut a, Btn::Down);
    }
    press(&mut a, Btn::Right);
    let mut fb = hill_core::fb::Fb::new();
    a.render(&mut fb);
    let mut out = b"P6\n640 480\n255\n".to_vec();
    for p in fb.px.iter() {
        out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    std::fs::write(format!("{dir}/outtab.ppm"), out).unwrap();
}
