//! hdtool: hardware-in-the-loop tests for the recording path (run on the SP over SSH).
//!
//!   hdtool record SECS OUT.wav [--note N]      capture -> mono 24-bit -> bounded queue -> SD card, with overrun counts
//!   hdtool latency [--runs N] [--note N]       MIDI note-on -> first audible sample in the capture, median/min/max
//!   hdtool drums list [TEXT]                   the HillBeat presets (id, name, style), optionally filtered
//!   hdtool drums render ID OUT.wav [--cycles N] [--bpm B] [--swing S] [--samples DIR]
//!                                              render a preset through the DAW mixer and limiter, so it can be heard with `aplay`
//!   hdtool probe-cc [--ch 2] [--from 70] [--to 93] [--out FILE]
//!                                              play a note while changing each MIDI CC and measure, from the FM-1's own audio, what it does
//!   hdtool record SECS OUT.wav --synthetic     no FM-1: a real-time tone source (to test the writer on any machine)

use hilldaw_core::record::{detect_onset, mono24_to_f32, Downmixer, StreamingWav};
use hilldaw_usb::desc::{parse_audio_streams, pick_capture};
use hilldaw_usb::dev::{find_devices, DeviceInfo, MidiDevice, SYSFS_USB};
use hilldaw_usb::iso::IsoCapture;
use hilldaw_usb::{midi, wav, FM1_PID, FM1_VID};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::process::exit;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const RATE: u32 = 44100;

fn opt<T: std::str::FromStr>(a: &[String], n: &str, d: T) -> T {
    a.iter().position(|x| x == n).and_then(|i| a.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(d)
}

fn device() -> DeviceInfo {
    find_devices(FM1_VID, FM1_PID, Path::new(SYSFS_USB)).into_iter().next().unwrap_or_else(|| {
        eprintln!("FM-1 not found. Plugged into the USB-C port?");
        exit(2)
    })
}

fn open_capture(d: &DeviceInfo) -> IsoCapture {
    let streams = parse_audio_streams(&d.descriptors().unwrap_or_default());
    let cap = pick_capture(&streams, RATE).unwrap_or_else(|| { eprintln!("no capture endpoint"); exit(2) });
    IsoCapture::open(d, cap, 16, 8).unwrap_or_else(|e| { eprintln!("{e}"); exit(2) })
}

/// What the writer thread reports back.
struct WriterReport {
    bytes: u64,
    flushes: u32,
    worst_write_ms: f64,
}

fn record(args: &[String]) {
    let secs: f64 = args.first().and_then(|v| v.parse().ok()).unwrap_or(5.0);
    let out = args.get(1).cloned().unwrap_or_else(|| "take.wav".into());
    let synthetic = args.iter().any(|a| a == "--synthetic");

    // capture thread (this one) -> bounded queue -> writer thread -> SD card
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(512); // ~512 USB-sized chunks of slack
    let out2 = out.clone();
    let writer = std::thread::spawn(move || {
        let f = File::create(&out2).unwrap_or_else(|e| { eprintln!("{out2}: {e}"); exit(2) });
        let mut w = StreamingWav::new(BufWriter::with_capacity(256 * 1024, f), 1, 3, RATE).unwrap();
        let (mut worst, mut last_flush) = (0.0f64, Instant::now());
        for chunk in rx {
            let t = Instant::now();
            w.write(&chunk).unwrap();
            if last_flush.elapsed() >= Duration::from_secs(1) {
                w.flush_header().unwrap(); // a crash now loses at most ~1 s
                last_flush = Instant::now();
            }
            worst = worst.max(t.elapsed().as_secs_f64() * 1000.0);
        }
        let flushes = w.header_flushes;
        let bytes = w.data_len();
        let _ = w.finish().unwrap().into_inner(); // BufWriter flushes on drop via into_inner
        WriterReport { bytes, flushes, worst_write_ms: worst }
    });

    let overruns = Arc::new(AtomicU64::new(0));
    let captured_frames = Arc::new(AtomicU64::new(0));
    let (ov, cf) = (overruns.clone(), captured_frames.clone());
    let mut dm = Downmixer::default();
    let mut scratch = Vec::with_capacity(512);
    let sink = move |p: &[u8]| {
        scratch.clear();
        dm.push(p, &mut scratch);
        cf.fetch_add((scratch.len() / 3) as u64, Ordering::Relaxed);
        if tx.try_send(scratch.clone()).is_err() {
            ov.fetch_add(1, Ordering::Relaxed); // the writer fell behind: a chunk was dropped
        }
    };

    let t0 = Instant::now();
    let (stats_line, wall);
    if synthetic {
        let mut sink = sink;
        let stop = AtomicBool::new(false);
        let mut n = 0u64;
        let step = 45usize; // one USB frame per ms
        while t0.elapsed().as_secs_f64() < secs && !stop.load(Ordering::Relaxed) {
            let due = Duration::from_micros(n * 1000);
            if let Some(w) = due.checked_sub(t0.elapsed()) { std::thread::sleep(w); }
            let mut pkt = Vec::with_capacity(step * 6);
            for i in 0..step {
                let t = (n as usize * step + i) as f64 / RATE as f64;
                let v = ((0.3 * 8388607.0 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as i32).to_le_bytes();
                pkt.extend_from_slice(&v[..3]);
                pkt.extend_from_slice(&v[..3]);
            }
            sink(&pkt);
            n += 1;
        }
        stats_line = format!("synthetic source, {n} ms of packets");
        wall = t0.elapsed().as_secs_f64();
    } else {
        let d = device();
        let mut cp = open_capture(&d);
        if let Some(note) = args.iter().position(|a| a == "--note").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u8>().ok()) {
            std::thread::spawn(move || {
                if let Ok(m) = MidiDevice::open(&d) {
                    std::thread::sleep(Duration::from_millis(500));
                    let _ = m.write(&midi::note_on(0, note, 110));
                    std::thread::sleep(Duration::from_secs_f64((secs / 2.0).max(0.3)));
                    let _ = m.write(&midi::note_off(0, note));
                }
            });
        }
        let st = cp.run(&AtomicBool::new(false), secs, sink).unwrap_or_else(|e| { eprintln!("{e}"); exit(2) });
        stats_line = format!("usb: {}", st.summary());
        wall = t0.elapsed().as_secs_f64();
    }
    let rep = writer.join().unwrap();
    let frames = rep.bytes / 3;
    println!("{stats_line}");
    println!("recorded {frames} mono frames = {:.3} s (wall {:.2} s), file {out}", frames as f64 / RATE as f64, wall);
    println!("captured_frames={} written_frames={} overruns={}  header_flushes={}  worst single write {:.2} ms",
        captured_frames.load(Ordering::Relaxed), frames, overruns.load(Ordering::Relaxed), rep.flushes, rep.worst_write_ms);
    let b = std::fs::read(&out).unwrap();
    let (c, bps, rate, data) = wav::read_wav(&b).unwrap();
    println!("{}", wav::analyze(data, c as usize, bps as usize, rate));
    let ok = overruns.load(Ordering::Relaxed) == 0 && captured_frames.load(Ordering::Relaxed) == frames;
    println!("RESULT: {}", if ok { "CLEAN (no dropped chunks, every captured frame is on disk)" } else { "PROBLEMS (see counts above)" });
}

fn latency(args: &[String]) {
    let runs: usize = opt(args, "--runs", 8);
    let note: u8 = opt(args, "--note", 60);
    let d = device();
    // one claim of the MIDI interface for the whole test (usbfs allows a single claim per interface)
    let midi_dev = Arc::new(MidiDevice::open(&d).unwrap_or_else(|e| { eprintln!("{e}"); exit(2) }));
    let mut results = Vec::new();
    println!("MIDI note {note} -> first audible sample, {runs} runs (each ~1.2 s). Keep the FM-1 volume UP.");
    for run in 0..runs {
        let mut cp = open_capture(&d);
        let mut pcm: Vec<u8> = Vec::new();
        let sent_at = Arc::new(std::sync::Mutex::new(None::<Instant>));
        let s2 = sent_at.clone();
        let md = midi_dev.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            *s2.lock().unwrap() = Some(Instant::now());
            let _ = md.write(&midi::note_on(0, note, 110));
            std::thread::sleep(Duration::from_millis(500));
            let _ = md.write(&midi::note_off(0, note));
        });
        let t_submit = Instant::now();
        let mut dm = Downmixer::default();
        cp.run(&AtomicBool::new(false), 1.2, |p| dm.push(p, &mut pcm)).unwrap_or_else(|e| { eprintln!("{e}"); exit(2) });
        h.join().unwrap();
        let mono = mono24_to_f32(&pcm);
        let sent = sent_at.lock().unwrap().unwrap();
        // -60 dBFS: the idle FM-1 stream is exact digital silence
        match detect_onset(&mono, 0.001) {
            Some(i) => {
                let onset_s = i as f64 / RATE as f64; // time of the onset inside the capture, from its first sample
                let sent_s = sent.duration_since(t_submit).as_secs_f64(); // time the note was sent, same origin
                let ms = (onset_s - sent_s) * 1000.0;
                println!("run {run}: sent at {:.1} ms, onset at {:.1} ms  =>  {:.1} ms", sent_s * 1000.0, onset_s * 1000.0, ms);
                results.push(ms);
            }
            None => println!("run {run}: no sound detected (volume down? sound muted?)"),
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let _ = midi_dev.write(&midi::panic([0]));
    if results.is_empty() {
        println!("RESULT: no measurements");
        return;
    }
    results.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = results[results.len() / 2];
    println!("RESULT: median {:.1} ms  min {:.1}  max {:.1}  spread {:.1} ms  ({} of {} runs)", med, results[0], results[results.len() - 1], results[results.len() - 1] - results[0], results.len(), runs);
    println!("= {:.0} frames at 44.1 kHz. Capture time origin is the submit of the first isochronous transfer, accurate to a couple of ms.", med * 44.1);
}

/// `hdtool probe-cc`: measure what MIDI CCs do to the sound (see `hilldaw_core::probe`). About 3 minutes.
fn probe_cc(args: &[String]) {
    use hilldaw_core::probe::{run, Config, State, Timing};
    use std::collections::BTreeMap;
    let ch = opt(args, "--ch", 2u8).clamp(1, 16) - 1;
    let note_ch = opt(args, "--note-ch", 1u8).clamp(1, 16) - 1;
    let (from, to): (u8, u8) = (opt(args, "--from", 70u8), opt(args, "--to", 93u8));
    let note: u8 = opt(args, "--note", 60);
    let out: String = opt(args, "--out", "/root/probe-cc.txt".to_string());
    if from > to || to > 127 {
        eprintln!("--from must be <= --to, both 0..127");
        exit(1);
    }
    let hold_s = 0.6;
    let total_s = 2.4;
    let d = device();
    let midi_dev = Arc::new(MidiDevice::open(&d).unwrap_or_else(|e| { eprintln!("{e}"); exit(2) }));
    let ccs: Vec<u8> = (from..=to).collect();
    let n_trials = 3 + ccs.len() + 3 + ccs.len();
    println!("Probing CC {from}..{to} on MIDI channel {} with note {note} on channel {}: {n_trials} trials, about {} s.", ch + 1, note_ch + 1, (n_trials as f64 * (total_s + 0.5)) as u32);
    println!("Keep the FM-1 volume UP and leave its controls alone until it says DONE. The CCs are left at 0 afterwards.");

    let mut sent: BTreeMap<u8, u8> = BTreeMap::new();
    let mut trial = |state: &State| -> Vec<f32> {
        // send only what changed since the last trial, then let the FM-1 settle
        for (cc, v) in state {
            if sent.get(cc) != Some(v) {
                let _ = midi_dev.write(&midi::cc(ch, *cc, *v));
                sent.insert(*cc, *v);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        std::thread::sleep(Duration::from_millis(150));
        let mut cp = open_capture(&d);
        let md = midi_dev.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let _ = md.write(&midi::note_on(note_ch, note, 110));
            std::thread::sleep(Duration::from_secs_f64(hold_s));
            let _ = md.write(&midi::note_off(note_ch, note));
        });
        let mut pcm: Vec<u8> = Vec::new();
        let mut dm = Downmixer::default();
        cp.run(&AtomicBool::new(false), total_s, |p| dm.push(p, &mut pcm)).unwrap_or_else(|e| { eprintln!("{e}"); exit(2) });
        h.join().unwrap();
        std::thread::sleep(Duration::from_millis(250));
        mono24_to_f32(&pcm)
    };
    let cfg = Config { ccs: ccs.clone(), low: 0, high: 127, timing: Timing { rate: RATE, hold_s }, baseline_reps: 3 };
    let mut n = 0;
    let result = run(&cfg, &mut trial, &mut |m| {
        n += 1;
        println!("[{n}/{n_trials}] {m}");
    });
    // leave the unit with these CCs at 0 and nothing sounding
    for cc in &ccs {
        let _ = midi_dev.write(&midi::cc(ch, *cc, 0));
    }
    let _ = midi_dev.write(&midi::panic([note_ch]));
    match result {
        Ok(rep) => {
            let text = rep.render();
            println!("\n{text}");
            match std::fs::write(&out, &text) {
                Ok(()) => println!("saved to {out}"),
                Err(e) => println!("(could not save {out}: {e})"),
            }
            println!("DONE");
        }
        Err(e) => {
            eprintln!("probe failed: {e}");
            exit(2);
        }
    }
}

fn status(args: &[String]) {
    use hilldaw_device::{battery, fmt_bytes, fmt_duration, record_seconds_left, storage, POWER_SUPPLY};
    let path = args.first().cloned().unwrap_or_else(|| "/mnt/sdcard".into());
    match battery(Path::new(POWER_SUPPLY)) {
        Some(b) => println!("battery: {}% {}{}{}", b.capacity, b.status, b.voltage_mv.map_or(String::new(), |v| format!(" {v} mV")), if b.is_low(15) { "  LOW" } else { "" }),
        None => println!("battery: not found under {POWER_SUPPLY}"),
    }
    match storage(Path::new(&path)) {
        Some(s) => println!("storage {path}: {} free of {}  =  {} of mono 24-bit recording", fmt_bytes(s.free), fmt_bytes(s.total), fmt_duration(record_seconds_left(s.free, RATE, 3, 1))),
        None => println!("storage {path}: cannot read"),
    }
}

/// `hdtool drums ...`: lets the owner hear their HillBeat presets and kits through the DAW's own drum path.
fn drums(args: &[String]) {
    use hilldaw_core::drums::DrumSection;
    use hilldaw_core::mixer::{Mixer, Track};
    use hilldaw_core::wav::{encode, Audio};
    use hilldaw_drums::host::DirHost;
    use hilldaw_drums::{all_presets, DrumLibrary};

    match args.first().map(String::as_str) {
        Some("list") => {
            use std::io::Write;
            let q = args.get(1).map(|s| s.to_lowercase());
            // `println!` panics if the reader goes away (`| head`); a listing meant for pipes just stops instead
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            for (id, name, style) in all_presets() {
                if q.as_ref().is_none_or(|q| id.contains(q.as_str()) || name.to_lowercase().contains(q.as_str()) || style.to_lowercase().contains(q.as_str())) && writeln!(out, "{id:28} {name:28} {style}").is_err() {
                    break;
                }
            }
        }
        Some("render") => {
            let (Some(id), Some(out)) = (args.get(1), args.get(2)) else {
                eprintln!("usage: hdtool drums render ID OUT.wav [--cycles N] [--bpm B] [--swing S] [--samples DIR]");
                exit(1)
            };
            let cycles: u32 = opt(args, "--cycles", 4u32).clamp(1, 256);
            let bpm: f64 = opt(args, "--bpm", 100.0f64).clamp(40.0, 240.0);
            let swing: f32 = opt(args, "--swing", 0.0f32).clamp(0.0, 0.25);
            let host = match args.iter().position(|a| a == "--samples").and_then(|i| args.get(i + 1)) {
                Some(d) => Some(DirHost::new(d)),
                None => DirHost::detect(),
            };
            match &host {
                Some(h) => println!("samples: {}", h.root().display()),
                None => println!("samples: none found, using the built-in sounds (set HILL_SAMPLES or --samples DIR for your kits)"),
            }
            let boxed: Box<dyn hilldaw_drums::Host> = match host {
                Some(h) => Box::new(h),
                None => Box::new(hilldaw_drums::MemHost::default()),
            };
            let mut lib = DrumLibrary::new(boxed);
            let lp = lib.pattern(id).unwrap_or_else(|e| { eprintln!("{e} (see: hdtool drums list)"); exit(2) });
            println!("preset {id}: kit = {}", lp.kit.as_deref().unwrap_or("built-in sounds"));
            let mut t = hilldaw_core::drums::DrumTrack::new(bpm);
            t.swing = swing;
            t.sections.push(DrumSection { name: id.clone(), pattern: lp.pattern, cycles });
            let frames = t.total_frames(RATE) as usize + 2 * RATE as usize; // two seconds for the last hit to ring out
            let mut mixer = Mixer::new(RATE, 1024);
            let mut track = Track::new("DRUMS");
            track.sources.push(Box::new(t));
            mixer.tracks.push(track);
            let mut audio = Audio { rate: RATE, data: Vec::with_capacity(frames * 2) };
            let mut buf = vec![0.0f32; 2048];
            let mut at = 0usize;
            while at < frames {
                let n = 1024.min(frames - at);
                mixer.render(at as u64, &mut buf[..n * 2]);
                audio.data.extend_from_slice(&buf[..n * 2]);
                at += n;
            }
            let peak = audio.data.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            std::fs::write(out, encode(&audio, 16)).unwrap_or_else(|e| { eprintln!("{out}: {e}"); exit(2) });
            println!("wrote {out}: {:.1} s, {cycles} cycles at {bpm} bpm, swing {swing}, peak {peak:.2}", frames as f64 / RATE as f64);
        }
        _ => println!("usage: hdtool drums list [TEXT] | render ID OUT.wav [--cycles N] [--bpm B] [--swing S] [--samples DIR]"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("status") => status(&args[1..]),
        Some("record") => record(&args[1..]),
        Some("latency") => latency(&args[1..]),
        Some("drums") => drums(&args[1..]),
        Some("probe-cc") => probe_cc(&args[1..]),
        _ => println!("usage: hdtool record SECS OUT.wav [--note N] [--synthetic] | latency [--runs N] [--note N] | probe-cc [--ch 2] [--from 70] [--to 93] [--out FILE] | drums list|render ... | status [PATH]"),
    }
}
