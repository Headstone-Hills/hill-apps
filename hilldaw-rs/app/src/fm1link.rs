//! The app's connection to the FM-1's MIDI port.
//!
//! The UI thread must never wait on USB, so it only drops [`Fm1Msg`]s into a channel; a small thread owns the port, encodes
//! them and writes them. The port is opened **lazily**, on the first message after startup or after a write failed, so the app
//! does not hold the USB interface (usbfs allows one claim per interface) while nobody is using the FM-1 screen. While the unit
//! is unplugged messages are dropped: the app resends what the user has chosen when the unit shows up again.

use hilldaw_usb::dev::{find_fm1, MidiDevice, SYSFS_USB};
use hilldaw_usb::{midi, FM1_PID, FM1_VID};
use std::path::Path;
use std::sync::mpsc::{channel, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Something the app wants the FM-1 to do. Channels are chosen by the link, not by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fm1Msg {
    /// Program Change on the note channel: voice `n + 1`
    Program(u8),
    /// an FX control (CC 70-93) on the FX channel
    FxCc { cc: u8, value: u8 },
    NoteOn(u8),
    NoteOff(u8),
    /// all notes off on both channels
    Panic,
}

/// Where the UI sends FM-1 messages. Tests use a recording implementation.
pub trait Fm1Out: Send {
    fn send(&mut self, m: Fm1Msg);
}

/// MIDI channels, 0-based as on the wire.
#[derive(Clone, Copy, Debug)]
pub struct Channels {
    pub note: u8,
    /// the FM-1's "Effect Ch"; default 2 (index 1)
    pub fx: u8,
}

impl Default for Channels {
    fn default() -> Self {
        Channels { note: 0, fx: hilldaw_core::fm1::fx_map().fx_channel_default.saturating_sub(1) }
    }
}

/// Test note velocity.
pub const NOTE_VEL: u8 = 100;

/// USB-MIDI packets for a message.
pub fn encode(m: Fm1Msg, ch: &Channels) -> Vec<u8> {
    match m {
        Fm1Msg::Program(p) => midi::program_change(ch.note, p.min(127)).to_vec(),
        Fm1Msg::FxCc { cc, value } => midi::cc(ch.fx, cc, value).to_vec(),
        Fm1Msg::NoteOn(n) => midi::note_on(ch.note, n, NOTE_VEL).to_vec(),
        Fm1Msg::NoteOff(n) => midi::note_off(ch.note, n).to_vec(),
        Fm1Msg::Panic => midi::panic([ch.note, ch.fx]),
    }
}

/// An open MIDI port.
pub trait MidiSink {
    fn write(&mut self, packets: &[u8]) -> Result<(), String>;
}

struct UsbSink(MidiDevice);

impl MidiSink for UsbSink {
    fn write(&mut self, packets: &[u8]) -> Result<(), String> {
        self.0.write(packets).map_err(|e| e.to_string())
    }
}

fn open_usb() -> Option<Box<dyn MidiSink>> {
    let info = find_fm1(FM1_VID, FM1_PID, Path::new(SYSFS_USB)).into_iter().next()?;
    MidiDevice::open(&info).ok().map(|d| Box::new(UsbSink(d)) as Box<dyn MidiSink>)
}

pub struct Fm1Link {
    tx: Option<Sender<Fm1Msg>>,
    handle: Option<JoinHandle<()>>,
}

/// Do not retry a failed open more often than this (the sysfs scan and the claim are cheap but not free).
const RETRY: Duration = Duration::from_millis(1000);

impl Fm1Link {
    /// Link over the real USB port.
    pub fn spawn_usb(ch: Channels) -> Fm1Link {
        Self::spawn_with(open_usb, ch)
    }

    /// Link over any port opener (tests give a fake). `open` is called only when a message needs sending and no port is open.
    pub fn spawn_with(mut open: impl FnMut() -> Option<Box<dyn MidiSink>> + Send + 'static, ch: Channels) -> Fm1Link {
        let (tx, rx) = channel::<Fm1Msg>();
        let handle = std::thread::spawn(move || {
            let mut sink: Option<Box<dyn MidiSink>> = None;
            let mut last_try: Option<Instant> = None;
            for m in rx {
                if sink.is_none() && last_try.is_none_or(|t| t.elapsed() >= RETRY) {
                    last_try = Some(Instant::now());
                    sink = open();
                }
                if let Some(s) = sink.as_mut() {
                    if s.write(&encode(m, &ch)).is_err() {
                        sink = None; // unplugged or reset: reopen on the next message
                        last_try = None;
                    }
                }
            }
        });
        Fm1Link { tx: Some(tx), handle: Some(handle) }
    }

    /// Close the channel and wait for the thread to finish what it has queued (tests; the app just drops the link).
    pub fn finish(mut self) {
        self.tx.take();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Fm1Out for Fm1Link {
    fn send(&mut self, m: Fm1Msg) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<Vec<u8>>>>;

    struct Fake {
        log: Log,
        /// fail every write once this many have succeeded
        fail_after: Option<usize>,
    }

    impl MidiSink for Fake {
        fn write(&mut self, p: &[u8]) -> Result<(), String> {
            let mut l = self.log.lock().unwrap();
            if self.fail_after.is_some_and(|n| l.len() >= n) {
                return Err("unplugged".into());
            }
            l.push(p.to_vec());
            Ok(())
        }
    }

    #[test]
    fn messages_are_encoded_on_the_right_channels() {
        let ch = Channels::default();
        assert_eq!((ch.note, ch.fx), (0, 1), "notes on 1, FX on 2");
        // USB-MIDI packet: [cable/CIN, status, data1, data2]
        assert_eq!(encode(Fm1Msg::Program(5), &ch), vec![0x0C, 0xC0, 5, 0]);
        assert_eq!(encode(Fm1Msg::Program(200), &ch)[2], 127, "program is clamped");
        assert_eq!(encode(Fm1Msg::FxCc { cc: 72, value: 40 }, &ch), vec![0x0B, 0xB1, 72, 40]);
        assert_eq!(encode(Fm1Msg::NoteOn(60), &ch), vec![0x09, 0x90, 60, NOTE_VEL]);
        assert_eq!(encode(Fm1Msg::NoteOff(60), &ch), vec![0x08, 0x80, 60, 0]);
        let panic = encode(Fm1Msg::Panic, &ch);
        assert!(panic.chunks(4).any(|p| p[1] == 0xB0 && p[2] == 123) && panic.chunks(4).any(|p| p[1] == 0xB1 && p[2] == 123), "both channels");
    }

    #[test]
    fn the_port_is_opened_only_when_something_is_sent_and_every_message_goes_out_in_order() {
        let log: Log = Arc::default();
        let opens = Arc::new(Mutex::new(0));
        let (l2, o2) = (log.clone(), opens.clone());
        let mut link = Fm1Link::spawn_with(move || { *o2.lock().unwrap() += 1; Some(Box::new(Fake { log: l2.clone(), fail_after: None })) }, Channels::default());
        assert_eq!(*opens.lock().unwrap(), 0, "nothing claimed before the first message");
        link.send(Fm1Msg::Program(7));
        link.send(Fm1Msg::FxCc { cc: 70, value: 1 });
        link.send(Fm1Msg::NoteOn(60));
        link.finish();
        assert_eq!(*opens.lock().unwrap(), 1);
        let l = log.lock().unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!((l[0][1], l[1][1], l[2][1]), (0xC0, 0xB1, 0x90));
    }

    #[test]
    fn messages_are_dropped_while_no_device_is_there_and_a_failed_write_reopens() {
        // no device at all: the sender never blocks or panics
        let mut link = Fm1Link::spawn_with(|| None, Channels::default());
        for i in 0..50 {
            link.send(Fm1Msg::Program(i));
        }
        link.finish();

        // a port that dies after one write: the next message opens a fresh one
        let log: Log = Arc::default();
        let opens = Arc::new(Mutex::new(0usize));
        let (l2, o2) = (log.clone(), opens.clone());
        let mut link = Fm1Link::spawn_with(
            move || {
                let mut n = o2.lock().unwrap();
                *n += 1;
                // the first port fails after its first write; the second one is healthy
                Some(Box::new(Fake { log: l2.clone(), fail_after: if *n == 1 { Some(1) } else { None } }))
            },
            Channels::default(),
        );
        link.send(Fm1Msg::Program(1)); // ok on port 1
        link.send(Fm1Msg::Program(2)); // port 1 fails: dropped, port discarded
        std::thread::sleep(RETRY + Duration::from_millis(100)); // past the retry limit
        link.send(Fm1Msg::Program(3)); // reopens, ok
        link.finish();
        assert_eq!(*opens.lock().unwrap(), 2);
        let sent: Vec<u8> = log.lock().unwrap().iter().map(|p| p[2]).collect();
        assert_eq!(sent, vec![1, 3], "the message that hit the dead port is lost, later ones arrive");
    }
}
