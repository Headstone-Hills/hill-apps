//! Drawing. Everything here is `&self`: the UI thread holds the same mutex as the audio callback while it draws.

use super::*;
use hill_core::fb::{rgb, scale, ACCENT, BG, CELL_BEAT, CELL_OFF, DIM, DRUM, MELODIC, PANEL, TEXT, WARN, WHITE};
use hill_core::ui::quit_bar;
use hilldaw_device::{fmt_bytes, fmt_duration, record_seconds_left};

const AMBER: u32 = rgb(255, 200, 60);
const METER_TOP: i32 = 150;
const METER_H: i32 = 170;
const STRIP_W: i32 = 62;
const STRIP_X0: i32 = 12;

/// 0..1 position of a linear level on the shared -60..+24 dB scale (meters and fader marks use the same one, so a fader at +18 dB
/// has somewhere to sit and a 0 dB tick marks unity).
pub(crate) fn meter_frac(level: f32) -> f32 {
    ((gain_to_db(level) + 60.0) / (60.0 + MAX_DB)).clamp(0.0, 1.0)
}

/// Where a dB value sits on that scale.
fn db_frac(db: f32) -> f32 {
    ((db + 60.0) / (60.0 + MAX_DB)).clamp(0.0, 1.0)
}

/// Fill a meter from the bottom up to `frac`, green below -9 dBFS, amber to -1 dBFS, red above.
fn fill_bar(fb: &mut Fb, x: i32, w: i32, frac: f32) {
    let h = (frac * METER_H as f32) as i32;
    // green below -9 dBFS, amber to -1 dBFS, red above
    for (lo, hi, c) in [(0.0f32, db_frac(-9.0), ACCENT), (db_frac(-9.0), db_frac(-1.0), AMBER), (db_frac(-1.0), 1.0, WARN)] {
        let lo = (lo * METER_H as f32) as i32;
        let hi = ((hi * METER_H as f32) as i32).min(h);
        if hi > lo {
            fb.rect(x, METER_TOP + METER_H - hi, w, hi - lo, c);
        }
    }
}

/// Horizontal level bar on a -60..0 dBFS scale with a tick at the held peak: green, amber from -9, red from -1.
fn hbar(fb: &mut Fb, x: i32, y: i32, w: i32, h: i32, level: f32, hold: f32) {
    let f = |l: f32| ((gain_to_db(l) + 60.0) / 60.0).clamp(0.0, 1.0);
    fb.rect(x, y, w, h, CELL_OFF);
    let fill = (f(level) * w as f32) as i32;
    for (lo, hi, c) in [(0.0f32, 51.0 / 60.0, ACCENT), (51.0 / 60.0, 59.0 / 60.0, AMBER), (59.0 / 60.0, 1.0, WARN)] {
        let (a, b) = ((lo * w as f32) as i32, ((hi * w as f32) as i32).min(fill));
        if b > a {
            fb.rect(x + a, y, b - a, h, c);
        }
    }
    if hold > 0.0 {
        fb.rect(x + ((f(hold) * w as f32) as i32).min(w - 2), y - 2, 2, h + 4, WHITE);
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
            View::Export => self.draw_export(fb),
            View::Track => self.draw_track(fb),
            View::Waves => self.draw_waves(fb),
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
        // the audio input that would be recorded from (the FM-1, a mic, ...)
        let (fm, fc) = match self.current_input() {
            Some(i) => (hill_core::fb::clip(&i.name.to_uppercase(), 14), ACCENT),
            None => ("NO INPUT".to_string(), DIM),
        };
        fb.text_right(x, 11, &fm, 1, fc);
        fb.rect(x - Fb::text_w(&fm, 1) - 12, 12, 7, 7, fc);
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
        if self.input_live() {
            // the FM-1 input level, so it can be set before and checked during a take
            fb.text(246, 108, "IN", 1, DIM);
            hbar(fb, 264, 107, 120, 8, self.in_level, self.in_hold);
            if self.clip_ms > 0 {
                fb.text(390, 108, "CLIP", 1, WARN);
            }
        }
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
        let (ms, mc) = if self.status.fm1 { ("FM-1 MIDI: connected", ACCENT) } else { ("FM-1 MIDI: not found", DIM) };
        fb.text(22, 388, ms, 1, mc);
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

    /// Export screen: the options, a progress bar while a job runs, and what the last export wrote.
    fn draw_export(&self, fb: &mut Fb) {
        fb.rect(10, 138, 620, 262, PANEL);
        fb.text(22, 146, "EXPORT", 2, DRUM);
        let e = &self.project.export;
        let onoff = |b: bool| if b { "on" } else { "off" };
        let rows: [(&str, String, &str, bool); 6] = [
            ("Format", FORMATS.iter().find(|f| (f.0, f.1) == (e.format, e.bits)).map_or("?", |f| f.2).to_string(), "", true),
            ("Dither", onoff(e.dither).to_string(), "16-bit only", e.bits == 16),
            ("Normalize", onoff(e.normalize).to_string(), "mixdown peak to -1 dBFS", true),
            ("Stems", onoff(e.stems).to_string(), "one file per track + the drums", true),
            ("Drums", onoff(e.drums).to_string(), "off: leave the drums out of the export", true),
            ("Export now", "press A".to_string(), "", true),
        ];
        for (r, (label, value, note, live)) in rows.iter().enumerate() {
            let y = 166 + r as i32 * 25;
            if r == self.export_row {
                fb.rect(16, y - 4, 608, 24, rgb(34, 42, 58));
            }
            fb.text(26, y, label, 2, if r == 5 { ACCENT } else { TEXT });
            let x = fb.text(250, y, value, 2, if *live { WHITE } else { DIM });
            fb.text(x + 16, y + 4, note, 1, DIM);
        }
        let y = 330;
        if let Some(f) = self.export_progress() {
            fb.frame(22, y, 400, 16, 2, DIM);
            fb.rect(24, y + 2, (396.0 * f) as i32, 12, ACCENT);
            fb.text(434, y + 4, &format!("{:.0}%   B cancels", f * 100.0), 1, TEXT);
            return;
        }
        match self.export_report() {
            Some(Ok(r)) if r.cancelled => {
                fb.text(22, y, "Cancelled. A partly written file may be left in the export folder.", 1, AMBER);
            }
            Some(Ok(r)) => {
                let secs = r.seconds.round() as u64;
                let mut line = format!("Done: {} file{}, {}:{:02}, peak {:.1} dBFS", r.files.len(), if r.files.len() == 1 { "" } else { "s" }, secs / 60, secs % 60, r.peak_db);
                if r.gain_db != 0.0 {
                    line += &format!(", normalized {:+.1} dB", r.gain_db);
                }
                fb.text(22, y, &line, 1, ACCENT);
                if r.clipped > 0 {
                    fb.text(22, y + 12, &format!("{} samples went past full scale and were clipped: lower the levels", r.clipped), 1, WARN);
                }
                if let Some(w) = r.warnings.first() {
                    fb.text(22, y + 12 + if r.clipped > 0 { 12 } else { 0 }, &hill_core::fb::clip(w, 74), 1, AMBER);
                }
                for (i, p) in r.files.iter().take(5).enumerate() {
                    let name = std::path::Path::new(p).file_name().map_or(p.clone(), |n| n.to_string_lossy().into_owned());
                    fb.text(22, y + 28 + i as i32 * 10, &hill_core::fb::clip(&name, 60), 1, DIM);
                }
                if let Some(dir) = r.files.first().and_then(|p| std::path::Path::new(p).parent()) {
                    fb.text(22, y + 28 + 5 * 10, &hill_core::fb::clip(&dir.display().to_string(), 74), 1, DIM);
                }
            }
            Some(Err(e)) => {
                fb.text(22, y, &hill_core::fb::clip(&format!("Export failed: {e}"), 74), 1, WARN);
            }
            None => {
                fb.text(22, y, "Writes the mix of everything on the Mixer and Arrange screens.", 1, DIM);
            }
        }
    }

    fn draw_footer(&self, fb: &mut Fb) {
        fb.rect(0, 446, 640, 34, PANEL);
        // five screens: names at scale 2 on the top line, the hint for this screen and the global keys below
        let tabs = [("MIX", View::Mixer, 12), ("TRACK", View::Track, 68), ("WAVES", View::Waves, 156), ("ARRANGE", View::Arrange, 244), ("FM-1", View::Fm1, 364), ("EXPORT", View::Export, 436)];
        for (name, v, x) in tabs {
            fb.text(x, 449, name, 2, if self.view == v { ACCENT } else { DIM });
        }
        let hint = match self.view {
            View::Mixer => "L/R strip U/D level X mute A arm  Y+B undo Y+A redo",
            View::Track => "L2/R2 tab  U/D row  L/R value (Y: big)  B mixer",
            View::Arrange => "U/D section  L/R pattern  L2/R2 style  A add  X del",
            View::Fm1 => "L2/R2 section  U/D row  L/R value  A on/off  X note",
            View::Export => "U/D option  L/R change  A export  B cancel",
            View::Waves => "L/R scroll  L2/R2 zoom  U/D row  A seek  X follow",
        };
        fb.text(12, 469, hint, 1, DIM);
        fb.text_right(630, 453, "FN+SEL help", 1, DIM);
        fb.text_right(630, 469, "START play  SELECT rec", 1, DIM);
    }

    /// Waves screen: every track's recorded audio over time, the drum arrangement, the playhead and the loop / punch regions.
    /// Tracks with audio have a bright label and their waveform; empty ones are dim and say so.
    fn draw_waves(&self, fb: &mut Fb) {
        use hilldaw_core::waveform::{envelope, frame_to_x};
        const X0: i32 = 62;
        const W: i32 = 560;
        const ROW_Y: i32 = 158;
        const ROW_H: i32 = 26;
        fb.rect(10, 138, 620, 262, PANEL);
        let bar = self.bar_frames();
        let span = self.waves_span();
        let start = self.waves_start;
        let bars_shown = ZOOM_BARS[self.waves_zoom.min(ZOOM_BARS.len() - 1)];
        let px_per_bar = W as f64 / bars_shown as f64;
        let x_of = |frame: u64| X0 + ((frame.saturating_sub(start)).min(span) as f64 / span as f64 * W as f64) as i32;

        // ruler: bar numbers (spaced so they never touch), a grid line every bar (every 4 when zoomed far out)
        fb.rect(X0, 140, W, 14, rgb(20, 24, 32));
        let every = [1u64, 2, 4, 8, 16].iter().copied().find(|e| px_per_bar * *e as f64 >= 34.0).unwrap_or(16);
        let grid_every = if px_per_bar >= 7.0 { 1 } else { 4 };
        let first_bar = start / bar;
        for k in 0..bars_shown {
            let b = first_bar + k;
            let x = X0 + (k as f64 * px_per_bar) as i32;
            if b.is_multiple_of(grid_every) {
                fb.rect(x, ROW_Y, 1, 9 * ROW_H, if b.is_multiple_of(4) { CELL_BEAT } else { CELL_OFF });
            }
            if b.is_multiple_of(every) {
                fb.rect(x, 146, 1, 8, DIM);
                fb.text(x + 5, 142, &format!("{}", b + 1), 1, DIM);
            }
        }
        // loop region (green) and punch region (red) as bands on the ruler
        for (region, col, y) in [(self.project.loop_region, scale(ACCENT, 0.7), 140), (self.project.punch, scale(WARN, 0.8), 151)] {
            if let Some((a, e)) = region {
                if e > start && a < start + span {
                    let (xa, xb) = (x_of(a.max(start)), x_of(e.min(start + span)));
                    fb.rect(xa, y, (xb - xa).max(2), 3, col);
                }
            }
        }
        fb.text(12, 142, &format!("{bars_shown} bars"), 1, DIM);
        if self.waves_follow {
            fb.text(12, 150, "follow", 1, scale(ACCENT, 0.7));
        }

        // one row per strip: 8 tracks and the drums
        let any_solo = (0..STRIPS).any(|i| self.strip_solo(i));
        for i in 0..STRIPS {
            let y = ROW_Y + i as i32 * ROW_H;
            let cy = y + ROW_H / 2;
            let (name, col) = if i == DRUMS { ("DRUM".to_string(), DRUM) } else { (format!("T{}", i + 1), MELODIC) };
            let used = if i == DRUMS { !self.project.drums.sections.is_empty() } else { !self.project.tracks[i].clips.is_empty() };
            let silent = self.strip_muted(i) || (any_solo && !self.strip_solo(i));
            let tone = if silent { rgb(96, 102, 116) } else { col };
            fb.rect(X0, y + 1, W, ROW_H - 2, rgb(22, 27, 36));
            fb.text(14, cy - 4, &name, 1, if used { tone } else { DIM });
            if self.strip_muted(i) {
                fb.text(36, cy - 4, "M", 1, AMBER);
            }
            if self.strip_solo(i) {
                fb.text(46, cy - 4, "S", 1, ACCENT);
            }
            if !used {
                fb.rect(X0, cy, W, 1, CELL_OFF);
                fb.text(X0 + 6, cy - 4, if i == DRUMS { "no drum sections" } else { "empty" }, 1, CELL_BEAT);
            } else if i == DRUMS {
                // the arrangement as blocks, then the same blocks lighter for as long as the drums repeat
                let d = &self.project.drums;
                let period = self.drum_total.max(1);
                let n = d.sections.len();
                let mut k = 0u64;
                while k * period < start + span && (k == 0 || d.loop_arrangement) {
                    for (si, sec) in d.sections.iter().enumerate() {
                        let s0 = k * period + self.drum_starts.get(si).copied().unwrap_or(0);
                        let len = self.drum_starts.get(si + 1).copied().unwrap_or(self.drum_starts.first().copied().unwrap_or(0) + period) - self.drum_starts.get(si).copied().unwrap_or(0);
                        let s1 = s0 + len;
                        if s1 <= start || s0 >= start + span {
                            continue;
                        }
                        let (xa, xb) = (x_of(s0.max(start)), x_of(s1.min(start + span)));
                        let shade = if k == 0 { 0.55 } else { 0.3 };
                        fb.rect(xa, y + 3, (xb - xa - 1).max(1), ROW_H - 6, if silent { rgb(52, 56, 66) } else { scale(DRUM, shade) });
                        if xb - xa > 44 && k == 0 {
                            fb.text(xa + 3, cy - 4, &hill_core::fb::clip(&sec.name, ((xb - xa) / 8 - 1).max(1) as usize), 1, if n > 0 { WHITE } else { TEXT });
                        }
                    }
                    k += 1;
                }
            } else {
                let g = self.strip_gain(i);
                let fpp = span as f64 / W as f64;
                for (ci, c) in self.project.tracks[i].clips.iter().enumerate() {
                    let len = c.len.unwrap_or(0);
                    let (cs, ce) = (c.start, c.start + len);
                    if ce <= start || cs >= start + span {
                        continue;
                    }
                    let (xa, xb) = (x_of(cs.max(start)), x_of(ce.min(start + span)));
                    if xb <= xa {
                        continue;
                    }
                    let active = self.project.tracks[i].active_take.is_some() || ci == 0;
                    fb.rect(xa, y + 2, xb - xa, ROW_H - 4, if silent { rgb(44, 48, 58) } else { scale(col, if active { 0.28 } else { 0.15 }) });
                    if let Some(pk) = self.peak_cache.get(&c.file) {
                        let from = c.offset as i64 + (start.max(cs) as i64 - cs as i64);
                        let env = envelope(pk, from, fpp, (xb - xa) as usize);
                        let half = (ROW_H / 2 - 3) as f32;
                        for (k, e) in env.iter().enumerate() {
                            if let Some((lo, hi)) = e {
                                let (lo, hi) = ((lo * g).clamp(-1.0, 1.0), (hi * g).clamp(-1.0, 1.0));
                                let (top, bot) = (cy - (hi * half).round() as i32, cy - (lo * half).round() as i32);
                                fb.rect(xa + k as i32, top, 1, (bot - top).max(1), tone);
                            }
                        }
                    }
                }
            }
            if i == self.sel {
                fb.frame(10, y, 616, ROW_H, 1, ACCENT);
            }
        }

        // the playhead (white; red while recording)
        if let Some(x) = frame_to_x(self.transport.pos, start, span, X0, W) {
            fb.rect(x, 140, 2, 9 * ROW_H + 18, if self.rec.is_some() { WARN } else { WHITE });
        }
    }

    /// Track screen: takes, effects, input and loop/punch for the strip picked on the Mixer.
    fn draw_track(&self, fb: &mut Fb) {
        fb.rect(10, 138, 620, 262, PANEL);
        let i = self.sel;
        let (name, col) = if i == DRUMS { ("DRUMS".to_string(), DRUM) } else { (self.project.tracks[i].name.clone(), MELODIC) };
        fb.text(22, 144, &hill_core::fb::clip(&name, 12), 2, col);
        let mut x = 300;
        for (t, label) in [(TrackTab::Takes, "TAKES"), (TrackTab::Fx, "FX"), (TrackTab::Input, "INPUT"), (TrackTab::Loop, "LOOP")] {
            x = fb.text(x, 150, label, 1, if t == self.track_tab { ACCENT } else { DIM }) + 14;
        }
        let row_y = |r: usize, step: i32| 180 + r as i32 * step;
        let sel_bar = |fb: &mut Fb, y: i32, h: i32| fb.rect(16, y - 4, 608, h, rgb(34, 42, 58));
        match self.track_tab {
            TrackTab::Takes => {
                if i == DRUMS {
                    fb.text(22, 190, "The drum track has no takes.", 1, DIM);
                    return;
                }
                let tm = &self.project.tracks[i];
                if tm.takes.is_empty() {
                    fb.text(22, 190, "No takes yet. Arm this track on the Mixer (A) and press SELECT.", 1, DIM);
                    return;
                }
                let bar = self.bar_frames();
                const ROWS: usize = 9;
                let first = self.take_sel.saturating_sub(ROWS - 1).min(tm.takes.len().saturating_sub(ROWS));
                for (r, (n, t)) in tm.takes.iter().enumerate().skip(first).take(ROWS).enumerate() {
                    let y = row_y(r, 24);
                    if n == self.take_sel {
                        sel_bar(fb, y, 22);
                    }
                    let secs = t.frames / 44100;
                    let playing = tm.active_take == Some(n);
                    fb.text(26, y + 3, &format!("Take {}", n + 1), 1, if playing { ACCENT } else { TEXT });
                    fb.text(110, y + 3, &format!("{}:{:02}", secs / 60, secs % 60), 1, TEXT);
                    fb.text(170, y + 3, &if t.placed { format!("from bar {}", t.start / bar + 1) } else { "older take (no place saved)".to_string() }, 1, DIM);
                    fb.text(330, y + 3, &hill_core::fb::clip(&t.file, 30), 1, DIM);
                    if playing {
                        fb.text_right(618, y + 3, "PLAYING", 1, ACCENT);
                    }
                }
                fb.text(22, 386, "A: play this take   X twice: remove it (audio stays on the card)", 1, DIM);
            }
            TrackTab::Fx => {
                let f = self.strip_fx(i);
                let pct = |v: f32| if v <= 0.0 { "off".to_string() } else { format!("{:.0}%", v * 100.0) };
                let hz = |v: u32| if v == 0 { "off".to_string() } else { format!("{v} Hz") };
                let rows: [(&str, String, bool); 7] = [
                    ("Low-pass", hz(f.lowpass_hz), f.lowpass_hz > 0),
                    ("High-pass", hz(f.highpass_hz), f.highpass_hz > 0),
                    ("Delay time", format!("{} ms", f.delay_ms), f.delay_mix > 0.0),
                    ("Delay feedback", pct(f.delay_feedback), f.delay_mix > 0.0),
                    ("Delay mix", pct(f.delay_mix), f.delay_mix > 0.0),
                    ("Reverb send", pct(f.reverb_send), f.reverb_send > 0.0),
                    ("Reverb room", format!("{:.0}%", self.project.reverb_room * 100.0), f.reverb_send > 0.0),
                ];
                for (r, (label, value, live)) in rows.iter().enumerate() {
                    let y = row_y(r, 28) - 6;
                    if r == self.track_row {
                        sel_bar(fb, y, 26);
                    }
                    fb.text(26, y + 2, label, 2, TEXT);
                    fb.text(300, y + 2, value, 2, if *live { WHITE } else { DIM });
                }
                fb.text(22, 386, "Reverb room is shared by every track. Delay needs its mix above off.", 1, DIM);
            }
            TrackTab::Input => {
                fb.text(26, 168, "Input level", 2, TEXT);
                hbar(fb, 26, 192, 400, 16, self.in_level, self.in_hold);
                for db in [-48.0f32, -24.0, -12.0, -6.0, 0.0] {
                    let x = 26 + ((db + 60.0) / 60.0 * 400.0) as i32;
                    fb.rect(x, 210, 1, 5, DIM);
                    fb.text(x - 12, 218, &format!("{db:.0}"), 1, DIM);
                }
                if self.clip_ms > 0 {
                    fb.text(440, 194, "CLIP", 2, WARN);
                }
                if let Some(e) = &self.monitor_err {
                    fb.text(26, 230, &hill_core::fb::clip(&format!("No input: {e}"), 70), 1, WARN);
                } else if !self.input_live() {
                    fb.text(26, 230, "Waiting for the input ...", 1, DIM);
                }
                // row 0: which input; its format on the line below
                let device = match (&self.settings.input_device, self.current_input()) {
                    (None, Some(i)) => format!("Auto: {}", i.name),
                    (None, None) => "Auto: none found".to_string(),
                    (Some(_), Some(i)) => i.name.clone(),
                    (Some(id), None) => format!("{id} (not connected)"),
                };
                let ms = self.project.latency_offset_frames as f64 / 44.1;
                let rows = [("Input", hill_core::fb::clip(&device, 22)), ("Latency offset", format!("{ms:.0} ms")), ("Count-in", (if self.project.count_in { "on" } else { "off" }).to_string())];
                let ys = [244, 280, 308];
                for (r, (label, value)) in rows.iter().enumerate() {
                    let y = ys[r];
                    if r == self.track_row {
                        sel_bar(fb, y, 26);
                    }
                    fb.text(26, y + 2, label, 2, TEXT);
                    fb.text(300, y + 2, value, 2, if r == 0 && self.current_input().is_none() { DIM } else { WHITE });
                }
                if let Some(i) = self.current_input() {
                    let note = if i.rate == 44100 { i.summary.clone() } else { format!("{}  (resampled to 44.1 kHz)", i.summary) };
                    fb.text(300, 266, &hill_core::fb::clip(&note, 42), 1, DIM);
                } else {
                    fb.text(300, 266, "plug in a USB audio input (mic, interface, FM-1)", 1, DIM);
                }
                fb.text(22, 346, "The meter runs while a track is armed or this tab is open.", 1, DIM);
                fb.text(22, 358, "Play your loudest part; set the input volume so peaks reach about -6.", 1, DIM);
                fb.text(22, 370, "Raising the latency offset moves takes earlier (use it if takes sit late).", 1, DIM);
                fb.text(22, 382, "Input: Left/Right picks Auto or a device. USB Audio 2.0 is not supported.", 1, DIM);
            }
            TrackTab::Loop => {
                let bar = self.bar_frames();
                let show = |r: Option<(u64, u64)>| match r {
                    Some((s, e)) => (format!("bar {}", s / bar + 1), format!("{} bar{}", ((e - s) / bar).max(1), if (e - s) / bar == 1 { "" } else { "s" }), true),
                    None => ("-".to_string(), "-".to_string(), false),
                };
                let (ls, ll, lon) = show(self.project.loop_region);
                let (ps, pl, pon) = show(self.project.punch);
                let rows: [(&str, String, bool); 6] = [
                    ("Loop", (if lon { "on" } else { "off" }).to_string(), true),
                    ("   start", ls, lon),
                    ("   length", ll, lon),
                    ("Punch in/out", (if pon { "on" } else { "off" }).to_string(), true),
                    ("   from", ps, pon),
                    ("   length", pl, pon),
                ];
                for (r, (label, value, live)) in rows.iter().enumerate() {
                    let y = row_y(r, 28) - 6;
                    if r == self.track_row {
                        sel_bar(fb, y, 26);
                    }
                    fb.text(26, y + 2, label, 2, TEXT);
                    fb.text(300, y + 2, value, 2, if *live { WHITE } else { DIM });
                }
                fb.text(22, 374, "Loop: playback goes round the region; recording makes a take per pass.", 1, DIM);
                fb.text(22, 386, "Punch: only the region is recorded. A sets a row to the playhead's bar.", 1, DIM);
            }
        }
    }
}
