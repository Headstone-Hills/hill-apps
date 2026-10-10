//! Dev tool: drive an app with a button script and write a PNG.
//! usage: shot <app> <out.png> [script]
//! script: space-separated tokens. `a`,`b`,`x`,`y`,`l1`,`r1`,`l2`,`r2`,`up`,`down`,`left`,`right`,`start`,`select`,`fn`
//! tap = press+release; `+name` = press only; `-name` = release; `wait:MS` runs audio+ticks.
use hill_core::{apps, fb::Fb, Btn, MemHost};

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for i in 0..256u32 {
        let mut c = i;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
        }
        table[i as usize] = c;
    }
    let mut c = 0xFFFFFFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 255) as usize] ^ (c >> 8);
    }
    !c
}

fn adler(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &d in data {
        a = (a + d as u32) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}

fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut c = ty.to_vec();
    c.extend_from_slice(data);
    out.extend_from_slice(&c);
    out.extend_from_slice(&crc32(&c).to_be_bytes());
}

fn png(fb: &Fb) -> Vec<u8> {
    let (w, h) = (640usize, 480usize);
    let mut raw = Vec::with_capacity((w * 3 + 1) * h);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let p = fb.px[y * w + x];
            raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut i = 0;
    while i < raw.len() {
        let n = (raw.len() - i).min(65535);
        z.push(if i + n == raw.len() { 1 } else { 0 });
        z.extend_from_slice(&(n as u16).to_le_bytes());
        z.extend_from_slice(&(!(n as u16)).to_le_bytes());
        z.extend_from_slice(&raw[i..i + n]);
        i += n;
    }
    z.extend_from_slice(&adler(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn btn(s: &str) -> Option<Btn> {
    Some(match s {
        "a" => Btn::A,
        "b" => Btn::B,
        "x" => Btn::X,
        "y" => Btn::Y,
        "l1" => Btn::L1,
        "r1" => Btn::R1,
        "l2" => Btn::L2,
        "r2" => Btn::R2,
        "up" => Btn::Up,
        "down" => Btn::Down,
        "left" => Btn::Left,
        "right" => Btn::Right,
        "start" => Btn::Start,
        "select" => Btn::Select,
        "fn" => Btn::Fn,
        _ => return None,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut app = apps::create(&args[1], Box::new(MemHost::default())).expect("app");
    let script = args.get(3).cloned().unwrap_or_default();
    let mut buf = vec![0f32; 2048];
    for tok in script.split_whitespace() {
        if let Some(ms) = tok.strip_prefix("wait:") {
            let ms: u32 = ms.parse().unwrap();
            let mut left = ms * 44;
            while left > 0 {
                let n = left.min(1024) as usize;
                app.audio(&mut buf[..n * 2]);
                app.tick((n as u32 * 1000 / 44100).max(1));
                left -= n as u32;
            }
        } else if let Some(n) = tok.strip_prefix('+') {
            app.event(btn(n).expect("btn"), true);
        } else if let Some(n) = tok.strip_prefix('-') {
            app.event(btn(n).expect("btn"), false);
        } else {
            let b = btn(tok).expect("btn");
            app.event(b, true);
            app.event(b, false);
        }
    }
    let mut fb = Fb::new();
    app.render(&mut fb);
    std::fs::write(&args[2], png(&fb)).unwrap();
}
