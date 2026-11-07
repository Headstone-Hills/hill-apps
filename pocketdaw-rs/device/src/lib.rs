//! Battery and storage indicators (brief feature 16). Reads sysfs and statvfs; no kernel driver needed.
//! On the RG35XXSP the battery is `axp2202-battery` under `/sys/class/power_supply` (there is no `current_now`).

use std::ffi::CString;
use std::fs;
use std::path::Path;

pub const POWER_SUPPLY: &str = "/sys/class/power_supply";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battery {
    /// 0..=100
    pub capacity: u8,
    /// "Charging", "Discharging", "Full", "Not charging" ... as the kernel reports it
    pub status: String,
    pub voltage_mv: Option<u32>,
}

impl Battery {
    pub fn is_charging(&self) -> bool {
        self.status.eq_ignore_ascii_case("charging") || self.status.eq_ignore_ascii_case("full")
    }
    /// Warn when low and not on power (a USB device plus a long recording drains the SP quickly).
    pub fn is_low(&self, threshold_percent: u8) -> bool {
        self.capacity <= threshold_percent && !self.is_charging()
    }
}

fn read(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// The first supply whose `type` is `Battery`.
pub fn battery(root: &Path) -> Option<Battery> {
    let mut names: Vec<_> = fs::read_dir(root).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    names.sort();
    for d in names {
        if read(&d.join("type")).as_deref() != Some("Battery") {
            continue;
        }
        let capacity = read(&d.join("capacity")).and_then(|s| s.parse::<u32>().ok())?.min(100) as u8;
        return Some(Battery {
            capacity,
            status: read(&d.join("status")).unwrap_or_else(|| "Unknown".into()),
            voltage_mv: read(&d.join("voltage_now")).and_then(|s| s.parse::<u64>().ok()).map(|uv| (uv / 1000) as u32),
        });
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Storage {
    pub total: u64,
    /// available to an ordinary user
    pub free: u64,
}

pub fn storage(path: &Path) -> Option<Storage> {
    let c = CString::new(path.to_str()?).ok()?;
    let mut v: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut v) } != 0 {
        return None;
    }
    let fr = v.f_frsize as u64;
    Some(Storage { total: v.f_blocks as u64 * fr, free: v.f_bavail as u64 * fr })
}

/// Seconds of recording that fit in `free_bytes`. Mono 24-bit at 44.1 kHz is 132,300 bytes per second.
pub fn record_seconds_left(free_bytes: u64, rate: u32, bytes_per_sample: u32, channels: u32) -> u64 {
    free_bytes / (rate as u64 * bytes_per_sample as u64 * channels as u64).max(1)
}

/// "12m 05s", "3h 20m", "210h".
pub fn fmt_duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h >= 100 {
        format!("{h}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m {s:02}s")
    }
}

pub fn fmt_bytes(b: u64) -> String {
    const G: f64 = 1024.0 * 1024.0 * 1024.0;
    if b as f64 >= G { format!("{:.1} GB", b as f64 / G) } else { format!("{:.0} MB", b as f64 / 1048576.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(root: &Path, rel: &str, v: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, format!("{v}\n")).unwrap();
    }

    #[test]
    fn finds_the_battery_among_other_supplies_like_the_sp_layout() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        w(r, "axp2202-usb/type", "USB");
        w(r, "axp2202-usb/online", "0");
        w(r, "axp2202-battery/type", "Battery");
        w(r, "axp2202-battery/capacity", "39");
        w(r, "axp2202-battery/status", "Discharging");
        w(r, "axp2202-battery/voltage_now", "3712000");
        let b = battery(r).unwrap();
        assert_eq!((b.capacity, b.status.as_str(), b.voltage_mv), (39, "Discharging", Some(3712)));
        assert!(!b.is_low(30) && b.is_low(40));
        assert!(!b.is_charging());
    }

    #[test]
    fn charging_batteries_are_never_low_and_bad_data_is_tolerated() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        w(r, "bat/type", "Battery");
        w(r, "bat/capacity", "180"); // nonsense gets clamped
        w(r, "bat/status", "Charging");
        let b = battery(r).unwrap();
        assert_eq!((b.capacity, b.voltage_mv), (100, None));
        let low = Battery { capacity: 3, status: "Charging".into(), voltage_mv: None };
        assert!(!low.is_low(15));
        assert!(battery(Path::new("/nonexistent")).is_none());
        let e = tempfile::tempdir().unwrap();
        assert!(battery(e.path()).is_none());
        w(e.path(), "bat/type", "Battery"); // no capacity file
        assert!(battery(e.path()).is_none());
    }

    #[test]
    fn storage_reports_sane_numbers_and_missing_paths_fail() {
        let d = tempfile::tempdir().unwrap();
        let s = storage(d.path()).unwrap();
        assert!(s.total > 0 && s.free <= s.total);
        assert!(storage(Path::new("/nonexistent/x")).is_none());
    }

    #[test]
    fn recording_time_and_formatting() {
        // the SP's SD card had ~100 GB free: mono 24-bit at 44.1 kHz = 132,300 B/s
        let free = 100u64 * 1024 * 1024 * 1024;
        let secs = record_seconds_left(free, 44100, 3, 1);
        assert_eq!(secs, free / 132_300);
        assert_eq!(fmt_duration(secs), format!("{}h", secs / 3600));
        assert_eq!(fmt_duration(125), "2m 05s");
        assert_eq!(fmt_duration(3 * 3600 + 20 * 60 + 9), "3h 20m");
        assert_eq!(fmt_duration(0), "0m 00s");
        assert_eq!(record_seconds_left(1000, 44100, 3, 0), 1000, "zero channels is treated as one, not a divide by zero");
        assert_eq!(fmt_bytes(100 * 1024 * 1024 * 1024), "100.0 GB");
        assert_eq!(fmt_bytes(512 * 1024 * 1024), "512 MB");
    }
}
