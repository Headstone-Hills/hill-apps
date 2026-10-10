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

/// One line of the folder view.
#[derive(Clone, PartialEq)]
enum Row {
    /// A sub-folder: (name, full path, number of sounds below it).
    Dir(String, String, usize),
    /// A sound: index into the catalog.
    Item(usize),
}

/// Folder-tree sound browser. Folders come from each entry's `dir`
/// (`Casio VL-1/Casio VL-1 Piano`), so the card's own folder layout is the tree.
#[derive(Default)]
pub struct Browser {
    pub open: bool,
    pub title: String,
    pub drum_filter: bool,
    pub fav_only: bool,
    pub favs_enabled: bool,
    cwd: String,
    rows: Vec<Row>,
    sel: usize,
    top: usize,
}

const ROWS: usize = 17;

fn parent_of(p: &str) -> String {
    p.rfind('/').map(|i| p[..i].to_string()).unwrap_or_default()
}

impl Browser {
    fn rebuild(&mut self, cat: &[Entry], favs: &[String]) {
        let keep = self.rows.get(self.sel).cloned();
        let prefix = if self.cwd.is_empty() { String::new() } else { format!("{}/", self.cwd) };
        let mut dirs: Vec<(String, usize, usize)> = Vec::new(); // (name, count, last entry)
        let mut items: Vec<usize> = Vec::new();
        for (i, e) in cat.iter().enumerate() {
            if self.drum_filter && !matches_filter(e, &DRUM_WORDS) {
                continue;
            }
            if self.fav_only && !favs.contains(&e.key) {
                continue;
            }
            if e.dir == self.cwd {
                items.push(i);
            } else if let Some(rest) = e.dir.strip_prefix(prefix.as_str()) {
                let name = rest.split('/').next().unwrap_or(rest).to_string();
                match dirs.iter_mut().find(|d| d.0 == name) {
                    Some(d) => {
                        d.1 += 1;
                        d.2 = i;
                    }
                    None => dirs.push((name, 1, i)),
                }
            }
        }
        let mut rows: Vec<Row> = Vec::new();
        dirs.sort_by_key(|d| d.0.to_lowercase());
        for (name, count, last) in dirs {
            if count == 1 {
                items.push(last); // a folder holding one sound: show the sound itself
            } else {
                rows.push(Row::Dir(name.clone(), format!("{}{}", prefix, name), count));
            }
        }
        items.sort();
        rows.extend(items.into_iter().map(Row::Item));
        self.rows = rows;
        self.sel = keep.and_then(|k| self.rows.iter().position(|r| *r == k)).unwrap_or(0);
        self.scroll_to_sel();
    }

    fn go(&mut self, cwd: String, select: Option<Row>, cat: &[Entry], favs: &[String]) {
        self.cwd = cwd;
        self.rows.clear();
        self.sel = 0;
        self.top = 0;
        self.rebuild(cat, favs);
        if let Some(r) = select {
            if let Some(p) = self.rows.iter().position(|x| *x == r) {
                self.sel = p;
                self.scroll_to_sel();
            }
        }
    }

    pub fn open(&mut self, title: &str, cat: &[Entry], drum_filter: bool, favs_enabled: bool, favs: &[String], current: Option<&str>) {
        self.open = true;
        self.title = title.to_string();
        self.drum_filter = drum_filter;
        self.favs_enabled = favs_enabled;
        self.fav_only = false;
        let cur = current.and_then(|k| cat.iter().position(|e| e.key == k));
        // start in the folder of the current sound, if the filter lets it through
        if let Some(i) = cur {
            self.go(cat[i].dir.clone(), Some(Row::Item(i)), cat, favs);
            if self.rows.get(self.sel) == Some(&Row::Item(i)) {
                return;
            }
        }
        self.go(String::new(), None, cat, favs);
    }

    fn scroll_to_sel(&mut self) {
        if self.sel < self.top {
            self.top = self.sel;
        } else if self.sel >= self.top + ROWS {
            self.top = self.sel + 1 - ROWS;
        }
    }

    fn up(&mut self, cat: &[Entry], favs: &[String]) -> bool {
        if self.cwd.is_empty() {
            return false;
        }
        let from = self.cwd.clone();
        // the folder may sit inside a chain of single-child levels: climb until rows exist
        let mut to = parent_of(&from);
        loop {
            self.go(to.clone(), None, cat, favs);
            let found = self.rows.iter().position(|r| matches!(r, Row::Dir(_, full, _) if *full == from));
            if let Some(p) = found {
                self.sel = p;
                self.scroll_to_sel();
                break;
            }
            if !self.rows.is_empty() || to.is_empty() {
                break;
            }
            to = parent_of(&to);
        }
        true
    }

    pub fn event(&mut self, b: Btn, cat: &[Entry], favs: &[String]) -> BrowserOut {
        let n = self.rows.len();
        match b {
            Btn::Up if n > 0 => self.sel = (self.sel + n - 1) % n,
            Btn::Down if n > 0 => self.sel = (self.sel + 1) % n,
            Btn::L1 if n > 0 => self.sel = self.sel.saturating_sub(ROWS),
            Btn::R1 if n > 0 => self.sel = (self.sel + ROWS).min(n - 1),
            Btn::Left => {
                self.up(cat, favs);
            }
            Btn::Right | Btn::A if n > 0 => match self.rows[self.sel].clone() {
                Row::Dir(_, full, _) => self.go(full, None, cat, favs),
                Row::Item(i) if b == Btn::A => {
                    self.open = false;
                    return BrowserOut::Pick(i);
                }
                Row::Item(_) => {}
            },
            Btn::B => {
                if !self.up(cat, favs) {
                    self.open = false;
                    return BrowserOut::Close;
                }
            }
            Btn::Y if self.favs_enabled && n > 0 => {
                if let Row::Item(i) = self.rows[self.sel] {
                    return BrowserOut::Fav(i);
                }
            }
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
        let pos = if self.rows.is_empty() { 0 } else { self.sel + 1 };
        flt.push_str(&format!("{}/{}", pos, self.rows.len()));
        fb.text_right(606, 28, &flt, 1, MELODIC);
        if self.rows.is_empty() {
            fb.text(34, 70, "Nothing here.", 2, DIM);
        }
        let mut y = 52;
        let end = (self.top + ROWS).min(self.rows.len());
        for vi in self.top..end {
            let sel = vi == self.sel;
            if sel {
                fb.rect(28, y - 2, 584, 22, scale(MELODIC, 0.45));
            }
            match &self.rows[vi] {
                Row::Dir(name, _, count) => {
                    fb.text(34, y + 1, ">", 2, ACCENT);
                    fb.text(58, y + 1, &clip(name, 30), 2, if sel { WHITE } else { ACCENT });
                    fb.text_right(606, y + 5, &format!("{}", count), 1, DIM);
                }
                Row::Item(i) => {
                    let e = &cat[*i];
                    let star = if favs.contains(&e.key) { "*" } else { " " };
                    fb.text(34, y + 1, star, 2, rgb(255, 210, 80));
                    fb.text(58, y + 1, &clip(&e.label, 30), 2, if sel { WHITE } else { TEXT });
                    if e.dir != self.cwd {
                        let leaf = e.dir.rsplit('/').next().unwrap_or(&e.dir);
                        fb.text_right(606, y + 5, &clip(leaf, 20), 1, DIM);
                    }
                }
            }
            y += 24;
        }
        let path = if self.cwd.is_empty() { "/".to_string() } else { format!("/{}", self.cwd) };
        let shown: String = if path.chars().count() > 26 { format!("..{}", path.chars().skip(path.chars().count() - 24).collect::<String>()) } else { path };
        fb.text(34, 446, &shown, 1, ACCENT);
        let hint = if self.favs_enabled {
            "A pick/open  B back  Y fav  X filter  L1/R1 page"
        } else if self.drum_filter {
            "A pick/open  B back  X all sounds  L1/R1 page"
        } else {
            "A pick/open  B back  L1/R1 page"
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
