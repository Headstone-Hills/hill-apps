//! What the status bar shows: battery, free space, FM-1 present. Gathered by a background thread so the UI and
//! audio thread never wait on sysfs; the app only copies the latest values.

use hilldaw_device::{battery, storage, POWER_SUPPLY};
use hilldaw_usb::audio::{list_inputs, list_outputs};
use hilldaw_usb::dev::{find_fm1, SYSFS_USB};
use hilldaw_usb::{FM1_PID, FM1_VID};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A USB audio input that is attached, as the screen needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct InputSummary {
    /// `vid:pid`
    pub id: String,
    pub name: String,
    /// `2ch 24-bit 44.1 kHz`
    pub summary: String,
    /// the rate it is read at; anything but 44100 is resampled
    pub rate: u32,
}

pub use hilldaw_usb::audio::FM1_ID;

/// Which input is in use: the chosen one if there is one (`None` when it is unplugged), otherwise automatic: the FM-1 when attached,
/// else the first. The recorder makes the same choice from the same rules.
pub fn effective_input<'a>(list: &'a [InputSummary], selected: &Option<String>) -> Option<&'a InputSummary> {
    match selected {
        Some(id) => list.iter().find(|i| i.id == *id),
        None => list.iter().find(|i| i.id == FM1_ID).or_else(|| list.first()),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub battery_pct: Option<u8>,
    pub charging: bool,
    pub battery_low: bool,
    pub free_bytes: Option<u64>,
    /// the FM-1 is on the USB bus (its MIDI side: the FM-1 screen)
    pub fm1: bool,
    /// every attached USB audio input
    pub inputs: Vec<InputSummary>,
    /// every attached USB audio output (same shape: id, name, format, rate)
    pub outputs: Vec<InputSummary>,
}

pub type StatusHandle = Arc<Mutex<Status>>;

pub fn poll_once(storage_path: &Path) -> Status {
    let b = battery(Path::new(POWER_SUPPLY));
    Status {
        battery_pct: b.as_ref().map(|b| b.capacity),
        charging: b.as_ref().is_some_and(|b| b.is_charging()),
        battery_low: b.as_ref().is_some_and(|b| b.is_low(15)),
        free_bytes: storage(storage_path).map(|s| s.free),
        fm1: !find_fm1(FM1_VID, FM1_PID, Path::new(SYSFS_USB)).is_empty(),
        inputs: list_inputs(Path::new(SYSFS_USB)).into_iter().map(|i| InputSummary { summary: i.summary(), rate: i.rate, id: i.id, name: i.name }).collect(),
        outputs: list_outputs(Path::new(SYSFS_USB)).into_iter().map(|o| InputSummary { summary: o.summary(), rate: o.rate, id: o.id, name: o.name }).collect(),
    }
}

/// Where recordings will go, for the free-space readout: the SD card on the SP, else the current folder.
pub fn default_storage_path() -> PathBuf {
    let sd = PathBuf::from("/mnt/sdcard");
    if sd.is_dir() { sd } else { PathBuf::from(".") }
}

pub fn spawn_poller(handle: StatusHandle, storage_path: PathBuf) {
    std::thread::spawn(move || loop {
        let s = poll_once(&storage_path);
        if let Ok(mut g) = handle.lock() {
            *g = s;
        }
        std::thread::sleep(Duration::from_secs(2));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i(id: &str) -> InputSummary {
        InputSummary { id: id.into(), name: id.into(), summary: String::new(), rate: 44100 }
    }

    #[test]
    fn the_effective_input_follows_the_same_rules_as_the_recorder() {
        let both = vec![i("1234:00ab"), i(FM1_ID)];
        assert_eq!(effective_input(&both, &None).unwrap().id, FM1_ID);
        assert_eq!(effective_input(&both[..1], &None).unwrap().id, "1234:00ab");
        assert_eq!(effective_input(&both, &Some("1234:00ab".into())).unwrap().id, "1234:00ab");
        assert!(effective_input(&both, &Some("dead:beef".into())).is_none());
        assert!(effective_input(&[], &None).is_none());
    }
}
