//! HillBeat's preset pattern menu: a style list, then that style's patterns
//! (rhythms, breaks, endings) with a step-grid preview of the highlighted one.
//! Navigation and drawing only; `GridApp` loads and auditions.

use crate::fb::*;
use crate::input::Btn;
use crate::presets::{voice_of, Kind, Preset, Presets, FLAM_BIT};

pub enum MenuOut {
    None,
    Load(usize),
    Audition,
    Close,
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Header(Kind),
    Item(usize),
}

#[derive(Default)]
pub struct PresetMenu {
    in_style: bool,
    style: usize,
    style_top: usize,
    rows: Vec<Row>,
    sel: usize,
    top: usize,
    /// Pattern waiting for "replace the current pattern?" (A yes / B no).
    pub confirm: Option<usize>,
    /// A line for the user (why audition is off, ...).
    pub note: String,
}

const ROWS: usize = 17;

impl PresetMenu {
    /// Reopening keeps the last style and pattern.
    pub fn open(&mut self, p: &Presets) {
        self.confirm = None;
        self.note.clear();
        self.style = self.style.min(p.styles.len().saturating_sub(1));
        if self.in_style {
            let keep = self.highlighted();
            self.enter(p, self.style, keep);
        }
    }

    /// Pattern under the cursor, in a pattern list.
    pub fn highlighted(&self) -> Option<usize> {
        match self.rows.get(self.sel) {
            Some(Row::Item(i)) if self.in_style => Some(*i),
            _ => None,
        }
    }

    /// (style index, highlighted pattern) for saving the position.
    pub fn position(&self) -> (usize, Option<usize>) {
        (self.style, self.highlighted())
    }

    /// Restore a saved position (the style list, with this style highlighted;
    /// or this pattern's list when a pattern is given).
    pub fn restore(&mut self, p: &Presets, style: usize, pattern: Option<usize>) {
        if style >= p.styles.len() {
            return;
        }
        self.style = style;
        self.scroll_styles(p);
        if pattern.is_some() {
            self.enter(p, style, pattern);
        }
    }

    fn enter(&mut self, p: &Presets, style: usize, select: Option<usize>) {
        self.in_style = true;
        self.style = style;
        self.rows.clear();
        let mut last = None;
        for i in p.of_style(style) {
            let k = p.patterns[i].kind;
            if last != Some(k) {
                self.rows.push(Row::Header(k));
                last = Some(k);
            }
            self.rows.push(Row::Item(i));
        }
        self.sel = select.and_then(|s| self.rows.iter().position(|r| *r == Row::Item(s))).unwrap_or(1.min(self.rows.len().saturating_sub(1)));
        self.top = 0;
        self.scroll();
    }

    fn scroll(&mut self) {
        // keep a kind header in view above the first pattern
        let want_top = if self.sel == 1 { 0 } else { self.sel };
        if want_top < self.top {
            self.top = want_top;
        } else if self.sel >= self.top + ROWS {
            self.top = self.sel + 1 - ROWS;
        }
    }

    fn scroll_styles(&mut self, p: &Presets) {
        if self.style < self.style_top {
            self.style_top = self.style;
        } else if self.style >= self.style_top + ROWS {
            self.style_top = self.style + 1 - ROWS;
        }
        self.style_top = self.style_top.min(p.styles.len().saturating_sub(ROWS));
    }

    /// Move to the next/previous pattern row, skipping headers, wrapping at the ends.
    fn step(&mut self, d: isize) {
        let n = self.rows.len() as isize;
        if n == 0 {
            return;
        }
        let mut i = self.sel as isize;
        for _ in 0..n {
            i = (i + d).rem_euclid(n);
            if matches!(self.rows[i as usize], Row::Item(_)) {
                break;
            }
        }
        self.sel = i as usize;
        self.scroll();
    }

    pub fn event(&mut self, b: Btn, p: &Presets) -> MenuOut {
        let ns = p.styles.len();
        if ns == 0 {
            return if b == Btn::B { MenuOut::Close } else { MenuOut::None };
        }
        if !self.in_style {
            match b {
                Btn::Up => self.style = (self.style + ns - 1) % ns,
                Btn::Down => self.style = (self.style + 1) % ns,
                Btn::A | Btn::Right => self.enter(p, self.style, None),
                Btn::B => return MenuOut::Close,
                _ => {}
            }
            self.scroll_styles(p);
            return MenuOut::None;
        }
        match b {
            Btn::Up => self.step(-1),
            Btn::Down => self.step(1),
            Btn::A => {
                if let Some(i) = self.highlighted() {
                    return MenuOut::Load(i);
                }
            }
            Btn::B | Btn::Left => {
                self.in_style = false;
                self.rows.clear();
                self.scroll_styles(p);
            }
            Btn::L1 => self.enter(p, (self.style + ns - 1) % ns, None),
            Btn::R1 => self.enter(p, (self.style + 1) % ns, None),
            Btn::X => return MenuOut::Audition,
            _ => {}
        }
        MenuOut::None
    }

    /// `voices`: the names of HillBeat's voices, to label the preview rows.
    pub fn draw(&self, fb: &mut Fb, p: &Presets, voices: &[String], auditioning: bool) {
        fb.dim(0.35);
        fb.rect(20, 14, 600, 452, PANEL);
        fb.frame(20, 14, 600, 452, 2, DIM);
        let title = if self.in_style { format!("Patterns / {}", clip(&p.styles[self.style], 18)) } else { "Drum patterns".to_string() };
        fb.text(34, 24, &title, 2, ACCENT);
        let mut y = 52;
        if !self.in_style {
            let end = (self.style_top + ROWS).min(p.styles.len());
            for si in self.style_top..end {
                let sel = si == self.style;
                if sel {
                    fb.rect(28, y - 2, 270, 22, scale(MELODIC, 0.45));
                }
                fb.text(34, y + 1, ">", 2, ACCENT);
                fb.text(58, y + 1, &clip(&p.styles[si], 14), 2, if sel { WHITE } else { TEXT });
                fb.text_right(292, y + 5, &format!("{}", p.of_style(si).len()), 1, DIM);
                y += 24;
            }
            self.draw_style_info(fb, p);
            fb.text_right(606, 446, "A open  B close  START+L1 close", 1, DIM);
        } else {
            let end = (self.top + ROWS).min(self.rows.len());
            for vi in self.top..end {
                match self.rows[vi] {
                    Row::Header(k) => {
                        fb.text(34, y + 5, k.header(), 1, MELODIC);
                    }
                    Row::Item(i) => {
                        let sel = vi == self.sel;
                        let x = &p.patterns[i];
                        if sel {
                            fb.rect(28, y - 2, 270, 22, scale(MELODIC, 0.45));
                        }
                        fb.text(34, y + 1, &clip(&x.name, 13), 2, if sel { WHITE } else { TEXT });
                        if let Some(t) = x.tag() {
                            fb.text_right(292, y + 5, t, 1, DRUM);
                        }
                    }
                }
                y += 24;
            }
            if let Some(i) = self.highlighted() {
                draw_preview(fb, &p.patterns[i], voices);
            }
            let hint = if auditioning { "A load  X stop  B back  L1/R1 style" } else { "A load  X listen  B back  L1/R1 style" };
            fb.text_right(606, 446, hint, 1, DIM);
        }
        if !self.note.is_empty() {
            fb.text(34, 446, &clip(&self.note, 36), 1, WARN);
        } else if auditioning {
            fb.text(34, 446, "listening...", 1, ACCENT);
        }
        if let Some(i) = self.confirm {
            let (x, y) = crate::ui::modal(fb, 440, 150, "Replace pattern?");
            fb.text(x, y + 4, &format!("Load {}", clip(&p.patterns[i].name, 18)), 2, WHITE);
            fb.text(x, y + 30, "over the notes in this pattern.", 1, DIM);
            fb.text(x, y + 66, "A replace   B keep", 1, ACCENT);
        }
    }

    fn draw_style_info(&self, fb: &mut Fb, p: &Presets) {
        let (x, y) = (316, 56);
        let list = p.of_style(self.style);
        let count = |k: Kind| list.iter().filter(|&&i| p.patterns[i].kind == k).count();
        let name = &p.styles[self.style];
        if name.chars().count() > 18 {
            fb.text(x, y + 4, &clip(name, 36), 1, WHITE);
        } else {
            fb.text(x, y, name, 2, WHITE);
        }
        let mut ly = y + 32;
        for (k, label) in [(Kind::Rhythm, "rhythms"), (Kind::Break, "breaks"), (Kind::Ending, "endings")] {
            let n = count(k);
            if n > 0 {
                fb.text(x, ly, &format!("{} {}", n, label), 2, TEXT);
                ly += 24;
            }
        }
        let tags: Vec<&str> = {
            let mut t: Vec<&str> = list.iter().filter_map(|&i| p.patterns[i].tag()).collect();
            t.dedup();
            t
        };
        if !tags.is_empty() {
            fb.text(x, ly + 8, &format!("feel: {}", tags.join(", ")), 1, DRUM);
        }
        fb.text(x, 380, "Loading replaces the notes of the", 1, DIM);
        fb.text(x, 394, "current pattern. Tempo, swing,", 1, DIM);
        fb.text(x, 408, "mute and sounds stay as they are.", 1, DIM);
    }
}

/// Mini step grid: one line per used row (as in the data), with the voice it plays on.
fn draw_preview(fb: &mut Fb, x: &Preset, voices: &[String]) {
    let (px, py) = (316, 56);
    fb.text(px, py, &clip(&x.name, 18), 2, WHITE);
    let kind = match x.kind {
        Kind::Rhythm => "rhythm",
        Kind::Break => "break",
        Kind::Ending => "ending",
    };
    let feel = x.tag().unwrap_or("16ths");
    fb.text(px, py + 24, &format!("{}  {}  {} steps  {}  p.{}", kind, x.time_sig, x.steps, feel, x.book_page), 1, DIM);

    let gx = px + 34;
    let cw = (256 / x.steps as i32).min(16);
    let rh = 16;
    let mut y = py + 46;
    // beat numbers over the grid
    for s in (0..x.steps).step_by(x.div as usize) {
        let cx = gx + s as i32 * cw;
        fb.text(cx + 1, y, &format!("{}", s / x.div as usize + 1), 1, DIM);
    }
    y += 12;
    let cells = x.to_cells(voices.len());
    for (key, pat) in &x.tracks {
        let v = voice_of(key);
        let base = if v.is_some() { DRUM } else { ACCENT };
        fb.text(px, y + 4, key, 1, if v.is_some() { TEXT } else { ACCENT });
        for (s, c) in pat.chars().enumerate() {
            let cx = gx + s as i32 * cw;
            let beat = (s / x.div as usize) % 2 == 0;
            fb.rect(cx, y, cw - 1, rh - 2, if beat { CELL_BEAT } else { CELL_OFF });
            if c != '.' {
                fb.rect(cx, y, cw - 1, rh - 2, base);
            }
            if c == 'F' {
                // flam: a small white grace mark on the left of the hit
                fb.rect(cx + 1, y + 2, 3, rh - 6, WHITE);
            }
        }
        y += rh;
        if y > 330 {
            break;
        }
    }
    // which voice each sounding row lands on
    let mut ly = y.max(py + 160) + 10;
    for (vi, name) in voices.iter().enumerate() {
        let rows: Vec<&str> = x.tracks.iter().filter(|(k, _)| voice_of(k) == Some(vi)).map(|(k, _)| k.as_str()).collect();
        if rows.is_empty() {
            continue;
        }
        let flams = cells[vi].iter().any(|c| c & FLAM_BIT != 0);
        let line = format!("{} > {} {}{}", rows.join("+"), vi + 1, clip(name, 10), if flams { "  (flam)" } else { "" });
        fb.text(px, ly, &clip(&line, 36), 1, TEXT);
        ly += 14;
    }
    if x.tracks.iter().any(|(k, _)| k == "AC") {
        fb.text(px, ly, "AC = accent: louder hits there", 1, ACCENT);
        ly += 14;
    }
    if ly < 420 {
        fb.text(px, 422, "white mark = flam", 1, DIM);
    }
}
