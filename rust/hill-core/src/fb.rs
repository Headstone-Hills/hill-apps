//! 640x480 software framebuffer with an 8x8 bitmap font. Pixels are
//! `0x00RRGGBB`.

use font8x8::{UnicodeFonts, BASIC_FONTS};

pub const W: i32 = 640;
pub const H: i32 = 480;

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

pub const BG: u32 = rgb(14, 17, 22);
pub const PANEL: u32 = rgb(26, 31, 40);
pub const CELL_OFF: u32 = rgb(38, 44, 56);
pub const CELL_BEAT: u32 = rgb(48, 56, 72);
pub const TEXT: u32 = rgb(222, 228, 238);
pub const DIM: u32 = rgb(118, 128, 146);
pub const WHITE: u32 = rgb(255, 255, 255);
pub const MELODIC: u32 = rgb(70, 140, 255);
pub const DRUM: u32 = rgb(255, 150, 50);
pub const ACCENT: u32 = rgb(120, 220, 160);
pub const WARN: u32 = rgb(255, 90, 90);

pub fn scale(c: u32, f: f32) -> u32 {
    let ch = |s: u32| (((c >> s) & 255) as f32 * f).clamp(0.0, 255.0) as u32;
    ch(16) << 16 | ch(8) << 8 | ch(0)
}

pub struct Fb {
    pub px: Vec<u32>,
}

impl Default for Fb {
    fn default() -> Self {
        Self::new()
    }
}

impl Fb {
    pub fn new() -> Self {
        Fb { px: vec![BG; (W * H) as usize] }
    }

    pub fn clear(&mut self, c: u32) {
        self.px.iter_mut().for_each(|p| *p = c);
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(W);
        let y1 = (y + h).min(H);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        for yy in y0..y1 {
            let row = (yy * W) as usize;
            self.px[row + x0 as usize..row + x1 as usize].fill(c);
        }
    }

    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, t: i32, c: u32) {
        self.rect(x, y, w, t, c);
        self.rect(x, y + h - t, w, t, c);
        self.rect(x, y, t, h, c);
        self.rect(x + w - t, y, t, h, c);
    }

    /// Darken everything (used behind modal overlays).
    pub fn dim(&mut self, f: f32) {
        for p in self.px.iter_mut() {
            *p = scale(*p, f);
        }
    }

    pub fn text_w(s: &str, sc: i32) -> i32 {
        s.chars().count() as i32 * 8 * sc
    }

    /// Draw text; returns the x position after the last glyph.
    pub fn text(&mut self, x: i32, y: i32, s: &str, sc: i32, c: u32) -> i32 {
        let mut cx = x;
        for ch in s.chars() {
            let g = BASIC_FONTS
                .get(ch)
                .or_else(|| BASIC_FONTS.get('?'))
                .unwrap_or([0; 8]);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..8 {
                    if bits >> col & 1 != 0 {
                        self.rect(cx + col * sc, y + row as i32 * sc, sc, sc, c);
                    }
                }
            }
            cx += 8 * sc;
        }
        cx
    }

    pub fn text_center(&mut self, cx: i32, y: i32, s: &str, sc: i32, c: u32) {
        let w = Self::text_w(s, sc);
        self.text(cx - w / 2, y, s, sc, c);
    }

    pub fn text_right(&mut self, rx: i32, y: i32, s: &str, sc: i32, c: u32) {
        let w = Self::text_w(s, sc);
        self.text(rx - w, y, s, sc, c);
    }

    /// Convert to RGBA bytes (canvas ImageData order).
    pub fn to_rgba(&self, out: &mut [u8]) {
        for (i, &p) in self.px.iter().enumerate() {
            let o = i * 4;
            if o + 3 >= out.len() {
                break;
            }
            out[o] = (p >> 16) as u8;
            out[o + 1] = (p >> 8) as u8;
            out[o + 2] = p as u8;
            out[o + 3] = 255;
        }
    }
}

/// Truncate to at most `n` characters, marking cuts with a trailing `~`.
pub fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('~');
        t
    }
}
