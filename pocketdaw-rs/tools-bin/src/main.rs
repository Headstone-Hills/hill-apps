//! pdtool: hardware-in-the-loop tests for the recording path (run on the SP over SSH).
//!
//!   pdtool record SECS OUT.wav [--note N]      capture -> mono 24-bit -> bounded queue -> SD card, with overrun counts
//!   pdtool latency [--runs N] [--note N]       MIDI note-on -> first audible sample in the capture, median/min/max
//!   pdtool record SECS OUT.wav --synthetic     no FM-1: a real-time tone source (to test the writer on any machine)

use pocketdaw_core::record::{detect_onset, mono24_to_f32, Downmixer, StreamingWav};
use pocketdaw_usb::desc::{parse_audio_streams, pick_capture};
use pocketdaw_usb::dev::{find_devices, DeviceInfo, MidiDevice, SYSFS_USB};
use pocketdaw_usb::iso::IsoCapture;
use pocketdaw_usb::{midi, wav, FM1_PID, FM1_VID};
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("record") => record(&args[1..]),
        Some("latency") => latency(&args[1..]),
        _ => println!("usage: pdtool record SECS OUT.wav [--note N] [--synthetic] | latency [--runs N] [--note N]"),
    }
}
