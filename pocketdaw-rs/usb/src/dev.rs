//! Device discovery (sysfs, by VID/PID, never a hardcoded bus) and the MIDI endpoint handle.

use crate::sys;
use std::ffi::CString;
use std::fs;
use std::path::{Path, PathBuf};

pub const SYSFS_USB: &str = "/sys/bus/usb/devices";
const DEFAULT_MIDI_IFACE: u8 = 4;
const DEFAULT_EP_OUT: u8 = 0x04;
const DEFAULT_EP_IN: u8 = 0x84;

#[derive(Debug)]
pub enum UsbError {
    /// ENODEV / EPIPE-style: the device went away. The caller should re-scan.
    Disconnected(String),
    Io(String),
}

impl std::fmt::Display for UsbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UsbError::Disconnected(s) => write!(f, "disconnected: {s}"),
            UsbError::Io(s) => write!(f, "{s}"),
        }
    }
}
impl std::error::Error for UsbError {}

pub fn os_error(what: &str) -> UsbError {
    let e = std::io::Error::last_os_error();
    let msg = format!("{what}: {e}");
    match e.raw_os_error() {
        Some(libc::ENODEV) | Some(libc::ESHUTDOWN) | Some(libc::EPROTO) => UsbError::Disconnected(msg),
        _ => UsbError::Io(msg),
    }
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub sysname: String,
    pub busnum: u32,
    pub devnum: u32,
    pub product: String,
    pub midi_iface: u8,
    pub ep_out: u8,
    pub ep_in: u8,
    pub sysdir: PathBuf,
}

impl DeviceInfo {
    pub fn path(&self) -> String {
        format!("/dev/bus/usb/{:03}/{:03}", self.busnum, self.devnum)
    }
    pub fn descriptors(&self) -> std::io::Result<Vec<u8>> {
        fs::read(self.sysdir.join("descriptors"))
    }
}

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// (interface number, bulk OUT, bulk IN) of the MIDI-streaming interface (class 01, subclass 03).
fn find_midi_interface(devdir: &Path, sysname: &str) -> (u8, u8, u8) {
    if let Ok(rd) = fs::read_dir(devdir) {
        let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for n in names.iter().filter(|n| n.starts_with(&format!("{sysname}:"))) {
            let d = devdir.join(n);
            if read_trim(&d.join("bInterfaceClass")).as_deref() != Some("01")
                || read_trim(&d.join("bInterfaceSubClass")).as_deref() != Some("03")
            {
                continue;
            }
            let num = read_trim(&d.join("bInterfaceNumber")).and_then(|s| u8::from_str_radix(&s, 16).ok());
            let (mut out_ep, mut in_ep) = (None, DEFAULT_EP_IN);
            if let Ok(eps) = fs::read_dir(&d) {
                for e in eps.filter_map(|e| e.ok()) {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if !name.starts_with("ep_") {
                        continue;
                    }
                    let addr = read_trim(&e.path().join("bEndpointAddress")).and_then(|s| u8::from_str_radix(&s, 16).ok());
                    match (addr, read_trim(&e.path().join("direction")).as_deref()) {
                        (Some(a), Some("out")) => out_ep = Some(a),
                        (Some(a), Some(_)) => in_ep = a,
                        _ => {}
                    }
                }
            }
            if let (Some(n), Some(o)) = (num, out_ep) {
                return (n, o, in_ep);
            }
        }
    }
    (DEFAULT_MIDI_IFACE, DEFAULT_EP_OUT, DEFAULT_EP_IN)
}

pub fn find_devices(vid: u16, pid: u16, sysfs: &Path) -> Vec<DeviceInfo> {
    let mut found = Vec::new();
    let Ok(rd) = fs::read_dir(sysfs) else { return found };
    let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    for name in names {
        let d = sysfs.join(&name);
        let (Some(v), Some(p)) = (read_trim(&d.join("idVendor")), read_trim(&d.join("idProduct"))) else { continue };
        if u16::from_str_radix(&v, 16).ok() != Some(vid) || u16::from_str_radix(&p, 16).ok() != Some(pid) {
            continue;
        }
        let (Some(bus), Some(dev)) = (
            read_trim(&d.join("busnum")).and_then(|s| s.parse().ok()),
            read_trim(&d.join("devnum")).and_then(|s| s.parse().ok()),
        ) else {
            continue;
        };
        let (iface, out, inn) = find_midi_interface(&d, &name);
        found.push(DeviceInfo {
            product: read_trim(&d.join("product")).unwrap_or_else(|| "FM-1".into()),
            sysname: name,
            busnum: bus,
            devnum: dev,
            midi_iface: iface,
            ep_out: out,
            ep_in: inn,
            sysdir: d,
        });
    }
    found
}

/// Open usbfs node.
pub(crate) fn open_node(info: &DeviceInfo) -> Result<i32, UsbError> {
    let c = CString::new(info.path()).map_err(|e| UsbError::Io(e.to_string()))?;
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return Err(os_error(&format!("open {}", info.path())));
    }
    Ok(fd)
}

pub(crate) fn claim(fd: i32, iface: u8) -> Result<(), UsbError> {
    let mut i: u32 = iface as u32;
    if unsafe { sys::ioctl(fd, sys::CLAIMINTERFACE, &mut i) } < 0 {
        return Err(os_error(&format!("claim interface {iface}")));
    }
    Ok(())
}

pub(crate) fn release(fd: i32, iface: u8) {
    let mut i: u32 = iface as u32;
    unsafe { sys::ioctl(fd, sys::RELEASEINTERFACE, &mut i) };
}

/// The MIDI-streaming interface of the FM-1: write packets to the OUT endpoint, read the IN endpoint.
/// Releases the interface and closes the node on drop.
pub struct MidiDevice {
    fd: i32,
    iface: u8,
    ep_out: u8,
    ep_in: u8,
    pub timeout_ms: u32,
}

impl MidiDevice {
    pub fn open(info: &DeviceInfo) -> Result<Self, UsbError> {
        let fd = open_node(info)?;
        if let Err(e) = claim(fd, info.midi_iface) {
            unsafe { libc::close(fd) };
            return Err(e);
        }
        Ok(MidiDevice { fd, iface: info.midi_iface, ep_out: info.ep_out, ep_in: info.ep_in, timeout_ms: 100 })
    }

    fn bulk(&self, ep: u8, buf: &mut [u8], timeout_ms: u32) -> Result<usize, UsbError> {
        let mut bt = sys::BulkTransfer { ep: ep as u32, len: buf.len() as u32, timeout: timeout_ms, data: buf.as_mut_ptr() as *mut _ };
        let r = unsafe { sys::ioctl(self.fd, sys::BULK, &mut bt) };
        if r < 0 {
            if sys::errno() == libc::ETIMEDOUT {
                return Ok(0);
            }
            return Err(os_error("bulk transfer"));
        }
        Ok(r as usize)
    }

    /// Write whole 4-byte packets (several may share one bulk write).
    pub fn write(&self, data: &[u8]) -> Result<(), UsbError> {
        let n = data.len() - data.len() % 4;
        if n == 0 {
            return Ok(());
        }
        let mut buf = data[..n].to_vec();
        let sent = self.bulk(self.ep_out, &mut buf, self.timeout_ms)?;
        if sent != n {
            return Err(UsbError::Io(format!("short write {sent}/{n}")));
        }
        Ok(())
    }

    /// Read up to 64 bytes from the IN endpoint; an empty Vec means the read timed out.
    pub fn read(&self, timeout_ms: u32) -> Result<Vec<u8>, UsbError> {
        let mut buf = vec![0u8; 64];
        let n = self.bulk(self.ep_in, &mut buf, timeout_ms)?;
        buf.truncate(n);
        Ok(buf)
    }
}

impl Drop for MidiDevice {
    fn drop(&mut self) {
        release(self.fd, self.iface);
        unsafe { libc::close(self.fd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(root: &Path, rel: &str, val: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, format!("{val}\n")).unwrap();
    }

    #[test]
    fn finds_by_vid_pid_and_uses_devnum_from_sysfs() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        for (k, v) in [("idVendor", "4c4a"), ("idProduct", "c755"), ("busnum", "8"), ("devnum", "7"), ("product", "FM-1")] {
            w(r, &format!("8-1/{k}"), v);
        }
        let i = "8-1/8-1:1.4";
        w(r, &format!("{i}/bInterfaceClass"), "01");
        w(r, &format!("{i}/bInterfaceSubClass"), "03");
        w(r, &format!("{i}/bInterfaceNumber"), "04");
        w(r, &format!("{i}/ep_04/direction"), "out");
        w(r, &format!("{i}/ep_04/bEndpointAddress"), "04");
        w(r, &format!("{i}/ep_84/direction"), "in");
        w(r, &format!("{i}/ep_84/bEndpointAddress"), "84");
        w(r, "1-1/idVendor", "dead");
        w(r, "1-1/idProduct", "beef");
        let f = find_devices(0x4C4A, 0xC755, r);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].path(), "/dev/bus/usb/008/007");
        assert_eq!((f[0].midi_iface, f[0].ep_out, f[0].ep_in), (4, 0x04, 0x84));
    }

    #[test]
    fn missing_sysfs_is_empty() {
        assert!(find_devices(0x4C4A, 0xC755, Path::new("/nonexistent")).is_empty());
    }
}
