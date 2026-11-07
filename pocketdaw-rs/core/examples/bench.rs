//! Mixer cost benchmark: `cargo run --release -p pocketdaw-core --example bench`.
//! Renders 8 clip tracks + click + tone (a busy project) in 1024-frame chunks and reports the time per chunk
//! against the 23.2 ms the audio callback has. The Python mixer needed ~2 ms for a click + tone alone on the SP.

use pocketdaw_core::mixer::{Click, Clip, Mixer, Tone, Track};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let rate = 44100u32;
    let chunk = 1024usize;
    let mut m = Mixer::new(rate, chunk);
    for i in 0..8 {
        let data: Vec<f32> = (0..rate as usize * 2 * 20).map(|n| ((n as f32) * (0.001 + i as f32 * 0.0003)).sin() * 0.2).collect();
        let mut t = Track::new("t");
        t.pan = (i as f32 - 3.5) / 4.0;
        t.sources.push(Box::new(Clip { data: Arc::new(data), offset: 0, looped: true, gain: 1.0 }));
        m.tracks.push(t);
    }
    let mut c = Track::new("click");
    c.sources.push(Box::new(Click { bpm: 120.0, beats_per_bar: 4, amp: 0.4, enabled: true }));
    c.sources.push(Box::new(Tone { freq: 220.0, amp: 0.1 }));
    m.tracks.push(c);

    let mut out = vec![0.0f32; chunk * 2];
    let n = 4000; // ~93 s of audio
    for i in 0..200 {
        m.render(i * chunk as u64, &mut out); // warm up
    }
    let mut worst = 0.0f64;
    let t0 = Instant::now();
    for i in 0..n {
        let s = Instant::now();
        m.render((200 + i) * chunk as u64, &mut out);
        worst = worst.max(s.elapsed().as_secs_f64());
    }
    let avg_ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
    let period_ms = chunk as f64 * 1000.0 / rate as f64;
    println!("9 tracks (8 clips + click/tone), {} frames/chunk", chunk);
    println!("render: avg {:.3} ms   worst {:.3} ms   budget {:.1} ms   CPU use ~{:.1}%", avg_ms, worst * 1000.0, period_ms, 100.0 * avg_ms / period_ms);
    println!("headroom: {:.0}x", period_ms / avg_ms);
}
