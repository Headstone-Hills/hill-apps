//! Shared overlays: the sample browser and the help sheet, plus small
//! drawing helpers.

use crate::fb::*;
use crate::input::Btn;
use crate::library::{matches_filter, Entry, DRUM_WORDS};

pub enum BrowserOut {
    None,
    Pick(usize),
    Fav(usize),
    Close,
}

#[derive(Default)]
pub struct Browser {
    pub open: bool,
    pub title: String,
    pub drum_filter: bool,
    pub fav_only: bool,
    pub favs_enabled: bool,
    view: Vec<usize>,
    sel: usize,
    top: usize,
}

const ROWS: usize = 17;

impl Browser {
    fn rebuild(&mut self, cat: &[Entry], favs: &[String]) {
        let keep = self.view.get(self.sel).copied();
        self.view = (0..cat.len())
            .filter(|&i| {
                let e = &cat[i];
                (!self.drum_filter || matches_filter(e, &DRUM_WORDS))
                    && (!self.fav_only || favs.contains(&e.key))
            })
            .collect();
        self.sel = keep.and_then(|k| self.view.iter().position(|&v| v == k)).unwrap_or(0);
        self.scroll_to_sel();
    }

    pub fn open(&mut self, title: &str, cat: &[Entry], drum_filter: bool, favs_enabled: bool, favs: &[String], current: Option<&str>) {
        self.open = true;
        self.title = title.to_string();
        self.drum_filter = drum_filter;
        self.favs_enabled = favs_enabled;
        self.fav_only = false;
        self.sel = 0;
        self.top = 0;
        self.rebuild(cat, favs);
        if let Some(k) = current {
            if let Some(p) = self.view.iter().position(|&i| cat[i].key == k) {
                self.sel = p;
                self.scroll_to_sel();
            }
        }
    }

    fn scroll_to_sel(&mut self) {
        if self.sel < self.top {
            self.top = self.sel;
        } else if self.sel >= self.top + ROWS {
            self.top = self.sel + 1 - ROWS;
        }
    }

    pub fn event(&mut self, b: Btn, cat: &[Entry], favs: &[String]) -> BrowserOut {
        let n = self.view.len();
        match b {
            Btn::Up if n > 0 => self.sel = (self.sel + n - 1) % n,
            Btn::Down if n > 0 => self.sel = (self.sel + 1) % n,
            Btn::L1 | Btn::Left if n > 0 => self.sel = self.sel.saturating_sub(ROWS),
            Btn::R1 | Btn::Right if n > 0 => self.sel = (self.sel + ROWS).min(n - 1),
            Btn::A if n > 0 => {
                self.open = false;
                return BrowserOut::Pick(self.view[self.sel]);
            }
            Btn::B => {
                self.open = false;
                return BrowserOut::Close;
            }
            Btn::Y if self.favs_enabled && n > 0 => return BrowserOut::Fav(self.view[self.sel]),
            Btn::X => {
                if self.drum_filter {
                    self.drum_filter = false;
                } else if self.favs_enabled {
                    self.fav_only = !self.fav_only;
                }
                self.rebuild(cat, favs);
            }
            _ => {}
        }
        self.scroll_to_sel();
        BrowserOut::None
    }

    pub fn refresh(&mut self, cat: &[Entry], favs: &[String]) {
        self.rebuild(cat, favs);
    }

    pub fn draw(&self, fb: &mut Fb, cat: &[Entry], favs: &[String]) {
        fb.dim(0.35);
        fb.rect(20, 14, 600, 452, PANEL);
        fb.frame(20, 14, 600, 452, 2, DIM);
        fb.text(34, 24, &self.title, 2, ACCENT);
        let mut flt = String::new();
        if self.drum_filter {
            flt.push_str("[DRUM] ");
        }
        if self.fav_only {
            flt.push_str("[FAV] ");
        }
        fb.text_right(606, 28, &flt, 1, MELODIC);
        if self.view.is_empty() {
            fb.text(34, 70, "Nothing here.", 2, DIM);
        }
        let mut y = 52;
        let end = (self.top + ROWS).min(self.view.len());
        for vi in self.top..end {
            let e = &cat[self.view[vi]];
            let sel = vi == self.sel;
            if sel {
                fb.rect(28, y - 2, 584, 22, scale(MELODIC, 0.45));
            }
            let star = if favs.contains(&e.key) { "*" } else { " " };
            fb.text(34, y + 1, star, 2, rgb(255, 210, 80));
            fb.text(58, y + 1, &clip(&e.label, 30), 2, if sel { WHITE } else { TEXT });
            fb.text_right(606, y + 5, &clip(&e.dir, 20), 1, DIM);
            y += 24;
        }
        let pos = if self.view.is_empty() { 0 } else { self.sel + 1 };
        fb.text(34, 446, &format!("{}/{}", pos, self.view.len()), 1, DIM);
        let hint = if self.favs_enabled {
            "A pick  B close  Y fav  X filter  L1/R1 page"
        } else if self.drum_filter {
            "A pick  B close  X clear filter  L1/R1 page"
        } else {
            "A pick  B close  L1/R1 page"
        };
        fb.text_right(606, 446, hint, 1, DIM);
    }
}

/// Scrollable cheat sheet.
#[derive(Default)]
pub struct Help {
    pub open: bool,
    scroll: usize,
}

const HELP_ROWS: usize = 18;

impl Help {
    pub fn show(&mut self) {
        self.open = true;
        self.scroll = 0;
    }

    pub fn event(&mut self, b: Btn, total: usize) {
        match b {
            Btn::A | Btn::B => self.open = false,
            Btn::Down => self.scroll = (self.scroll + 1).min(total.saturating_sub(HELP_ROWS)),
            Btn::Up => self.scroll = self.scroll.saturating_sub(1),
            _ => {}
        }
    }

    pub fn draw(&self, fb: &mut Fb, title: &str, lines: &[(&str, &str)]) {
        fb.dim(0.3);
        fb.rect(14, 10, 612, 460, PANEL);
        fb.frame(14, 10, 612, 460, 2, DIM);
        fb.text(28, 20, &format!("{} controls", title), 2, ACCENT);
        let mut y = 48;
        for (k, v) in lines.iter().skip(self.scroll).take(HELP_ROWS) {
            if v.is_empty() {
                fb.text(28, y + 3, k, 1, MELODIC);
            } else {
                fb.text(28, y + 3, k, 1, TEXT);
                fb.text(228, y + 3, v, 1, DIM);
            }
            y += 22;
        }
        if lines.len() > HELP_ROWS {
            fb.text_right(612, 452, "Up/Down scroll   A/B close", 1, DIM);
        } else {
            fb.text_right(612, 452, "A/B close", 1, DIM);
        }
    }
}

/// How long START+SELECT must be held to quit, in every app.
pub const QUIT_HOLD_MS: u32 = 1500;

/// Progress bar shown while START+SELECT is held, so the quit gesture is visible.
pub fn quit_bar(fb: &mut Fb, held_ms: u32) {
    if held_ms < 250 {
        return;
    }
    let w = 360;
    let x = (W - w) / 2;
    fb.rect(x - 12, 180, w + 24, 84, rgb(10, 12, 16));
    fb.frame(x - 12, 180, w + 24, 84, 2, ACCENT);
    fb.text_center(W / 2, 194, "Hold to quit", 3, WHITE);
    fb.frame(x, 236, w, 18, 2, DIM);
    let fill = (w - 4) * held_ms.min(QUIT_HOLD_MS) as i32 / QUIT_HOLD_MS as i32;
    fb.rect(x + 2, 238, fill, 14, ACCENT);
}

/// Transient one-line message.
#[derive(Default)]
pub struct Toast {
    pub text: String,
    left_ms: u32,
}

impl Toast {
    pub fn say(&mut self, s: &str) {
        self.text = s.to_string();
        self.left_ms = 1600;
    }
    pub fn tick(&mut self, dt: u32) {
        self.left_ms = self.left_ms.saturating_sub(dt);
    }
    pub fn draw(&self, fb: &mut Fb) {
        if self.left_ms > 0 {
            let w = Fb::text_w(&self.text, 2) + 24;
            let x = (W - w) / 2;
            fb.rect(x, 404, w, 32, rgb(10, 12, 16));
            fb.frame(x, 404, w, 32, 2, ACCENT);
            fb.text(x + 12, 412, &self.text, 2, WHITE);
        }
    }
}

/// A small centered modal frame; returns the inner top-left.
pub fn modal(fb: &mut Fb, w: i32, h: i32, title: &str) -> (i32, i32) {
    fb.dim(0.35);
    let x = (W - w) / 2;
    let y = (H - h) / 2;
    fb.rect(x, y, w, h, PANEL);
    fb.frame(x, y, w, h, 2, DIM);
    fb.text(x + 14, y + 12, title, 2, ACCENT);
    (x + 14, y + 44)
}
