//! Several songs (the browser) and recordings that never made it into a take (recovery).

use hill_core::fb::Fb;
use hill_core::{App, Btn, Host, MemHost};
use hilldaw_app::recorder::{RecInfo, Recorder};
use hilldaw_app::status::{Status, StatusHandle};
use hilldaw_app::{HillDaw, TrackTab, View};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    /// per song folder: (file, length in project frames)
    files: BTreeMap<String, Vec<(String, u64)>>,
    project: String,
    switches: Vec<String>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<State>>);

impl Fake {
    fn add(&self, song: &str, file: &str, frames: u64) {
        self.0.lock().unwrap().files.entry(song.into()).or_default().push((file.into(), frames));
    }
}

impl Recorder for Fake {
    fn start(&mut self) -> Result<String, String> {
        Ok("audio/rec-900.wav".into())
    }
    fn frames(&self) -> u64 {
        0
    }
    fn finish(&mut self, min_frames: u64) -> Result<RecInfo, String> {
        Ok(RecInfo { frames: min_frames + 500, overruns: 0 })
    }
    fn load(&mut self, rel: &str) -> Option<Vec<f32>> {
        let s = self.0.lock().unwrap();
        let len = s.files.get(&s.project)?.iter().find(|(f, _)| f == rel)?.1;
        Some((0..len).map(|i| i as f32 * 1e-6).collect())
    }
    fn set_project(&mut self, slug: &str) {
        let mut s = self.0.lock().unwrap();
        s.project = slug.into();
        s.switches.push(slug.into());
    }
    fn list_recordings(&mut self) -> Vec<(String, u64)> {
        let s = self.0.lock().unwrap();
        s.files.get(&s.project).cloned().unwrap_or_default()
    }
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<MemHost>>);

impl Host for Shared {
    fn list_samples(&self) -> Vec<String> {
        self.0.lock().unwrap().list_samples()
    }
    fn read_sample(&self, n: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().read_sample(n)
    }
    fn load_state(&self, a: &str) -> Option<String> {
        self.0.lock().unwrap().load_state(a)
    }
    fn save_state(&mut self, a: &str, d: &str) {
        self.0.lock().unwrap().save_state(a, d)
    }
}

fn press(a: &mut HillDaw, b: Btn) {
    a.event(b, true);
    a.event(b, false);
}

fn with(a: &mut HillDaw, hold: Btn, b: Btn) {
    a.event(hold, true);
    press(a, b);
    a.event(hold, false);
}

fn app(host: &Shared, fake: &Fake) -> HillDaw {
    let st: StatusHandle = Arc::new(Mutex::new(Status::default()));
    HillDaw::new(Box::new(host.clone()), st).with_recorder(Box::new(fake.clone()))
}

fn to_takes(a: &mut HillDaw) {
    for _ in 0..8 {
        if a.view() == View::Track {
            break;
        }
        press(a, Btn::R1);
    }
    while a.track_tab() != TrackTab::Takes {
        press(a, Btn::L2);
    }
}

// ---- recovery ----

#[test]
fn recordings_that_no_take_names_are_found_and_put_back_oldest_first_with_undo() {
    let (host, fake) = (Shared::default(), Fake::default());
    fake.add("song", "audio/rec-001.wav", 100_000);
    fake.add("song", "audio/rec-002.wav", 50_000);
    fake.add("song", "audio/rec-003.wav", 500); // shorter than the latency offset: nothing left after it
    let mut a = app(&host, &fake);
    assert_eq!(a.orphan_count(), 3, "all three are in no take");
    assert!(a.toast_text().contains("unused"), "{}", a.toast_text());
    to_takes(&mut a);
    with(&mut a, Btn::Y, Btn::X);
    let t = &a.project.tracks[0];
    assert_eq!((t.takes.len(), t.active_take), (1, Some(0)));
    let k = &t.takes[0];
    assert_eq!((k.file.as_str(), k.start, k.offset, k.frames, k.placed), ("audio/rec-001.wav", 0, 563, 100_000 - 563, true));
    assert_eq!(t.clips[0].file, "audio/rec-001.wav", "and it plays");
    assert!(a.toast_text().contains("Recovered rec-001.wav"), "{}", a.toast_text());
    assert_eq!(a.orphan_count(), 2);
    with(&mut a, Btn::Y, Btn::X);
    assert_eq!(a.project.tracks[0].takes.len(), 2);
    assert_eq!(a.project.tracks[0].takes[1].file, "audio/rec-002.wav");
    // the short one is refused and dropped from the list, so it cannot be offered forever
    with(&mut a, Btn::Y, Btn::X);
    assert_eq!(a.project.tracks[0].takes.len(), 2);
    assert!(a.toast_text().contains("too short"), "{}", a.toast_text());
    assert_eq!(a.orphan_count(), 0);
    with(&mut a, Btn::Y, Btn::X);
    assert!(a.toast_text().contains("No unused"), "{}", a.toast_text());
    // undo takes the second recovery back out
    with(&mut a, Btn::Y, Btn::B);
    assert_eq!(a.project.tracks[0].takes.len(), 1);
    let mut fb = Fb::new();
    a.render(&mut fb);
}

#[test]
fn a_recording_that_is_in_a_take_is_not_an_orphan_and_the_hint_is_drawn_only_when_there_is_one() {
    let (host, fake) = (Shared::default(), Fake::default());
    fake.add("song", "audio/rec-001.wav", 100_000);
    let mut a = app(&host, &fake);
    assert_eq!(a.orphan_count(), 1);
    to_takes(&mut a);
    let blank = |a: &HillDaw| {
        let mut fb = Fb::new();
        a.render(&mut fb);
        fb.px.iter().filter(|p| **p == hill_core::fb::WARN).count()
    };
    assert!(blank(&a) > 50, "the hint is drawn in the warning colour");
    with(&mut a, Btn::Y, Btn::X);
    assert_eq!(a.orphan_count(), 0);
    assert_eq!(blank(&a), 0, "no hint once nothing is left");
    // a restart sees the take in the project, so the file is not offered again
    let b = app(&host, &fake);
    assert_eq!(b.orphan_count(), 0);
}

// ---- songs ----

#[test]
fn the_browser_makes_new_songs_switches_between_them_and_keeps_each_ones_data_apart() {
    let (host, fake) = (Shared::default(), Fake::default());
    let mut a = app(&host, &fake);
    assert_eq!(a.song_slug(), "song");
    a.project.bpm = 133.0;
    a.project.tracks[0].name = "FIRST".into();
    with(&mut a, Btn::Fn, Btn::R1);
    assert_eq!(a.view(), View::Songs);
    assert_eq!(a.song_rows().len(), 1);
    assert!(a.song_rows()[0].open && a.song_rows()[0].bpm == 133.0, "{:?}", a.song_rows());
    press(&mut a, Btn::Down); // the "+ New song" row
    press(&mut a, Btn::A);
    assert_eq!((a.view(), a.song_slug()), (View::Mixer, "song-2"));
    assert_eq!(a.project.name, "SONG 2");
    assert_ne!(a.project.bpm, 133.0, "a fresh song, not a copy");
    assert_eq!(fake.0.lock().unwrap().switches.last().unwrap(), "song-2", "recordings now go to the new folder");
    assert!(!a.can_undo(), "undo history does not cross songs");
    a.project.bpm = 90.0;
    // back to the first: its tempo and track name are intact
    with(&mut a, Btn::Fn, Btn::R1);
    assert_eq!(a.song_rows().len(), 2);
    assert_eq!(a.song_sel(), 1, "the open song is picked");
    press(&mut a, Btn::Up);
    press(&mut a, Btn::A);
    assert_eq!((a.song_slug(), a.project.bpm, a.project.tracks[0].name.as_str()), ("song", 133.0, "FIRST"));
    assert_eq!(fake.0.lock().unwrap().switches.last().unwrap(), "song");
    // and the second one kept its own (it was saved when it was left)
    with(&mut a, Btn::Fn, Btn::R1);
    assert_eq!(a.song_rows()[1].bpm, 90.0);
    let mut fb = Fb::new();
    a.render(&mut fb);
    // the project files really are separate in the host
    assert!(host.load_state("hilldaw").unwrap().contains("133"));
    assert!(host.load_state("hilldaw-p-song-2").unwrap().contains("90"));
}

#[test]
fn the_open_song_is_remembered_across_a_restart_and_audio_belongs_to_its_song() {
    let (host, fake) = (Shared::default(), Fake::default());
    fake.add("song-2", "audio/rec-001.wav", 80_000);
    let mut a = app(&host, &fake);
    with(&mut a, Btn::Fn, Btn::R1);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::A); // song-2: its folder holds the unused recording
    assert!(a.toast_text().contains("unused") || a.orphan_count() == 1, "{}", a.toast_text());
    assert_eq!(a.orphan_count(), 1);
    a.project.bpm = 77.0;
    a.save();
    let b = app(&host, &fake);
    assert_eq!((b.song_slug(), b.project.bpm), ("song-2", 77.0));
    assert_eq!(fake.0.lock().unwrap().switches.last().unwrap(), "song-2", "the recorder was pointed at the open song before anything was opened");
    assert_eq!(b.orphan_count(), 1);
}

#[test]
fn removing_a_song_needs_two_presses_never_the_open_one_and_leaves_its_files() {
    let (host, fake) = (Shared::default(), Fake::default());
    fake.add("song-2", "audio/rec-001.wav", 80_000);
    let mut a = app(&host, &fake);
    with(&mut a, Btn::Fn, Btn::R1);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::A); // make song-2 and stay in it
    with(&mut a, Btn::Fn, Btn::R1);
    // the open song (row 1) cannot be removed
    press(&mut a, Btn::X);
    assert!(a.toast_text().contains("cannot be removed"), "{}", a.toast_text());
    press(&mut a, Btn::Up); // "song", not open
    press(&mut a, Btn::X);
    assert_eq!(a.song_rows().len(), 2, "one press only asks");
    press(&mut a, Btn::X);
    assert_eq!(a.song_rows().len(), 1);
    assert!(a.toast_text().contains("files stay"), "{}", a.toast_text());
    assert_eq!(fake.0.lock().unwrap().files["song-2"].len(), 1, "nothing was deleted");
    // a restart still has the remaining song open, and a new song never reuses a name that has data
    let mut b = app(&host, &fake);
    assert_eq!(b.song_slug(), "song-2");
    with(&mut b, Btn::Fn, Btn::R1);
    press(&mut b, Btn::Down);
    press(&mut b, Btn::A);
    assert_eq!(b.song_slug(), "song-3");
}

#[test]
fn the_browser_will_not_open_while_recording() {
    let (host, fake) = (Shared::default(), Fake::default());
    let mut a = app(&host, &fake);
    press(&mut a, Btn::A); // arm track 1
    press(&mut a, Btn::Select); // record (count-in first)
    assert!(a.is_recording());
    with(&mut a, Btn::Fn, Btn::R1);
    assert_ne!(a.view(), View::Songs);
    assert!(a.toast_text().contains("Stop recording"), "{}", a.toast_text());
}
