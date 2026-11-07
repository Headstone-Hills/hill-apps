//! Drawing. Everything here is `&self`: the UI thread holds the same mutex as the audio callback while it draws.

use super::*;
use hill_core::fb::{rgb, ACCENT, BG, CELL_BEAT, CELL_OFF, DIM, DRUM, MELODIC, PANEL, TEXT, WARN, WHITE};
use hill_core::ui::quit_bar;
use hilldaw_device::{fmt_bytes, fmt_duration, record_seconds_left};

const AMBER: u32 = rgb(255, 200, 60);
const METER_TOP: i32 = 150;
const METER_H: i32 = 170;
const STRIP_W: i32 = 62;
const STRIP_X0: i32 = 12;

/// 0..1 position of a linear level on the shared -60..+6 dB scale (meters and fader marks use the same one).
pub(crate) fn meter_frac(level: f32) -> f32 {
    ((gain_to_db(level) + 60.0) / 66.0).clamp(0.0, 1.0)
}

/// Fill a meter from the bottom up to `frac`, green below -9 dBFS, amber to -1 dBFS, red above.
fn fill_bar(fb: &mut Fb, x: i32, w: i32, frac: f32) {
    let h = (frac * METER_H as f32) as i32;
    for (lo, hi, c) in [(0.0f32, 0.77f32, ACCENT), (0.77, 0.9, AMBER), (0.9, 1.0, WARN)] {
        let lo = (lo * METER_H as f32) as i32;
        let hi = ((hi * METER_H as f32) as i32).min(h);
        if hi > lo {
            fb.rect(x, METER_TOP + METER_H - hi, w, hi - lo, c);
        }
    }
}

pub(crate) fn db_text(g: f32) -> String {
    if g <= 0.0 {
        return "-inf".into();
    }
    let db = gain_to_db(g).round() as i32;
    if db == 0 { "0".into() } else { format!("{db:+}") }
}

pub(crate) fn pan_text(p: f32) -> String {
    let n = (p.abs() * 100.0).round() as i32;
    if n == 0 { "C".into() } else if p < 0.0 { format!("L{n}") } else { format!("R{n}") }
}

impl HillDaw {
    pub(crate) fn draw(&self, fb: &mut Fb) {
        fb.clear(BG);
        self.draw_status_bar(fb);
        self.draw_transport(fb);
        match self.view {
            View::Mixer => self.draw_mixer(fb),
            View::Arrange => self.draw_arrange(fb),
            View::Fm1 => self.draw_fm1(fb),
        }
        self.draw_footer(fb);
        self.draw_clock(fb);
        self.toast.draw(fb);
        if self.help.open {
            self.help.draw(fb, "HillDAW", HELP);
        }
        quit_bar(fb, self.quit_ms);
    }

    fn draw_status_bar(&self, fb: &mut Fb) {
        fb.rect(0, 0, 640, 30, PANEL);
        fb.text(10, 7, "HILLDAW", 2, ACCENT);
        // battery
        let s = &self.status;
        let (bat, bc) = match s.battery_pct {
            Some(p) => (format!("BAT {p}%{}", if s.charging { "+" } else { "" }), if s.battery_low { WARN } else { TEXT }),
            None => ("BAT --".to_string(), DIM),
        };
        let mut x = 640 - 10;
        fb.text_right(x, 11, &bat, 1, bc);
        x -= Fb::text_w(&bat, 1) + 18;
        // storage + how long a mono 24-bit take would fit
        let (sd, sc) = match s.free_bytes {
            Some(f) => (format!("{} = {}", fmt_bytes(f), fmt_duration(record_seconds_left(f, self.rate, 3, 1))), TEXT),
            None => ("SD --".to_string(), DIM),
        };
        fb.text_right(x, 11, &sd, 1, sc);
        x -= Fb::text_w(&sd, 1) + 18;
        // FM-1 on the USB bus
        let (fm, fc) = if s.fm1 { ("FM-1", ACCENT) } else { ("FM-1 --", DIM) };
        fb.text_right(x, 11, fm, 1, fc);
        fb.rect(x - Fb::text_w(fm, 1) - 12, 12, 7, 7, fc);
    }

    fn draw_transport(&self, fb: &mut Fb) {
        let t = &self.transport;
        let (bar, beat) = t.bar_beat();
        fb.rect(10, 38, 620, 92, PANEL);
        fb.text(22, 48, &format!("{bar:03}:{beat}"), 5, WHITE);
        let secs = t.pos as f64 / t.rate.max(1) as f64;
        fb.text(22, 98, &format!("{:02}:{:04.1}", (secs / 60.0) as u32, secs % 60.0), 2, DIM);
        let (state, sc) = if t.playing { ("PLAY", ACCENT) } else { ("STOP", DIM) };
        fb.text(246, 50, state, 3, sc);
        // the REC lamp is honest: it shows n/a when no recorder is connected
        let (rec_text, rec_col) = match &self.rec {
            Some(r) if r.counting_in() => ("COUNT-IN".to_string(), AMBER),
            Some(r) => {
                let secs = r.elapsed / t.rate.max(1) as u64;
                (format!("REC {:02}:{:02}", secs / 60, secs % 60), WARN)
            }
            None if self.has_recorder() => ("REC".to_string(), DIM),
            None => ("REC n/a".to_string(), DIM),
        };
        fb.text(246, 84, &rec_text, 2, rec_col);
        fb.text(400, 48, &format!("{:.0}", self.project.bpm), 4, TEXT);
        fb.text(400 + 3 * 32 + 6, 62, "BPM", 2, DIM);
        let sw = 50 + (self.project.drums.swing * 100.0).round() as i32;
        fb.text(400, 90, &format!("{}/4  swing {sw}%", self.project.beats_per_bar), 1, DIM);
        fb.text(400, 106, &format!("click {}", if self.click_on { "on" } else { "off" }), 1, if self.click_on { ACCENT } else { DIM });
        let looping = self.project.drums.loop_arrangement;
        fb.text(480, 106, if looping { "drums loop" } else { "drums once" }, 1, if looping { ACCENT } else { DIM });
        let armed = self.armed.iter().filter(|a| **a).count();
        if armed > 0 {
            fb.text(570, 106, &format!("{armed} armed"), 1, WARN);
        }
    }

    fn draw_mixer(&self, fb: &mut Fb) {
        for i in 0..STRIPS {
            let x = STRIP_X0 + i as i32 * STRIP_W;
            let sel = i == self.sel;
            fb.rect(x, 138, STRIP_W - 4, 262, if sel { rgb(34, 42, 58) } else { PANEL });
            if sel {
                fb.frame(x, 138, STRIP_W - 4, 262, 2, ACCENT);
            }
            let (label, lc) = if i == DRUMS { ("DRUM".to_string(), DRUM) } else { (format!("T{}", i + 1), MELODIC) };
            fb.text_center(x + (STRIP_W - 4) / 2, 144, &label, 1, lc);

            // meter + level marker
            let mx = x + 8;
            fb.rect(mx, METER_TOP, 16, METER_H, CELL_OFF);
            let f = meter_frac(self.meter[i]);
            fill_bar(fb, mx, 16, f);
            let gy = METER_TOP + METER_H - (meter_frac(self.strip_gain(i)) * METER_H as f32) as i32;
            fb.rect(mx + 28, METER_TOP, 2, METER_H, CELL_BEAT); // fader rail
            let zero = METER_TOP + METER_H - (meter_frac(1.0) * METER_H as f32) as i32;
            fb.rect(mx - 2, zero, 20, 1, DIM); // 0 dB
            fb.rect(mx + 20, gy - 1, 18, 3, WHITE); // fader position = the strip's level

            let by = METER_TOP + METER_H + 6;
            fb.text_center(x + (STRIP_W - 4) / 2, by, &db_text(self.strip_gain(i)), 1, TEXT);
            fb.text_center(x + (STRIP_W - 4) / 2, by + 12, &pan_text(self.strip_pan(i)), 1, DIM);
            // M / S / R chips
            let chip = |fb: &mut Fb, cx: i32, ch: &str, on: bool, c: u32| {
                fb.rect(cx, by + 28, 14, 14, if on { c } else { CELL_OFF });
                fb.text(cx + 3, by + 31, ch, 1, if on { BG } else { DIM });
            };
            chip(fb, x + 4, "M", self.strip_muted(i), AMBER);
            chip(fb, x + 20, "S", self.strip_solo(i), ACCENT);
            if i < MAX_TRACKS {
                chip(fb, x + 36, "R", self.armed[i], WARN);
            }
        }
        // master
        let mx = STRIP_X0 + STRIPS as i32 * STRIP_W + 2;
        fb.rect(mx, 138, 640 - mx - 10, 262, PANEL);
        fb.text_center(mx + (640 - mx - 10) / 2, 144, "MST", 1, TEXT);
        let w = 640 - mx - 10 - 16;
        fb.rect(mx + 8, METER_TOP, w, METER_H, CELL_OFF);
        fill_bar(fb, mx + 8, w, meter_frac(self.meter_master));
        fb.text_center(mx + (640 - mx - 10) / 2, METER_TOP + METER_H + 6, &format!("{:.0}", gain_to_db(self.meter_master.max(1e-6)).max(-99.0)), 1, DIM);
    }

    fn draw_arrange(&self, fb: &mut Fb) {
        fb.rect(10, 138, 620, 262, PANEL);
        let secs = if self.rate > 0 { self.drum_total as f64 / self.rate as f64 } else { 0.0 };
        let cycles: u32 = self.project.drums.sections.iter().map(|s| s.cycles).sum();
        fb.text(22, 146, "DRUM SECTIONS", 2, DRUM);
        fb.text_right(618, 150, &format!("{} sections, {} cycles, {:.0}s", self.project.drums.sections.len(), cycles, secs), 1, DIM);

        let secs_v = &self.project.drums.sections;
        if secs_v.is_empty() {
            fb.text(22, 190, "No sections. Press A to add one.", 2, DIM);
            return;
        }
        // which section is sounding now
        let playing_idx = if self.transport.playing {
            self.drum_starts.iter().rposition(|s| self.transport.pos >= *s)
        } else {
            None
        };
        const ROWS: usize = 6;
        let first = self.arr_sel.saturating_sub(ROWS - 1).min(secs_v.len().saturating_sub(ROWS));
        for (r, (i, s)) in secs_v.iter().enumerate().skip(first).take(ROWS).enumerate() {
            let y = 172 + r as i32 * 22;
            let sel = i == self.arr_sel;
            if sel {
                fb.rect(16, y - 3, 608, 21, rgb(34, 42, 58));
                fb.frame(16, y - 3, 608, 21, 1, ACCENT);
            }
            let play = playing_idx == Some(i);
            if play {
                fb.rect(18, y + 2, 6, 10, ACCENT);
            }
            let style = presets::builtin().patterns.iter().find(|p| p.id == s.preset).map_or("", |p| p.style.as_str());
            fb.text(30, y, &format!("{:>2}", i + 1), 1, DIM);
            fb.text(54, y, &hill_core::fb::clip(&s.name, 22), 1, if sel { WHITE } else { TEXT });
            fb.text(250, y, &hill_core::fb::clip(style, 12), 1, DIM);
            fb.text(360, y, &format!("x{}", s.cycles), 1, TEXT);
            let kit = self.section_pats.get(i).and_then(|p| p.1.as_deref()).unwrap_or("built-in");
            fb.text(420, y, &hill_core::fb::clip(kit, 25), 1, DIM);
        }

        // mini step grid of the selected section: one row per drum voice, brighter = louder, F = flam
        if let Some((pat, _)) = self.section_pats.get(self.arr_sel) {
            let gy = 312;
            fb.text(22, gy - 12, "PATTERN", 1, DIM);
            let cw = (560 / pat.steps.max(1) as i32).min(34);
            for v in 0..pat.cells.len() {
                let used = pat.cells[v].iter().any(|c| *c != 0);
                for s in 0..pat.steps {
                    let c = pat.cells[v].get(s).copied().unwrap_or(0);
                    let vel = c & 0x7f;
                    let x = 40 + s as i32 * cw + (s as i32 / pat.div.max(1) as i32) * 3;
                    let base = if (s as u32 / pat.div.max(1)).is_multiple_of(2) { CELL_BEAT } else { CELL_OFF };
                    let col = if vel == 0 { base } else if vel >= 127 { WHITE } else { hill_core::fb::scale(DRUM, 0.55 + 0.45 * vel as f32 / 127.0) };
                    fb.rect(x, gy + v as i32 * 10, cw - 2, 8, col);
                    if c & 0x80 != 0 {
                        fb.rect(x, gy + v as i32 * 10, 3, 8, WARN);
                    }
                }
                if used {
                    fb.rect(30, gy + v as i32 * 10 + 2, 4, 4, ACCENT);
                }
            }
        }
    }

    /// FM-1 screen: voice (Program Change) and the effect controls. Unset values show "--": the unit's real state is unknown.
    fn draw_fm1(&self, fb: &mut Fb) {
        fb.rect(10, 138, 620, 262, PANEL);
        let effects = hilldaw_core::fm1::fx_map().effects();
        let mut x = 22;
        let names: Vec<&str> = std::iter::once("VOICE").chain(effects.iter().map(|(e, _)| *e)).collect();
        for (i, n) in names.iter().enumerate() {
            let c = if i == self.fm1_tab { ACCENT } else { DIM };
            x = fb.text(x, 146, &n.to_uppercase(), 1, c) + 14;
        }
        if self.fm1_tab == 0 {
            let f = &self.project.fm1;
            fb.text(22, 190, &format!("{:03}", f.program as u32 + 1), 8, if f.program_set { WHITE } else { DIM });
            let note = if f.program_set { format!("Program Change {} on the note channel", f.program) } else { "not set: the FM-1 keeps its own sound".to_string() };
            fb.text(22, 280, &note, 1, DIM);
            return;
        }
        let ps = &effects[self.fm1_tab - 1].1;
        for (r, p) in ps.iter().enumerate() {
            let y = 180 + r as i32 * 40;
            if r == self.fm1_row {
                fb.rect(16, y - 6, 608, 34, rgb(34, 42, 58));
            }
            fb.text(26, y + 2, &format!("{}{}", p.param, if p.evidence == "order" { " ?" } else { "" }), 2, TEXT);
            fb.text(250, y + 2, &format!("CC {}", p.cc), 1, DIM);
            fb.rect(330, y + 4, 200, 12, CELL_OFF);
            match self.project.fm1.fx.get(&p.cc) {
                Some(v) => {
                    fb.rect(330, y + 4, (200.0 * p.to_fraction(*v)) as i32, 12, DRUM);
                    let t = if p.is_switch() { if *v > 0 { "ON".to_string() } else { "OFF".to_string() } } else { v.to_string() };
                    fb.text(544, y + 2, &t, 2, WHITE);
                }
                None => {
                    fb.text(544, y + 2, "--", 2, DIM);
                }
            }
        }
        fb.text(22, 372, "? = effect could not be measured; position from the vendor guide", 1, DIM);
    }

    /// The sound card's measured clock, once there are 30 s of data: what the drift test reads off the screen.
    fn draw_clock(&self, fb: &mut Fb) {
        if let Some(f) = self.out_clock() {
            fb.text(12, 437, &format!("AUDIO OUT {:.2} Hz {:+.0} ppm  {:.0} s", f.rate_hz, f.ppm, f.span_s), 1, DIM);
        }
    }

    fn draw_footer(&self, fb: &mut Fb) {
        fb.rect(0, 446, 640, 34, PANEL);
        let tab = |fb: &mut Fb, x: i32, name: &str, on: bool| {
            fb.text(x, 454, name, 2, if on { ACCENT } else { DIM });
        };
        tab(fb, 12, "MIXER", self.view == View::Mixer);
        tab(fb, 124, "ARRANGE", self.view == View::Arrange);
        tab(fb, 268, "FM-1", self.view == View::Fm1);
        let hint = match self.view {
            View::Mixer => "L/R strip U/D level X mute A arm",
            View::Arrange => "U/D section L/R pattern A add X del",
            View::Fm1 => "L2/R2 tab U/D row L/R value A on/off",
        };
        fb.text_right(630, 452, hint, 1, DIM);
        fb.text_right(630, 464, "START play   FN+SELECT help", 1, DIM);
    }
}
