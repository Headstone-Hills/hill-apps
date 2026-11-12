//! How long taking an echo out of a 3 minute take takes: `cargo run --release -p hilldaw-core --example echobench`
use hilldaw_core::echo::cancel;
use std::time::Instant;

fn main() {
    let n = 44_100 * 180;
    let mut s = 1u32;
    let mut noise = |k: usize| -> Vec<f32> {
        (0..k)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 8) as f32 / (1 << 23) as f32 - 1.0
            })
            .collect()
    };
    let reference = noise(n + 10_000);
    let wanted = noise(n);
    let mut y: Vec<f32> = (0..n).map(|c| 0.1 * wanted[c] + 0.7 * reference[c + 4_000]).collect();
    let t = Instant::now();
    let r = cancel(&mut y, &reference, 30_000);
    println!("{:?} in {:.2} s", r, t.elapsed().as_secs_f64());
}
