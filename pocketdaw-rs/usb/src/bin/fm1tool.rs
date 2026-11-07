//! fm1tool: hardware diagnostics for the FM-1 (run over SSH on the SP, no UI).
//!
//!   fm1tool list
//!   fm1tool descriptors [--file F] [--save F]
//!   fm1tool note N [--vel V] [--len SECS] [--ch C]
//!   fm1tool pc N [--ch C]        fm1tool cc N V [--ch C]        fm1tool panic
//!   fm1tool midi-in [--seconds S]
//!   fm1tool capture SECONDS OUT.wav [--note N] [--frames F] [--urbs U]
//!   fm1tool wavcheck FILE.wav

use pocketdaw_usb::desc::{parse_audio_streams, pick_capture};
use pocketdaw_usb::dev::{find_devices, DeviceInfo, MidiDevice, SYSFS_USB};
use pocketdaw_usb::iso::IsoCapture;
use pocketdaw_usb::{midi, wav, FM1_PID, FM1_VID};
use std::path::Path;
use std::process::exit;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn opt<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}
fn opt_s(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn need_device() -> DeviceInfo {
    match find_devices(FM1_VID, FM1_PID, Path::new(SYSFS_USB)).into_iter().next() {
        Some(d) => d,
        None => {
            eprintln!("FM-1 ({:04x}:{:04x}) not found. Plugged into the USB-C port? Try `lsusb`.", FM1_VID, FM1_PID);
            exit(2)
        }
    }
}

fn open_midi() -> MidiDevice {
    MidiDevice::open(&need_device()).unwrap_or_else(|e| {
        eprintln!("cannot open FM-1 MIDI: {e}");
        exit(2)
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    let ch = opt(rest, "--ch", 1u8).saturating_sub(1);
    match cmd {
        "list" => {
            let devs = find_devices(FM1_VID, FM1_PID, Path::new(SYSFS_USB));
            if devs.is_empty() {
                println!("no FM-1 found");
                exit(1);
            }
            for d in devs {
                println!("{}  {}  product={:?}", d.sysname, d.path(), d.product);
                println!("  MIDI interface {}: bulk-OUT 0x{:02x}, bulk-IN 0x{:02x}", d.midi_iface, d.ep_out, d.ep_in);
                for s in parse_audio_streams(&d.descriptors().unwrap_or_default()) {
                    println!("  audio <{}>", s.describe());
                }
            }
        }
        "descriptors" => {
            let blob = match opt_s(rest, "--file") {
                Some(f) => std::fs::read(&f).unwrap_or_else(|e| { eprintln!("{f}: {e}"); exit(2) }),
                None => {
                    let b = need_device().descriptors().unwrap_or_else(|e| { eprintln!("{e}"); exit(2) });
                    if let Some(p) = opt_s(rest, "--save") {
                        std::fs::write(&p, &b).unwrap();
                        println!("saved {} bytes to {}", b.len(), p);
                    }
                    b
                }
            };
            println!("descriptor blob: {} bytes", blob.len());
            let s = parse_audio_streams(&blob);
            for x in &s {
                println!("  <{}>", x.describe());
            }
            match pick_capture(&s, 44100) {
                Some(c) => println!("capture choice: <{}>", c.describe()),
                None => println!("capture choice: none"),
            }
        }
        "note" => {
            let n: u8 = rest.first().and_then(|v| v.parse().ok()).unwrap_or(60);
            let m = open_midi();
            let _ = m.write(&midi::note_on(ch, n, opt(rest, "--vel", 100)));
            std::thread::sleep(Duration::from_secs_f64(opt(rest, "--len", 0.8)));
            let _ = m.write(&midi::note_off(ch, n));
            println!("note {n} sent");
        }
        "pc" => {
            let n: u8 = rest.first().and_then(|v| v.parse().ok()).unwrap_or(0);
            open_midi().write(&midi::program_change(ch, n)).unwrap_or_else(|e| { eprintln!("{e}"); exit(1) });
            println!("program change {n} sent");
        }
        "cc" => {
            let n: u8 = rest.first().and_then(|v| v.parse().ok()).unwrap_or(0);
            let v: u8 = rest.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            open_midi().write(&midi::cc(ch, n, v)).unwrap_or_else(|e| { eprintln!("{e}"); exit(1) });
            println!("CC {n} = {v} sent");
        }
        "panic" => {
            let m = open_midi();
            m.write(&midi::panic(0..16)).unwrap_or_else(|e| { eprintln!("{e}"); exit(1) });
            for c in 0..16 {
                let _ = m.write(&midi::pitch_bend(c, 8192));
            }
            println!("panic sent (CC123+CC120 on all channels, bend centred)");
        }
        "midi-in" => {
            let secs: u64 = opt(rest, "--seconds", 10);
            let d = need_device();
            let m = MidiDevice::open(&d).unwrap_or_else(|e| { eprintln!("{e}"); exit(2) });
            println!("listening {secs}s on endpoint 0x{:02x} - press FM-1 keys / turn knobs ...", d.ep_in);
            let (end, mut reads, mut timeouts, mut errs, mut got) = (Instant::now() + Duration::from_secs(secs), 0, 0, 0, 0);
            while Instant::now() < end {
                match m.read(100) {
                    Ok(data) => {
                        reads += 1;
                        if data.is_empty() {
                            timeouts += 1;
                        }
                        for e in midi::decode(&data) {
                            got += 1;
                            println!("{}", midi::describe(&e));
                        }
                    }
                    Err(e) => {
                        errs += 1;
                        eprintln!("read error: {e}");
                        std::thread::sleep(Duration::from_millis(200));
                    }
                }
            }
            println!("reads={reads} (timeouts={timeouts}, errors={errs}), MIDI events={got}");
            println!("RESULT: {}", if got > 0 { "FM-1 TRANSMITS MIDI" } else if errs > 0 { "read errors - endpoint not readable" } else { "endpoint readable but the FM-1 sent nothing" });
        }
        "capture" => {
            let secs: f64 = rest.first().and_then(|v| v.parse().ok()).unwrap_or(4.0);
            let out = rest.get(1).cloned().unwrap_or_else(|| "capture.wav".into());
            let d = need_device();
            let streams = parse_audio_streams(&d.descriptors().unwrap_or_default());
            let cap = pick_capture(&streams, 44100).unwrap_or_else(|| { eprintln!("no capture endpoint in descriptors"); exit(2) });
            println!("capturing from {}: iface {} alt {} ep 0x{:02x} maxpkt {} ({}ch {}-bit {:?} Hz)", d.path(), cap.iface, cap.alt, cap.ep, cap.max_packet, cap.channels, cap.bits, cap.rates);
            let rate = if cap.rates.contains(&44100) { 44100 } else { cap.rates[0] };
            let mut cp = IsoCapture::open(&d, cap, opt(rest, "--frames", 16usize), opt(rest, "--urbs", 8usize)).unwrap_or_else(|e| { eprintln!("{e}"); exit(2) });
            if let Some(note) = opt_s(rest, "--note").and_then(|v| v.parse::<u8>().ok()) {
                let dd = d.clone();
                std::thread::spawn(move || {
                    if let Ok(m) = MidiDevice::open(&dd) {
                        std::thread::sleep(Duration::from_millis(500));
                        let _ = m.write(&midi::note_on(0, note, 110));
                        std::thread::sleep(Duration::from_secs_f64((secs / 2.0).max(0.3)));
                        let _ = m.write(&midi::note_off(0, note));
                        let _ = m.write(&midi::panic([0]));
                    }
                });
            }
            let mut pcm: Vec<u8> = Vec::with_capacity((secs * rate as f64) as usize * cap.frame_bytes());
            let t0 = Instant::now();
            let st = cp.run(&AtomicBool::new(false), secs, |p| pcm.extend_from_slice(p)).unwrap_or_else(|e| { eprintln!("{e}"); exit(2) });
            let wall = t0.elapsed().as_secs_f64();
            println!("stats: {}  wall={:.2}s", st.summary(), wall);
            wav::write_wav(std::fs::File::create(&out).unwrap_or_else(|e| { eprintln!("{out}: {e}"); exit(2) }), cap.channels as u16, cap.subframe as u16, rate, &pcm).unwrap();
            println!("wrote {out}");
            println!("{}", wav::analyze(&pcm, cap.channels as usize, cap.subframe as usize, rate));
        }
        "wavcheck" => {
            let f = rest.first().cloned().unwrap_or_default();
            let b = std::fs::read(&f).unwrap_or_else(|e| { eprintln!("{f}: {e}"); exit(2) });
            let (c, bps, rate, data) = wav::read_wav(&b).unwrap_or_else(|e| { eprintln!("{f}: {e}"); exit(2) });
            println!("{}", wav::analyze(data, c as usize, bps as usize, rate));
        }
        _ => {
            println!("usage: fm1tool list | descriptors [--file F] [--save F] | note N | pc N | cc N V | panic | midi-in [--seconds S] | capture SECS OUT.wav [--note N] | wavcheck FILE.wav");
        }
    }
}
