use hill_core::{apps, fb::Fb, Btn, MemHost};
use std::time::Instant;

fn main() {
    for name in apps::NAMES {
        let mut a = apps::create(name, Box::new(MemHost::default())).unwrap();
        // fill a pattern: hold A and sweep right on every row
        for _ in 0..8 {
            for _ in 0..16 {
                a.event(Btn::A, true);
                a.event(Btn::A, false);
                a.event(Btn::Right, true);
                a.event(Btn::Right, false);
            }
            a.event(Btn::Down, true);
            a.event(Btn::Down, false);
        }
        a.event(Btn::Start, true);
        a.event(Btn::Start, false);
        let mut buf = vec![0f32; 1024 * 2];
        let t = Instant::now();
        let secs = 10.0;
        let n = (44100.0 * secs / 1024.0) as usize;
        for _ in 0..n {
            a.audio(&mut buf);
        }
        let audio = t.elapsed().as_secs_f64();
        let mut fb = Fb::new();
        let t = Instant::now();
        for _ in 0..120 {
            a.render(&mut fb);
        }
        let frame = t.elapsed().as_secs_f64() / 120.0;
        println!("{:14} audio: {:5.1}x realtime ({:.1}% of one core)   render: {:.2} ms/frame", name, secs / audio, audio / secs * 100.0, frame * 1000.0);
    }
}
