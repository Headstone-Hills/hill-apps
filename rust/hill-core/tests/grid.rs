use hill_core::host::Host;
use hill_core::wav::encode16;
use hill_core::{apps, App, Btn, MemHost};

use std::sync::{Arc, Mutex};

struct Shared(Arc<Mutex<MemHost>>);
impl Host for Shared {
    fn list_samples(&self) -> Vec<String> {
        self.0.lock().unwrap().list_samples()
    }
    fn read_sample(&self, n: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().read_sample(n)
    }
    fn load_state(&self, n: &str) -> Option<String> {
        self.0.lock().unwrap().load_state(n)
    }
    fn save_state(&mut self, n: &str, d: &str) {
        self.0.lock().unwrap().save_state(n, d)
    }
}

fn shared() -> Arc<Mutex<MemHost>> {
    Arc::new(Mutex::new(MemHost::default()))
}

fn saved(store: &Arc<Mutex<MemHost>>, app: &str) -> hill_core::kv::Kv {
    hill_core::kv::Kv::parse(&store.lock().unwrap().load_state(app).expect("state saved"))
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

fn run(a: &mut dyn App, secs: f32) -> Vec<f32> {
    let total = (44100.0 * secs) as usize;
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; 1024];
    let mut done = 0;
    while done < total {
        let n = (total - done).min(512);
        a.audio(&mut buf[..n * 2]);
        a.tick((n * 1000 / 44100) as u32);
        out.extend_from_slice(&buf[..n * 2]);
        done += n;
    }
    out
}

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0, |m, x| m.max(x.abs()))
}

#[test]
fn silent_until_playing_and_a_step_is_set() {
    let mut a = apps::create("hillbeat", Box::new(MemHost::default())).unwrap();
    assert_eq!(peak(&run(a.as_mut(), 0.3)), 0.0);
    tap(a.as_mut(), Btn::Start); // play with an empty pattern
    assert_eq!(peak(&run(a.as_mut(), 0.5)), 0.0);
}

#[test]
fn step_toggle_plays_on_the_right_beat() {
    let mut a = apps::create("hillbeat", Box::new(MemHost::default())).unwrap();
    tap(a.as_mut(), Btn::Right); // move to step 2
    tap(a.as_mut(), Btn::A); // toggles on (and auditions)
    run(a.as_mut(), 0.5); // let audition ring out
    tap(a.as_mut(), Btn::Start);
    let out = run(a.as_mut(), 0.6);
    // step 2 fires at 1/16 note = 0.125 s at 120 bpm
    let first = out.chunks(2).position(|f| f[0].abs() > 0.01).unwrap();
    let t = first as f32 / 44100.0;
    assert!((t - 0.125).abs() < 0.01, "first sound at {}", t);
}

#[test]
fn velocity_scales_volume() {
    let mut loud = apps::create("hillbeat", Box::new(MemHost::default())).unwrap();
    let mut soft = apps::create("hillbeat", Box::new(MemHost::default())).unwrap();
    for (app, ups, downs) in [(&mut loud, 1, 0), (&mut soft, 0, 6)] {
        tap(app.as_mut(), Btn::A); // on
        app.event(Btn::A, true);
        for _ in 0..ups {
            tap(app.as_mut(), Btn::Up);
        }
        for _ in 0..downs {
            tap(app.as_mut(), Btn::Down);
        }
        app.event(Btn::A, false); // used as modifier: must NOT toggle off
        run(app.as_mut(), 0.5);
        tap(app.as_mut(), Btn::Start);
    }
    let pl = peak(&run(loud.as_mut(), 0.4));
    let ps = peak(&run(soft.as_mut(), 0.4));
    assert!(pl > ps * 1.5, "loud {} soft {}", pl, ps);
}

#[test]
fn mute_silences_track() {
    let mut a = apps::create("hillbeat", Box::new(MemHost::default())).unwrap();
    tap(a.as_mut(), Btn::A);
    run(a.as_mut(), 0.5);
    tap(a.as_mut(), Btn::Y); // mute
    tap(a.as_mut(), Btn::Start);
    assert_eq!(peak(&run(a.as_mut(), 0.5)), 0.0);
}

#[test]
fn bpm_changes_and_repeats_when_held() {
    let store = shared();
    let mut a = apps::create("hillbeat", Box::new(Shared(store.clone()))).unwrap();
    tap(a.as_mut(), Btn::R2);
    a.save();
    assert_eq!(saved(&store, "hillbeat").f32("bpm"), Some(121.0));
    a.event(Btn::R2, true);
    run(a.as_mut(), 1.0); // held: auto-repeat after 400ms, then every 70ms
    a.event(Btn::R2, false);
    a.save();
    let bpm = saved(&store, "hillbeat").f32("bpm").unwrap();
    assert!(bpm > 128.0 && bpm < 140.0, "bpm {}", bpm);
    a.event(Btn::L2, true);
    a.event(Btn::L2, false);
    a.save();
    assert_eq!(saved(&store, "hillbeat").f32("bpm"), Some(bpm - 1.0));
}

#[test]
fn state_roundtrips_through_host() {
    let store = shared();
    let mut a = apps::create("hillband", Box::new(Shared(store.clone()))).unwrap();
    tap(a.as_mut(), Btn::A);
    tap(a.as_mut(), Btn::Right);
    tap(a.as_mut(), Btn::Right);
    tap(a.as_mut(), Btn::A);
    chord(a.as_mut(), &[Btn::Start], Btn::Y); // save
    let saved = store.lock().unwrap().load_state("hillband").unwrap();
    assert!(saved.contains("p0.t0=640064"), "{}", saved);
    // new instance restores the pattern
    let mut b = apps::create("hillband", Box::new(Shared(store.clone()))).unwrap();
    tap(b.as_mut(), Btn::Start);
    assert!(peak(&run(b.as_mut(), 0.1)) > 0.0);
}

#[test]
fn pattern_switch_waits_for_bar_end_while_playing() {
    let mut a = apps::create("hillbeat", Box::new(MemHost::default())).unwrap();
    tap(a.as_mut(), Btn::R1); // pattern 2, empty, immediate while stopped
    tap(a.as_mut(), Btn::A);
    run(a.as_mut(), 0.5);
    tap(a.as_mut(), Btn::L1); // back to pattern 1 (empty)
    tap(a.as_mut(), Btn::Start);
    assert_eq!(peak(&run(a.as_mut(), 0.3)), 0.0);
    tap(a.as_mut(), Btn::R1); // queue pattern 2 for the next bar
    assert_eq!(peak(&run(a.as_mut(), 1.0)), 0.0); // still bar 1
    assert!(peak(&run(a.as_mut(), 1.0)) > 0.01); // bar 2 starts at 2.0s
}

#[test]
fn start_select_hold_quits_tap_stops() {
    let mut a = apps::create("hillbeat", Box::new(MemHost::default())).unwrap();
    tap(a.as_mut(), Btn::Start);
    run(a.as_mut(), 0.2);
    a.event(Btn::Start, true);
    a.event(Btn::Select, true);
    run(a.as_mut(), 0.3);
    a.event(Btn::Select, false);
    a.event(Btn::Start, false);
    assert!(!a.wants_quit());
    a.event(Btn::Start, true);
    a.event(Btn::Select, true);
    run(a.as_mut(), 2.2);
    assert!(a.wants_quit());
}

#[test]
fn band_chord_overlay_applies_to_track() {
    let store = shared();
    let mut a = apps::create("hillband", Box::new(Shared(store.clone()))).unwrap();
    chord(a.as_mut(), &[Btn::Start], Btn::L2);
    tap(a.as_mut(), Btn::Right); // unison -> power
    tap(a.as_mut(), Btn::Down); // scale none -> major
    tap(a.as_mut(), Btn::A);
    a.save();
    let k = saved(&store, "hillband");
    assert_eq!(k.usize("t0.chord"), Some(1));
    assert_eq!(k.usize("t0.scale"), Some(1));
    assert_eq!(k.usize("t1.chord"), Some(0));
    // cancel leaves things alone
    chord(a.as_mut(), &[Btn::Start], Btn::L2);
    tap(a.as_mut(), Btn::Right);
    tap(a.as_mut(), Btn::B);
    a.save();
    assert_eq!(saved(&store, "hillband").usize("t0.chord"), Some(1));
}

#[test]
fn drum_track_refuses_chords() {
    let mut a = apps::create("hillband", Box::new(MemHost::default())).unwrap();
    for _ in 0..4 {
        tap(a.as_mut(), Btn::Down);
    }
    chord(a.as_mut(), &[Btn::Start], Btn::L2);
    let mut fb = hill_core::fb::Fb::new();
    a.render(&mut fb); // no overlay: just must not panic
}

#[test]
fn sequencer_key_overlay_spreads_scale() {
    let store = shared();
    let mut a = apps::create("hillsequencer", Box::new(Shared(store.clone()))).unwrap();
    chord(a.as_mut(), &[Btn::Start], Btn::L2);
    tap(a.as_mut(), Btn::Right); // root C -> C#, scale stays major
    tap(a.as_mut(), Btn::A);
    a.save();
    let k = saved(&store, "hillsequencer");
    assert_eq!(k.usize("t7.note"), Some(49)); // lowest track: the root
    assert_eq!(k.usize("t0.note"), Some(61)); // highest track: root + octave
    assert_eq!(k.usize("t3.note"), Some(56));
}

#[test]
fn multi_select_assigns_one_sound_to_many_tracks() {
    let store = shared();
    let mut a = apps::create("hillsequencer", Box::new(Shared(store.clone()))).unwrap();
    chord(a.as_mut(), &[Btn::Start], Btn::R2); // select all tracks
    chord(a.as_mut(), &[Btn::Start], Btn::B); // browser
    // the browser opens in Built-in on the current sound (Bell, the last one); Down wraps to Kick
    tap(a.as_mut(), Btn::Down);
    tap(a.as_mut(), Btn::A);
    a.save();
    let k = saved(&store, "hillsequencer");
    for t in 0..8 {
        assert_eq!(k.get(&format!("t{}.inst", t)), Some("builtin:Kick"), "track {}", t);
    }
}

#[test]
fn multi_select_toggles_off_again() {
    let store = shared();
    let mut a = apps::create("hillsequencer", Box::new(Shared(store.clone()))).unwrap();
    chord(a.as_mut(), &[Btn::Start], Btn::R2); // all on
    chord(a.as_mut(), &[Btn::Start], Btn::R2); // all off
    chord(a.as_mut(), &[Btn::Start], Btn::B);
    tap(a.as_mut(), Btn::Down);
    tap(a.as_mut(), Btn::A);
    a.save();
    let k = saved(&store, "hillsequencer");
    assert_eq!(k.get("t0.inst"), Some("builtin:Kick"));
    assert_ne!(k.get("t1.inst"), Some("builtin:Kick"), "only the cursor track changed");
}

#[test]
fn browser_is_a_folder_tree() {
    let tone: Vec<f32> = (0..2000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    let wav = encode16(&tone, 1, 44100);
    let store = shared();
    for f in ["Lib/Keys/Piano/Piano_C3.wav", "Lib/Keys/Piano/Piano_C4.wav", "Lib/Keys/Organ/Organ_C3.wav", "Lib/Keys/Organ/Organ_C4.wav"] {
        store.lock().unwrap().samples.insert(f.into(), wav.clone());
    }
    let mut a = apps::create("hillsequencer", Box::new(Shared(store.clone()))).unwrap();
    chord(a.as_mut(), &[Btn::Start], Btn::B);
    tap(a.as_mut(), Btn::B); // up from Built-in to the root: rows are Built-in, Lib
    tap(a.as_mut(), Btn::Down); // Built-in -> Lib
    tap(a.as_mut(), Btn::A); // enter Lib (single sub-folder chain: Keys)
    tap(a.as_mut(), Btn::A); // enter Keys: rows Organ, Piano
    tap(a.as_mut(), Btn::Down); // Piano
    tap(a.as_mut(), Btn::A); // enter Piano
    tap(a.as_mut(), Btn::A); // pick its only sound
    a.save();
    let k = saved(&store, "hillsequencer");
    assert_eq!(k.get("t0.inst"), Some("Lib/Keys/Piano/Piano*"));
}

#[test]
fn loop_follows_tempo_with_wav_from_host() {
    let mut host = MemHost::default();
    // a 2-second loop: 'bpm' in name -> 4 beats at 120 would be 2s
    let tone: Vec<f32> = (0..88200).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    host.samples.insert("loops/groove_120bpm.wav".into(), encode16(&tone, 1, 44100));
    let mut a = apps::create("hillbeat", Box::new(host)).unwrap();
    chord(a.as_mut(), &[Btn::Start], Btn::A); // opens loop browser (no loop yet)
    // browser lists the one file; pick it
    tap(a.as_mut(), Btn::A);
    tap(a.as_mut(), Btn::Start);
    let out = run(a.as_mut(), 0.5);
    assert!(peak(&out) > 0.2, "loop audible");
    // pause: loop stops advancing and goes quiet
    tap(a.as_mut(), Btn::Start);
    let out = run(a.as_mut(), 0.2);
    assert!(peak(&out) < 0.01);
}

#[test]
fn bad_wav_in_library_does_not_crash() {
    let mut host = MemHost::default();
    host.samples.insert("kit/kick.wav".into(), b"not a wav".to_vec());
    let mut a = apps::create("hillbeat", Box::new(host)).unwrap();
    chord(a.as_mut(), &[Btn::Start], Btn::B);
    for _ in 0..9 {
        tap(a.as_mut(), Btn::Down);
    }
    tap(a.as_mut(), Btn::A); // try to load the corrupt file; must survive
    let mut fb = hill_core::fb::Fb::new();
    a.render(&mut fb);
}

#[test]
fn picking_a_drum_kit_fills_every_drum_track_in_order() {
    let tone: Vec<f32> = (0..2000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    let wav = encode16(&tone, 1, 44100);
    // the kit is somewhere in the drum-filtered list: scan down to it
    let mut found = false;
    for downs in 0..40 {
        let store = shared();
        for f in ["Packs/Dusty/perc_x.wav", "Packs/Dusty/hat_closed.wav", "Packs/Dusty/kick_01.wav", "Packs/Dusty/snare_01.wav", "Pads/warm.wav"] {
            store.lock().unwrap().samples.insert(f.into(), wav.clone());
        }
        let mut a = apps::create("hillbeat", Box::new(Shared(store.clone()))).unwrap();
        chord(a.as_mut(), &[Btn::Start], Btn::B);
        tap(a.as_mut(), Btn::B); // up to the root: Built-in, Packs
        tap(a.as_mut(), Btn::Down);
        tap(a.as_mut(), Btn::A); // into Packs
        for _ in 0..downs {
            tap(a.as_mut(), Btn::Down);
        }
        tap(a.as_mut(), Btn::A);
        a.save();
        let k = saved(&store, "hillbeat");
        if k.get("t0.inst") == Some("Packs/Dusty/kick_01.wav") {
            assert_eq!(k.get("t1.inst"), Some("Packs/Dusty/snare_01.wav"));
            assert_eq!(k.get("t2.inst"), Some("Packs/Dusty/hat_closed.wav"));
            assert_eq!(k.get("t3.inst"), Some("Packs/Dusty/perc_x.wav"));
            found = true;
            break;
        }
    }
    assert!(found, "kit entry reachable in the drum browser");
}

#[test]
fn pausing_silences_ringing_notes() {
    let mut a = apps::create("hillsequencer", Box::new(MemHost::default())).unwrap();
    tap(a.as_mut(), Btn::A); // a step on track 1
    tap(a.as_mut(), Btn::Start); // play
    let playing = run(a.as_mut(), 0.6);
    assert!(peak(&playing) > 0.05, "audible while playing");
    tap(a.as_mut(), Btn::Start); // pause
    run(a.as_mut(), 0.3); // release fade
    assert!(peak(&run(a.as_mut(), 0.5)) < 1e-4, "silent after pausing");
}

#[test]
fn setting_a_step_on_a_long_sound_does_not_drone() {
    // a 6 s steady tone as the only instrument on track 1, set up through saved state
    let tone: Vec<f32> = (0..44100 * 6).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    let store = shared();
    store.lock().unwrap().samples.insert("Pads/pad.wav".into(), encode16(&tone, 1, 44100));
    let mut a = apps::create("hillsequencer", Box::new(Shared(store.clone()))).unwrap();
    // browse to the pad: up to the root, into Pads, pick it
    chord(a.as_mut(), &[Btn::Start], Btn::B);
    tap(a.as_mut(), Btn::B);
    tap(a.as_mut(), Btn::Down); // Built-in -> pad.wav (a folder with one sound shows the sound)
    tap(a.as_mut(), Btn::A);
    a.save();
    assert_eq!(saved(&store, "hillsequencer").get("t0.inst"), Some("Pads/pad.wav"), "the 6 s pad was chosen");
    run(a.as_mut(), 1.0); // the choose-a-sound audition rings out and is cut
    tap(a.as_mut(), Btn::A); // set a step: audition
    let first = run(a.as_mut(), 0.3);
    assert!(peak(&first) > 0.05, "audition is audible");
    run(a.as_mut(), 0.6); // 450 ms cut plus the release fade
    assert!(peak(&run(a.as_mut(), 1.0)) < 1e-4, "audition note must not keep droning");
}

/// Counts how many sample files the app reads.
struct Counting(Arc<Mutex<MemHost>>, Arc<Mutex<usize>>);
impl Host for Counting {
    fn list_samples(&self) -> Vec<String> {
        self.0.lock().unwrap().list_samples()
    }
    fn read_sample(&self, n: &str) -> Option<Vec<u8>> {
        *self.1.lock().unwrap() += 1;
        self.0.lock().unwrap().read_sample(n)
    }
    fn load_state(&self, n: &str) -> Option<String> {
        self.0.lock().unwrap().load_state(n)
    }
    fn save_state(&mut self, n: &str, d: &str) {
        self.0.lock().unwrap().save_state(n, d)
    }
}

#[test]
fn tracks_sharing_a_sound_load_it_once() {
    let wav = encode16(&(0..2000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect::<Vec<_>>(), 1, 44100);
    let store = shared();
    for n in ["C3", "C4", "C5"] {
        store.lock().unwrap().samples.insert(format!("Keys/Piano/Piano_{}.wav", n), wav.clone());
    }
    let reads = Arc::new(Mutex::new(0usize));
    let mut a = apps::create("hillsequencer", Box::new(Counting(store.clone(), reads.clone()))).unwrap();
    chord(a.as_mut(), &[Btn::Start], Btn::R2); // all tracks
    chord(a.as_mut(), &[Btn::Start], Btn::B);
    tap(a.as_mut(), Btn::B);
    tap(a.as_mut(), Btn::Down);
    tap(a.as_mut(), Btn::A); // enter Keys... (folder chain)
    tap(a.as_mut(), Btn::A);
    a.save();
    let state = saved(&store, "hillsequencer");
    assert_eq!(state.get("t7.inst"), Some("Keys/Piano/Piano*"));
    let st = store.lock().unwrap().load_state("hillsequencer").unwrap();
    let mut b = apps::create("hillsequencer", Box::new(Counting(store.clone(), reads.clone()))).unwrap();
    let before = *reads.lock().unwrap();
    // restart with the saved state: 8 tracks, one sound
    store.lock().unwrap().state.insert("hillsequencer".into(), st);
    drop(b);
    b = apps::create("hillsequencer", Box::new(Counting(store.clone(), reads.clone()))).unwrap();
    let after = *reads.lock().unwrap();
    assert_eq!(after - before, 3, "3 files read once, not 8 times");
    let _ = b;
}
