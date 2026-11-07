//! Mixdown and stem export through the app, with a fake store that keeps the files in memory.

use hill_core::fb::Fb;
use hill_core::{App, Btn, MemHost};
use hilldaw_app::recorder::{ExportFile, RecInfo, Recorder};
use hilldaw_app::status::{Status, StatusHandle};
use hilldaw_app::{HillDaw, View};
use hilldaw_core::wav::decode;
use std::collections::BTreeMap;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

const R: f64 = 44100.0;
const SLOPE: f32 = 1e-5;
const PAN_CENTRE: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// A file whose bytes can be read back after the export thread is done with it.
#[derive(Clone, Default)]
struct MemFile(Arc<Mutex<Cursor<Vec<u8>>>>);

impl Write for MemFile {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for MemFile {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.0.lock().unwrap().seek(p)
    }
}

#[derive(Default)]
struct State {
    started: usize,
    /// recording length (frames) by file, for `load`
    files: BTreeMap<String, u64>,
    /// exported files by the path shown to the user
    exports: BTreeMap<String, MemFile>,
    fail_export: bool,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<State>>);

impl Recorder for Fake {
    fn start(&mut self) -> Result<String, String> {
        let mut s = self.0.lock().unwrap();
        s.started += 1;
        Ok(format!("audio/rec-{:03}.wav", s.started))
    }
    fn frames(&self) -> u64 {
        0
    }
    fn finish(&mut self, min_frames: u64) -> Result<RecInfo, String> {
        let mut s = self.0.lock().unwrap();
        let n = format!("audio/rec-{:03}.wav", s.started);
        s.files.insert(n, min_frames + 500);
        Ok(RecInfo { frames: min_frames + 500, overruns: 0 })
    }
    fn load(&mut self, rel: &str) -> Option<Vec<f32>> {
        let n = *self.0.lock().unwrap().files.get(rel)?;
        Some((0..n).map(|i| i as f32 * SLOPE).collect())
    }
    fn create_export(&mut self, stem: &str) -> Result<(Box<dyn ExportFile>, String), String> {
        let mut s = self.0.lock().unwrap();
        if s.fail_export {
            return Err("card is read-only".into());
        }
        let mut path = format!("/card/export/{stem}.wav");
        let mut n = 2;
        while s.exports.contains_key(&path) {
            path = format!("/card/export/{stem}-{n}.wav");
            n += 1;
        }
        let f = MemFile::default();
        s.exports.insert(path.clone(), f.clone());
        Ok((Box::new(f), path))
    }
}

impl Fake {
    fn file(&self, suffix: &str) -> Vec<u8> {
        let s = self.0.lock().unwrap();
        let (_, f) = s.exports.iter().find(|(p, _)| p.ends_with(suffix)).unwrap_or_else(|| panic!("no export {suffix}: {:?}", s.exports.keys().collect::<Vec<_>>()));
        let b = f.0.lock().unwrap().get_ref().clone();
        b
    }
    fn names(&self) -> Vec<String> {
        self.0.lock().unwrap().exports.keys().map(|k| k.rsplit('/').next().unwrap().to_string()).collect()
    }
}

fn status() -> StatusHandle {
    Arc::new(Mutex::new(Status::default()))
}

fn press(a: &mut HillDaw, b: Btn) {
    a.event(b, true);
    a.event(b, false);
}

fn new_app() -> (HillDaw, Fake) {
    let fake = Fake::default();
    let a = HillDaw::new(Box::new(MemHost::default()), status()).with_recorder(Box::new(fake.clone()));
    (a, fake)
}

/// Run the UI until the export job has finished.
fn wait_export(a: &mut HillDaw) {
    for _ in 0..4000 {
        a.tick(16);
        if !a.export_running() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the export did not finish");
}

/// Record `secs` seconds on track 1 (count-in off) with the clock at 0, so the clip starts at 0 with offset 563.
fn record_take(a: &mut HillDaw, secs: f64) {
    press(a, Btn::A); // arm track 1
    a.event(Btn::Fn, true);
    press(a, Btn::L1); // count-in off
    a.event(Btn::Fn, false);
    a.set_test_clock(Some(0.0));
    press(a, Btn::Select);
    let (mut done, mut buf) = (0u64, vec![0.0f32; 2048]);
    while (done as f64) < secs * R {
        a.audio_at(&mut buf, done as f64 / R);
        done += 1024;
    }
    press(a, Btn::Select);
    assert_eq!(a.project.tracks[0].clips.len(), 1);
}

/// Turn the drum strip down `db` dB (and come back to track 1), so the mix stays clear of the master limiter: the starter groove alone
/// peaks above its ceiling, and a limited mixdown is not the sum of its stems.
fn drums_down(a: &mut HillDaw, db: usize) {
    for _ in 0..8 {
        press(a, Btn::Right);
    }
    for _ in 0..db {
        press(a, Btn::Down);
    }
    for _ in 0..8 {
        press(a, Btn::Left);
    }
    assert_eq!(a.sel(), 0);
}

fn remove_drums(a: &mut HillDaw) {
    press(a, Btn::R1);
    press(a, Btn::X);
    press(a, Btn::L1);
    assert!(a.project.drums.sections.is_empty());
}

fn export_now(a: &mut HillDaw) {
    press(a, Btn::R1);
    press(a, Btn::R1);
    press(a, Btn::R1);
    assert_eq!(a.view(), View::Export);
    press(a, Btn::A);
    wait_export(a);
}

#[test]
fn nothing_to_export_without_storage_or_without_anything_on_the_timeline() {
    let mut a = HillDaw::new(Box::new(MemHost::default()), status());
    a.start_export();
    assert!(a.toast_text().contains("storage"), "{}", a.toast_text());
    let (mut a, fake) = new_app();
    remove_drums(&mut a);
    a.start_export();
    assert!(a.toast_text().contains("Nothing to export") && !a.export_running());
    assert!(fake.names().is_empty());
    // a card that refuses the file is reported, and nothing is left running
    let (mut a, fake) = new_app();
    fake.0.lock().unwrap().fail_export = true;
    a.start_export();
    assert!(a.toast_text().contains("read-only") && !a.export_running(), "{}", a.toast_text());
}

#[test]
fn the_exported_drums_are_the_same_audio_as_what_plays_live_plus_a_tail() {
    let (mut a, fake) = new_app();
    export_now(&mut a);
    let wav = decode(&fake.file("song-mixdown.wav")).expect("a valid WAV");
    let len = a.drum_frames() as usize;
    assert_eq!(wav.rate, 44100);
    assert_eq!(wav.frames(), len + 88_200, "the song plus a 2 s tail");
    // the same frames played live
    let mut live = Vec::new();
    let mut buf = vec![0.0f32; 2048];
    press(&mut a, Btn::L1);
    press(&mut a, Btn::L1);
    press(&mut a, Btn::L1);
    press(&mut a, Btn::Start);
    for _ in 0..(3 * 44100 / 1024) {
        a.audio(&mut buf);
        live.extend_from_slice(&buf);
    }
    let worst = live.iter().zip(&wav.data).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
    assert!(worst < 1e-5, "24-bit export vs live render differ by {worst}");
    assert!(live.iter().any(|x| x.abs() > 0.05), "the comparison is not on silence");
    // the tail: the last hits ring out, then it is silent, and the drums did not start again after the end
    let tail = &wav.data[2 * len + 2 * 80_000..];
    assert!(tail.iter().all(|x| x.abs() < 1e-3), "the drums did not restart in the tail");
}

#[test]
fn a_recorded_take_is_in_the_mixdown_at_the_right_place_and_stems_add_up_to_it() {
    let (mut a, fake) = new_app();
    record_take(&mut a, 2.0);
    // the take is a loud ramp: 24 dB down on it and 12 dB on the drums keeps the sum clear of the limiter
    for _ in 0..24 {
        press(&mut a, Btn::Down);
    }
    drums_down(&mut a, 12);
    a.project.export.stems = true;
    export_now(&mut a);
    assert_eq!(fake.names(), vec!["song-mixdown.wav", "song-stem-1-track-1.wav", "song-stem-drums.wav"]);
    let mix = decode(&fake.file("song-mixdown.wav")).unwrap();
    let t1 = decode(&fake.file("track-1.wav")).unwrap();
    let dr = decode(&fake.file("drums.wav")).unwrap();
    assert_eq!((mix.frames(), t1.frames(), dr.frames()), (mix.frames(), mix.frames(), mix.frames()), "all files are the same length");
    // the stem of the take: file frame 563 + p, through -24 dB and the centre pan law
    let g = 10f32.powf(-24.0 / 20.0);
    for p in [0usize, 1000, 40_000] {
        let want = (563 + p) as f32 * SLOPE * g * PAN_CENTRE;
        assert!((t1.data[2 * p] - want).abs() < 2e-6, "stem frame {p}: {} vs {want}", t1.data[2 * p]);
    }
    // the stems add up to the mixdown while the limiter is not working
    let peak = mix.data.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak < 0.9, "the mix stays under the limiter ({peak})");
    let worst = (0..mix.data.len()).fold(0.0f32, |m, i| m.max((mix.data[i] - (t1.data[i] + dr.data[i])).abs()));
    assert!(worst < 3e-6, "stems vs mixdown differ by {worst}");
}

#[test]
fn mute_and_solo_shape_the_mixdown_but_not_the_stems() {
    let (mut a, fake) = new_app();
    record_take(&mut a, 1.0);
    drums_down(&mut a, 12);
    a.project.export.stems = true;
    press(&mut a, Btn::X); // mute track 1
    export_now(&mut a);
    let mix = decode(&fake.file("song-mixdown.wav")).unwrap();
    let stem = decode(&fake.file("track-1.wav")).unwrap();
    let drums = decode(&fake.file("drums.wav")).unwrap();
    let diff = (0..2 * 20_000).fold(0.0f32, |m, i| m.max((mix.data[i] - drums.data[i]).abs()));
    assert!(diff < 3e-6, "the muted track is not in the mixdown ({diff})");
    assert!(stem.data[2 * 1000] > 0.0, "but its stem is still written, un-muted");
}

#[test]
fn normalize_brings_the_peak_to_minus_one_dbfs_and_the_stems_get_the_same_gain() {
    let (mut a, fake) = new_app();
    // a quiet song: drums 30 dB down
    press(&mut a, Btn::Right);
    for _ in 0..8 {
        press(&mut a, Btn::Right);
    }
    assert_eq!(a.sel(), 8);
    for _ in 0..30 {
        press(&mut a, Btn::Down);
    }
    a.project.export.normalize = true;
    a.project.export.stems = true;
    export_now(&mut a);
    let mix = decode(&fake.file("song-mixdown.wav")).unwrap();
    let peak = mix.data.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!((20.0 * peak.log10() + 1.0).abs() < 0.02, "peak {:.2} dBFS", 20.0 * peak.log10());
    let rep = a.export_report().unwrap().as_ref().unwrap();
    assert!(rep.gain_db > 20.0 && (rep.peak_db + 1.0).abs() < 0.02, "{rep:?}");
    let dr = decode(&fake.file("drums.wav")).unwrap();
    let worst = (0..mix.data.len()).fold(0.0f32, |m, i| m.max((mix.data[i] - dr.data[i]).abs()));
    assert!(worst < 2e-5, "a lone stem equals the normalized mix: {worst}");
}

#[test]
fn sixteen_bit_export_is_dithered_unless_turned_off_and_reads_back_close_to_24_bit() {
    let (mut a, fake) = new_app();
    export_now(&mut a); // 24-bit reference
    press(&mut a, Btn::Down); // row 1 = dither...
    press(&mut a, Btn::Up); // ...back to row 0 = format
    press(&mut a, Btn::Right); // 24 -> 16
    assert_eq!(a.project.export.bits, 16);
    press(&mut a, Btn::A);
    wait_export(&mut a);
    a.project.export.dither = false;
    press(&mut a, Btn::A);
    wait_export(&mut a);
    let names = fake.names();
    assert_eq!(names.len(), 3, "{names:?}");
    let ref24 = decode(&fake.file("song-mixdown.wav")).unwrap();
    assert_eq!(fake.file("song-mixdown-2.wav")[34], 16, "bits per sample in the header");
    // compare the raw 16-bit integers with what the 24-bit float says they should be
    let ints = |name: &str| -> Vec<i32> { fake.file(name)[44..].chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as i32).collect() };
    let (d_ints, n_ints) = (ints("song-mixdown-2.wav"), ints("song-mixdown-3.wav"));
    let expect: Vec<i32> = ref24.data.iter().map(|x| (x * 32767.0).round() as i32).collect();
    assert_eq!(d_ints.len(), expect.len());
    let off = |a: &[i32]| a.iter().zip(&expect).map(|(x, y)| (x - y).abs()).max().unwrap();
    assert!(off(&n_ints) <= 1, "undithered 16-bit is the rounded 24-bit value: off by {}", off(&n_ints));
    assert!(off(&d_ints) <= 2, "dithered 16-bit is within 2 LSB: off by {}", off(&d_ints));
    let changed = d_ints.iter().zip(&n_ints).filter(|(a, b)| a != b).count();
    assert!(changed > d_ints.len() / 4, "dither changed many samples ({changed} of {})", d_ints.len());
}

#[test]
fn an_export_can_be_cancelled_and_reports_it() {
    let (mut a, _fake) = new_app();
    // a very long song: 256 cycles of the starter groove (about 10 minutes)
    press(&mut a, Btn::R1);
    a.event(Btn::Y, true);
    for _ in 0..300 {
        press(&mut a, Btn::Right);
    }
    a.event(Btn::Y, false);
    press(&mut a, Btn::R1);
    press(&mut a, Btn::R1);
    assert_eq!(a.view(), View::Export);
    press(&mut a, Btn::A);
    assert!(a.export_running());
    press(&mut a, Btn::B);
    wait_export(&mut a);
    let rep = a.export_report().unwrap().as_ref().unwrap();
    assert!(rep.cancelled, "{rep:?}");
    assert!(a.toast_text().contains("cancelled"), "{}", a.toast_text());
}

#[test]
fn the_export_screen_shows_options_progress_and_results() {
    let (mut a, _fake) = new_app();
    export_now(&mut a);
    let mut fb = Fb::new();
    a.render(&mut fb);
    let lit = fb.px.iter().filter(|p| **p != hill_core::fb::BG).count();
    assert!(lit > 15_000);
    // option changes are kept in the project
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Right); // normalize on
    press(&mut a, Btn::Down);
    press(&mut a, Btn::Right); // stems on
    assert!(a.project.export.normalize && a.project.export.stems);
    // a second export while one runs is refused
    press(&mut a, Btn::A);
    a.start_export();
    assert!(a.toast_text().contains("already running"), "{}", a.toast_text());
    wait_export(&mut a);
    a.render(&mut fb);
}
