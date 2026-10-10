use hill_core::host::Host;
use hill_core::wav::encode16;
use hill_core::{apps, App, Btn, MemHost};

use std::cell::RefCell;
use std::rc::Rc;

struct Shared(Rc<RefCell<MemHost>>);
impl Host for Shared {
    fn list_samples(&self) -> Vec<String> {
        self.0.borrow().list_samples()
    }
    fn read_sample(&self, n: &str) -> Option<Vec<u8>> {
        self.0.borrow().read_sample(n)
    }
    fn load_state(&self, n: &str) -> Option<String> {
        self.0.borrow().load_state(n)
    }
    fn save_state(&mut self, n: &str, d: &str) {
        self.0.borrow_mut().save_state(n, d)
    }
}

fn shared() -> Rc<RefCell<MemHost>> {
    Rc::new(RefCell::new(MemHost::default()))
}

fn saved(store: &Rc<RefCell<MemHost>>, app: &str) -> hill_core::kv::Kv {
    hill_core::kv::Kv::parse(&store.borrow().load_state(app).expect("state saved"))
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
    let saved = store.borrow().load_state("hillband").unwrap();
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
    chord(a.as_mut(), &[Btn::Start], Btn::R2); // select track 1
    tap(a.as_mut(), Btn::Down);
    chord(a.as_mut(), &[Btn::Start], Btn::R2); // select track 2
    chord(a.as_mut(), &[Btn::Start], Btn::B); // browser
    // the browser opens on the current sound (Bell, the last built-in); Down wraps to Kick
    tap(a.as_mut(), Btn::Down);
    tap(a.as_mut(), Btn::A);
    a.save();
    let k = saved(&store, "hillsequencer");
    assert_eq!(k.get("t0.inst"), Some("builtin:Kick"));
    assert_eq!(k.get("t1.inst"), Some("builtin:Kick"));
    assert_ne!(k.get("t2.inst"), Some("builtin:Kick"));
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
