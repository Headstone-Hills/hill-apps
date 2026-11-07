//! What the status bar shows: battery, free space, FM-1 present. Gathered by a background thread so the UI and
//! audio thread never wait on sysfs; the app only copies the latest values.

use hilldaw_device::{battery, storage, POWER_SUPPLY};
use hilldaw_usb::dev::{find_devices, SYSFS_USB};
use hilldaw_usb::{FM1_PID, FM1_VID};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub battery_pct: Option<u8>,
    pub charging: bool,
    pub battery_low: bool,
    pub free_bytes: Option<u64>,
    /// the FM-1 is on the USB bus (not the same as "capturing from it")
    pub fm1: bool,
}

pub type StatusHandle = Arc<Mutex<Status>>;

pub fn poll_once(storage_path: &Path) -> Status {
    let b = battery(Path::new(POWER_SUPPLY));
    Status {
        battery_pct: b.as_ref().map(|b| b.capacity),
        charging: b.as_ref().is_some_and(|b| b.is_charging()),
        battery_low: b.as_ref().is_some_and(|b| b.is_low(15)),
        free_bytes: storage(storage_path).map(|s| s.free),
        fm1: !find_devices(FM1_VID, FM1_PID, Path::new(SYSFS_USB)).is_empty(),
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
