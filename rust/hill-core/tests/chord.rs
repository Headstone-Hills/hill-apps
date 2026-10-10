use hill_core::{apps, App, Btn, MemHost};

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
        let n = (total - done).min(441);
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

/// Peak in the time window [t0, t1) seconds of a recording.
fn win(v: &[f32], t0: f32, t1: f32) -> f32 {
    let a = (t0 * 44100.0) as usize * 2;
    let b = ((t1 * 44100.0) as usize * 2).min(v.len());
    peak(&v[a..b])
}

fn onsets(v: &[f32]) -> usize {
    let w = 1102; // 25 ms
    let mut n = 0;
    let mut prev = 0.0;
    for c in v.chunks(w * 2) {
        let p = peak(c);
        if p > 0.02 && prev < 0.004 {
            n += 1;
        }
        prev = p;
    }
    n
}

fn pick_builtin(a: &mut dyn App, ups: usize) {
    tap(a, Btn::Start); // library opens on release of a plain START
    for _ in 0..ups {
        tap(a, Btn::Up);
    }
    tap(a, Btn::A);
}

#[test]
fn silent_until_played_then_sounds_and_releases() {
    let mut a = apps::create("hillchord", Box::new(MemHost::default())).unwrap();
    assert_eq!(peak(&run(a.as_mut(), 0.2)), 0.0);
    a.event(Btn::A, true);
    let on = run(a.as_mut(), 0.3);
    assert!(peak(&on) > 0.1);
    a.event(Btn::A, false);
    let tail = run(a.as_mut(), 0.4);
    // long sounds release over 250 ms (short ones over 80 ms)
    assert!(win(&tail, 0.3, 0.4) < 0.001, "release should fade to silence");
}

#[test]
fn chord_has_several_notes_and_note_mode_one() {
    let mut chord_a = apps::create("hillchord", Box::new(MemHost::default())).unwrap();
    let mut note_a = apps::create("hillchord", Box::new(MemHost::default())).unwrap();
    chord(note_a.as_mut(), &[Btn::Select], Btn::X); // note mode
    chord_a.event(Btn::A, true);
    note_a.event(Btn::A, true);
    let c = run(chord_a.as_mut(), 0.2);
    let n = run(note_a.as_mut(), 0.2);
    let rms = |v: &[f32]| (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
    assert!(rms(&c) > 0.0 && rms(&n) > 0.0);
    assert!(c != n, "chord and note should differ");
}

#[test]
fn looper_records_loops_and_replays() {
    let mut a = apps::create("hillchord", Box::new(MemHost::default())).unwrap();
    tap(a.as_mut(), Btn::L2); // IDLE -> REC at beat 0
    run(a.as_mut(), 0.5); // beat 1
    a.event(Btn::A, true);
    run(a.as_mut(), 0.25);
    a.event(Btn::A, false);
    run(a.as_mut(), 1.25); // t = 2.0s = beat 4
    tap(a.as_mut(), Btn::L2); // REC -> LOOP, 4 beats long
    let out = run(a.as_mut(), 4.1);
    // loop event is at beat 1 of each 4-beat cycle: 0.5 s into every 2 s cycle
    assert!(win(&out, 0.52, 0.8) > 0.05, "first replay");
    assert!(win(&out, 0.0, 0.45) < 0.002, "quiet before the note");
    assert!(win(&out, 1.2, 1.9) < 0.002, "quiet after the note");
    assert!(win(&out, 2.52, 2.8) > 0.05, "second replay");
}

#[test]
fn overdub_adds_events_and_clear_empties_loop() {
    let mut a = apps::create("hillchord", Box::new(MemHost::default())).unwrap();
    tap(a.as_mut(), Btn::L2);
    run(a.as_mut(), 2.0);
    tap(a.as_mut(), Btn::L2); // empty 4-beat loop
    assert!(peak(&run(a.as_mut(), 2.0)) == 0.0);
    tap(a.as_mut(), Btn::L2); // overdub
    run(a.as_mut(), 0.2);
    a.event(Btn::A, true);
    run(a.as_mut(), 0.2);
    a.event(Btn::A, false);
    run(a.as_mut(), 0.5);
    tap(a.as_mut(), Btn::L2); // back to loop
    let later = run(a.as_mut(), 4.0);
    assert!(peak(&later) > 0.05, "overdubbed note replays");
    // hold L2 for 2s clears
    a.event(Btn::L2, true);
    run(a.as_mut(), 2.1);
    a.event(Btn::L2, false);
    run(a.as_mut(), 0.5);
    assert_eq!(peak(&run(a.as_mut(), 4.0)), 0.0);
}

#[test]
fn loop_follows_bpm_changes() {
    let mut a = apps::create("hillchord", Box::new(MemHost::default())).unwrap();
    tap(a.as_mut(), Btn::L2);
    run(a.as_mut(), 0.5);
    a.event(Btn::A, true);
    run(a.as_mut(), 0.25);
    a.event(Btn::A, false);
    run(a.as_mut(), 1.25);
    tap(a.as_mut(), Btn::L2);
    for _ in 0..60 {
        tap(a.as_mut(), Btn::Right); // 120 -> 180 bpm
    }
    run(a.as_mut(), 0.1);
    let out = run(a.as_mut(), 6.0);
    // at 180 bpm a 4-beat loop is 1.333 s: replays are ~1.33 s apart
    assert!(onsets(&out) >= 3, "onsets {}", onsets(&out));
}

#[test]
fn arpeggiator_fires_discrete_notes() {
    let mut a = apps::create("hillchord", Box::new(MemHost::default())).unwrap();
    pick_builtin(a.as_mut(), 3); // Pluck -> Hat (short sound)
    chord(a.as_mut(), &[Btn::Select], Btn::B); // arp on, 1/8 = 0.25s at 120
    a.event(Btn::A, true);
    let out = run(a.as_mut(), 1.2);
    a.event(Btn::A, false);
    let n = onsets(&out);
    assert!((4..=5).contains(&n), "expected ~4-5 arp notes, got {}", n);
    // with arp off the same hold produces one onset
    let mut b = apps::create("hillchord", Box::new(MemHost::default())).unwrap();
    pick_builtin(b.as_mut(), 3);
    b.event(Btn::A, true);
    let out = run(b.as_mut(), 1.2);
    assert_eq!(onsets(&out), 1);
}

#[test]
fn metronome_clicks_on_beats() {
    let mut a = apps::create("hillchord", Box::new(MemHost::default())).unwrap();
    chord(a.as_mut(), &[Btn::Select], Btn::A);
    let out = run(a.as_mut(), 2.2);
    let n = onsets(&out);
    assert!((3..=5).contains(&n), "clicks {}", n);
}

#[test]
fn dpad_modifier_retriggers_held_chord() {
    let mut a = apps::create("hillchord", Box::new(MemHost::default())).unwrap();
    a.event(Btn::A, true);
    let before = run(a.as_mut(), 0.1);
    a.event(Btn::Up, true); // add the 7th while held
    let after = run(a.as_mut(), 0.1);
    assert!(peak(&before) > 0.0 && peak(&after) > 0.0);
    a.event(Btn::Up, false);
    a.event(Btn::A, false);
}

#[test]
fn state_persists_settings() {
    use hill_core::host::Host;
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
    let store = Arc::new(Mutex::new(MemHost::default()));
    let mut a = apps::create("hillchord", Box::new(Shared(store.clone()))).unwrap();
    tap(a.as_mut(), Btn::Up); // minor
    chord(a.as_mut(), &[Btn::Fn], Btn::Up); // root up
    chord(a.as_mut(), &[Btn::Select], Btn::A); // metronome
    chord(a.as_mut(), &[Btn::Start], Btn::Y); // save
    let text = store.lock().unwrap().load_state("hillchord").unwrap();
    assert!(text.contains("minor=1") && text.contains("root=1") && text.contains("click=1"), "{}", text);
    let mut b = apps::create("hillchord", Box::new(Shared(store.clone()))).unwrap();
    let out = run(b.as_mut(), 1.2);
    assert!(onsets(&out) >= 1, "restored metronome clicks");
}

#[test]
fn favorites_toggle_in_library() {
    use hill_core::host::Host;
    use std::sync::{Arc, Mutex};
    struct Shared(Arc<Mutex<MemHost>>);
    impl Host for Shared {
        fn list_samples(&self) -> Vec<String> {
            vec![]
        }
        fn read_sample(&self, _: &str) -> Option<Vec<u8>> {
            None
        }
        fn load_state(&self, n: &str) -> Option<String> {
            self.0.lock().unwrap().load_state(n)
        }
        fn save_state(&mut self, n: &str, d: &str) {
            self.0.lock().unwrap().save_state(n, d)
        }
    }
    let store = Arc::new(Mutex::new(MemHost::default()));
    let mut a = apps::create("hillchord", Box::new(Shared(store.clone()))).unwrap();
    tap(a.as_mut(), Btn::Start);
    tap(a.as_mut(), Btn::Y); // star the current (Pluck)
    tap(a.as_mut(), Btn::B);
    a.save();
    let text = store.lock().unwrap().load_state("hillchord").unwrap();
    assert!(text.contains("favs=builtin:Pluck"), "{}", text);
}

#[test]
fn start_select_tap_panics_hold_quits() {
    let mut a = apps::create("hillchord", Box::new(MemHost::default())).unwrap();
    a.event(Btn::A, true);
    run(a.as_mut(), 0.1);
    a.event(Btn::Start, true);
    a.event(Btn::Select, true);
    run(a.as_mut(), 0.1);
    a.event(Btn::Select, false);
    a.event(Btn::Start, false);
    assert!(!a.wants_quit());
    assert_eq!(peak(&run(a.as_mut(), 0.2)), 0.0, "panic silences everything");
    a.event(Btn::Start, true);
    a.event(Btn::Select, true);
    run(a.as_mut(), 1.7);
    assert!(a.wants_quit());
}
