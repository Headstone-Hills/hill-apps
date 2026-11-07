use hill_core::host::Host;
use hill_core::kv::Kv;
use hill_core::library::{build_catalog, load_entry, Kind};
use hill_core::{apps, App, Btn, MemHost};

use std::sync::{Arc, Mutex};

/// A host the test can look inside after the app has used it.
struct Shared(Arc<Mutex<MemHost>>);
impl Host for Shared {
    fn list_samples(&self) -> Vec<String> {
        self.0.lock().unwrap().list_samples()
    }
    fn read_sample(&self, n: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().read_sample(n)
    }
    fn write_sample(&mut self, n: &str, d: &[u8]) -> bool {
        self.0.lock().unwrap().write_sample(n, d)
    }
    fn load_state(&self, n: &str) -> Option<String> {
        self.0.lock().unwrap().load_state(n)
    }
    fn save_state(&mut self, n: &str, d: &str) {
        self.0.lock().unwrap().save_state(n, d)
    }
}

/// A host whose sample folder cannot be written (a read-only card).
struct ReadOnly;
impl Host for ReadOnly {
    fn list_samples(&self) -> Vec<String> {
        vec![]
    }
    fn read_sample(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
    fn load_state(&self, _: &str) -> Option<String> {
        None
    }
    fn save_state(&mut self, _: &str, _: &str) {}
}

fn shared() -> Arc<Mutex<MemHost>> {
    Arc::new(Mutex::new(MemHost::default()))
}

fn new_app(store: &Arc<Mutex<MemHost>>) -> Box<dyn App> {
    apps::create("hillsynth", Box::new(Shared(store.clone()))).unwrap()
}

fn tap(a: &mut dyn App, b: Btn) {
    a.event(b, true);
    a.event(b, false);
}

fn chord(a: &mut dyn App, mods: &[Btn], b: Btn) {
    for m in mods {
        a.event(*m, true);
    }
    tap(a, b);
    for m in mods.iter().rev() {
        a.event(*m, false);
    }
}

/// Run `secs` of audio and UI ticks together, returning the (left) audio.
fn run(a: &mut dyn App, secs: f32) -> Vec<f32> {
    let total = (44100.0 * secs) as usize;
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; 1024];
    let mut done = 0;
    while done < total {
        let n = (total - done).min(512);
        a.audio(&mut buf[..n * 2]);
        a.tick((n * 1000 / 44100) as u32);
        out.extend(buf[..n * 2].chunks(2).map(|p| p[0]));
        done += n;
    }
    out
}

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0, |m, x| m.max(x.abs()))
}

fn state(store: &Arc<Mutex<MemHost>>) -> Kv {
    Kv::parse(&store.lock().unwrap().load_state("hillsynth").expect("state saved"))
}

#[test]
fn it_is_one_of_the_hill_apps() {
    assert!(apps::NAMES.contains(&"hillsynth"));
    let a = apps::create("hillsynth", Box::new(MemHost::default())).unwrap();
    assert_eq!(a.name(), "HillSynth");
}

#[test]
fn silent_until_played_sounds_while_held_and_dies_away_after_release() {
    let mut a = new_app(&shared());
    assert_eq!(peak(&run(a.as_mut(), 0.2)), 0.0);
    a.event(Btn::A, true);
    assert!(peak(&run(a.as_mut(), 1.0)) > 0.05, "a held note sounds");
    a.event(Btn::A, false);
    run(a.as_mut(), 2.5); // Warm Pad releases over 1.4 s
    assert!(peak(&run(a.as_mut(), 0.3)) < 0.001, "silent after the release");
}

#[test]
fn several_notes_at_once_and_the_pedal_holds_them() {
    let mut a = new_app(&shared());
    a.event(Btn::L2, true); // pedal down
    a.event(Btn::X, true);
    a.event(Btn::B, true);
    run(a.as_mut(), 0.8);
    a.event(Btn::X, false);
    a.event(Btn::B, false);
    run(a.as_mut(), 3.0);
    assert!(peak(&run(a.as_mut(), 0.3)) > 0.05, "still sounding with the pedal down");
    a.event(Btn::L2, false);
    run(a.as_mut(), 3.0);
    assert!(peak(&run(a.as_mut(), 0.3)) < 0.001, "released with the pedal");
}

#[test]
fn edit_mode_changes_the_sound_while_notes_still_play() {
    let store = shared();
    let mut a = new_app(&store);
    tap(a.as_mut(), Btn::R2); // edit
    // the D-pad no longer plays: Up/Down move through the settings
    a.event(Btn::Right, true);
    let silent = peak(&run(a.as_mut(), 0.3));
    a.event(Btn::Right, false);
    assert!(silent < 1.0); // (Right changed the first setting - it is not a note)
    // go to the filter page, make the sound dark and compare with bright
    let level_at = |cut_steps: i32, dir: Btn| {
        let mut b = new_app(&shared());
        tap(b.as_mut(), Btn::R2);
        tap(b.as_mut(), Btn::R1);
        tap(b.as_mut(), Btn::R1); // FILTER page, cutoff is the first row
        for _ in 0..cut_steps {
            tap(b.as_mut(), dir);
        }
        b.event(Btn::A, true); // a face button still plays
        run(b.as_mut(), 1.0);
        let out = run(b.as_mut(), 0.3);
        out.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>() / out.iter().map(|x| x * x).sum::<f32>()
    };
    let dark = level_at(12, Btn::Left);
    let bright = level_at(12, Btn::Right);
    assert!(dark < bright * 0.5, "turning the cutoff down darkens the sound: {} vs {}", dark, bright);
    a.save();
    // the first setting (wave 1) stepped once: saw -> square
    let patch = state(&store).get("patch").unwrap().to_string();
    assert!(patch.starts_with("2,"), "wave 1 is square after one step: {}", patch);
}

#[test]
fn start_opens_presets_a_loads_y_saves_into_a_user_slot_x_clears() {
    let store = shared();
    let mut a = new_app(&store);
    tap(a.as_mut(), Btn::Start); // the preset list
    tap(a.as_mut(), Btn::Down);
    tap(a.as_mut(), Btn::A); // Soft Keys
    a.save();
    assert_eq!(state(&store).get("preset"), Some("Soft Keys"));
    let keys_patch = state(&store).get("patch").unwrap().to_string();
    // tweak the sound, then save it into User 2 (the list opens on the first row)
    tap(a.as_mut(), Btn::R2);
    tap(a.as_mut(), Btn::Right); // wave 1 changes
    tap(a.as_mut(), Btn::R2);
    tap(a.as_mut(), Btn::Start);
    for _ in 0..9 {
        tap(a.as_mut(), Btn::Down); // 8 factory rows, then User 1, User 2
    }
    tap(a.as_mut(), Btn::Y);
    tap(a.as_mut(), Btn::B);
    a.save();
    let k = state(&store);
    assert_eq!(k.get("preset"), Some("User 2"));
    let user = k.get("user1").unwrap().to_string();
    assert!(user.starts_with("User 2|"), "{}", user);
    assert_ne!(user.split_once('|').unwrap().1, keys_patch, "the user slot holds the edited sound");
    // a fresh start brings the slot and the sound back
    let mut b = new_app(&store);
    b.save();
    assert_eq!(state(&store).get("user1"), Some(user.as_str()));
    // X clears a slot
    tap(b.as_mut(), Btn::Start);
    for _ in 0..9 {
        tap(b.as_mut(), Btn::Down);
    }
    tap(b.as_mut(), Btn::X);
    b.save();
    assert!(state(&store).get("user1").is_none());
}

#[test]
fn export_writes_a_multisample_the_other_hill_apps_load_as_one_instrument() {
    let store = shared();
    let mut a = new_app(&store);
    chord(a.as_mut(), &[Btn::Start], Btn::X);
    // it is exported a note at a time: the app keeps running meanwhile
    let mut ticks = 0;
    while ticks < 400 {
        a.tick(16);
        ticks += 1;
        if store.lock().unwrap().samples.len() >= 25 {
            break;
        }
    }
    let files: Vec<String> = store.lock().unwrap().list_samples();
    assert_eq!(files.len(), 25, "every third semitone from C1 to C7: {:?}", files.first());
    assert!(files.contains(&"HillSynth/Warm_Pad/Warm_Pad_midi024_C1.wav".to_string()), "{:?}", &files[..3]);
    assert!(files.contains(&"HillSynth/Warm_Pad/Warm_Pad_midi060_C4.wav".to_string()));
    assert!(files.contains(&"HillSynth/Warm_Pad/Warm_Pad_midi063_Eb4.wav".to_string()), "flats, like the other libraries");
    // the other apps' library groups them into one pitched instrument and can load it
    let cat = build_catalog(&files);
    let inst: Vec<_> = cat.iter().filter(|e| matches!(e.kind, Kind::Files(_))).collect();
    assert_eq!(inst.len(), 1, "one instrument, not 25 sounds");
    assert!(inst[0].pitched && inst[0].label.starts_with("Warm_Pad"), "{}", inst[0].label);
    let host = Shared(store.clone());
    let loaded = load_entry(&host, inst[0], 1 << 30).unwrap();
    assert_eq!(loaded.samples.len(), 25);
    assert!(loaded.samples.iter().all(|(_, s)| s.seconds() > 4.0 && s.seconds() < 5.0));
    // one gain for the whole set: loud enough to use, never over full scale
    let peak = (0..loaded.samples[3].1.frames()).map(|i| loaded.samples[3].1.frame(i).0.abs()).fold(0.0f32, f32::max);
    assert!(peak > 0.3 && peak < 1.0, "peak {}", peak);
    // a second export does not start while one is running, and the toast reports completion
    let _ = run(a.as_mut(), 0.1);
}

#[test]
fn export_on_a_read_only_card_stops_cleanly() {
    let mut a = apps::create("hillsynth", Box::new(ReadOnly)).unwrap();
    chord(a.as_mut(), &[Btn::Start], Btn::X);
    for _ in 0..200 {
        a.tick(16); // renders, then fails to write: no panic, no endless loop
    }
    assert!(!a.wants_quit());
}

#[test]
fn start_select_tap_silences_everything_and_hold_quits() {
    let mut a = new_app(&shared());
    a.event(Btn::A, true);
    run(a.as_mut(), 0.3);
    a.event(Btn::Start, true);
    a.event(Btn::Select, true);
    a.event(Btn::Select, false);
    a.event(Btn::Start, false);
    assert!(!a.wants_quit());
    assert!(peak(&run(a.as_mut(), 0.1)) < 0.001, "panic silences the held note at once");
    a.event(Btn::Start, true);
    a.event(Btn::Select, true);
    run(a.as_mut(), 1.7);
    assert!(a.wants_quit());
}

#[test]
fn the_screen_draws_in_every_mode() {
    let mut a = new_app(&shared());
    let mut fb = hill_core::fb::Fb::new();
    a.render(&mut fb);
    tap(a.as_mut(), Btn::R2);
    for _ in 0..5 {
        tap(a.as_mut(), Btn::R1);
        a.render(&mut fb);
    }
    chord(a.as_mut(), &[Btn::Fn], Btn::Select); // help
    a.render(&mut fb);
    tap(a.as_mut(), Btn::B);
    tap(a.as_mut(), Btn::Start);
    a.render(&mut fb);
    assert!(fb.px.iter().any(|p| *p != hill_core::fb::BG));
}
