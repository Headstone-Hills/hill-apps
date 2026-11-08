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

fn device_of(d: &Path, name: String) -> Option<DeviceInfo> {
    let bus = read_trim(&d.join("busnum")).and_then(|s| s.parse().ok())?;
    let dev = read_trim(&d.join("devnum")).and_then(|s| s.parse().ok())?;
    let (iface, out, inn) = find_midi_interface(d, &name);
    Some(DeviceInfo {
        product: read_trim(&d.join("product")).unwrap_or_else(|| "FM-1".into()),
        sysname: name,
        busnum: bus,
        devnum: dev,
        midi_iface: iface,
        ep_out: out,
        ep_in: inn,
        sysdir: d.to_path_buf(),
    })
}

/// Whether a device folder has a MIDI-streaming interface (class 01, subclass 03).
fn has_midi_interface(devdir: &Path, sysname: &str) -> bool {
    let Ok(rd) = fs::read_dir(devdir) else { return false };
    rd.filter_map(|e| e.ok()).any(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        n.starts_with(&format!("{sysname}:"))
            && read_trim(&e.path().join("bInterfaceClass")).as_deref() == Some("01")
            && read_trim(&e.path().join("bInterfaceSubClass")).as_deref() == Some("03")
    })
}

/// The FM-1 by its exact id; failing that (custom firmware can change the id) any attached device with a MIDI-streaming interface.
pub fn find_fm1(vid: u16, pid: u16, sysfs: &Path) -> Vec<DeviceInfo> {
    let exact = find_devices(vid, pid, sysfs);
    if !exact.is_empty() {
        return exact;
    }
    let Ok(rd) = fs::read_dir(sysfs) else { return Vec::new() };
    let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let d = sysfs.join(&name);
            read_trim(&d.join("idVendor"))?;
            if !has_midi_interface(&d, &name) {
                return None;
            }
            device_of(&d, name)
        })
        .collect()
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

/// USB Audio Class 1.0: set the sampling frequency of an isochronous endpoint (SET_CUR, SAMPLING_FREQ_CONTROL). A device that offers
/// several rates stays at its default until told; one with a single rate ignores or stalls the request, which callers can ignore.
pub(crate) fn set_sampling_rate(fd: i32, ep: u8, rate: u32) -> Result<(), UsbError> {
    let mut data = [rate as u8, (rate >> 8) as u8, (rate >> 16) as u8];
    let mut t = sys::CtrlTransfer { request_type: 0x22, request: 0x01, value: 0x0100, index: ep as u16, length: 3, timeout: 1000, data: data.as_mut_ptr() as *mut std::os::raw::c_void };
    if unsafe { sys::ioctl(fd, sys::CONTROL, &mut t) } < 0 {
        return Err(os_error("set sampling rate"));
    }
    Ok(())
}

/// Diagnostics: a class/standard control request on a claimed interface. Returns the bytes transferred (IN: `data` is filled).
pub fn class_request(info: &DeviceInfo, iface: u8, request_type: u8, request: u8, value: u16, index: u16, data: &mut [u8]) -> Result<usize, UsbError> {
    let fd = open_node(info)?;
    if let Err(e) = claim(fd, iface) {
        unsafe { libc::close(fd) };
        return Err(e);
    }
    let mut t = sys::CtrlTransfer { request_type, request, value, index, length: data.len() as u16, timeout: 1000, data: data.as_mut_ptr() as *mut std::os::raw::c_void };
    let n = unsafe { sys::ioctl(fd, sys::CONTROL, &mut t) };
    let r = if n < 0 { Err(os_error("control request")) } else { Ok(n as usize) };
    release(fd, iface);
    unsafe { libc::close(fd) };
    r
}

/// Diagnostics: keep a streaming interface switched to `alt` (so the device sees that stream as active) until dropped.
pub struct AltHold {
    fd: i32,
    iface: u8,
}

impl AltHold {
    pub fn open(info: &DeviceInfo, iface: u8, alt: u8) -> Result<Self, UsbError> {
        let fd = open_node(info)?;
        if let Err(e) = claim(fd, iface) {
            unsafe { libc::close(fd) };
            return Err(e);
        }
        let mut si = sys::SetInterface { interface: iface as u32, altsetting: alt as u32 };
        if unsafe { sys::ioctl(fd, sys::SETINTERFACE, &mut si) } < 0 {
            let e = os_error("set alt setting");
            release(fd, iface);
            unsafe { libc::close(fd) };
            return Err(e);
        }
        Ok(AltHold { fd, iface })
    }
}

impl Drop for AltHold {
    fn drop(&mut self) {
        release(self.fd, self.iface);
        unsafe { libc::close(self.fd) };
    }
}

/// USB Audio Class 2.0: the clock source's current sampling frequency (GET_CUR, CS_SAM_FREQ_CONTROL), or set it (SET_CUR).
fn uac2_rate(fd: i32, c: crate::desc::Uac2Clock, set: Option<u32>) -> Result<u32, UsbError> {
    let mut data = set.unwrap_or(0).to_le_bytes();
    let (request_type, what) = if set.is_some() { (0x21u8, "set clock rate") } else { (0xA1u8, "read clock rate") };
    let mut t = sys::CtrlTransfer { request_type, request: 0x01, value: 0x0100, index: (c.clock_id as u16) << 8 | c.ac_iface as u16, length: 4, timeout: 1000, data: data.as_mut_ptr() as *mut std::os::raw::c_void };
    if unsafe { sys::ioctl(fd, sys::CONTROL, &mut t) } < 0 {
        return Err(os_error(what));
    }
    Ok(u32::from_le_bytes(data))
}

/// Ask a USB Audio 2.0 device for `wanted` Hz, then for each candidate rate, until one is accepted **and reads back the same**; returns the
/// rate the device is really running. `None` if it accepts none of them.
pub(crate) fn negotiate_uac2_rate(fd: i32, c: crate::desc::Uac2Clock, wanted: u32) -> Option<u32> {
    let mut order = std::iter::once(wanted).chain(crate::desc::UAC2_CANDIDATE_RATES);
    order.find(|&r| uac2_rate(fd, c, Some(r)).is_ok() && uac2_rate(fd, c, None).ok() == Some(r))
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

    #[test]
    fn find_fm1_falls_back_to_any_midi_device_when_the_id_changed() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path().to_path_buf();
        for (k, v) in [("idVendor", "1234"), ("idProduct", "5678"), ("busnum", "1"), ("devnum", "3"), ("product", "NewFw")] {
            w(&r, &format!("1-1/{k}"), v);
        }
        assert!(find_fm1(0x4C4A, 0xC755, &r).is_empty(), "no MIDI interface yet");
        w(&r, "1-1/1-1:1.1/bInterfaceClass", "01");
        w(&r, "1-1/1-1:1.1/bInterfaceSubClass", "03");
        w(&r, "1-1/1-1:1.1/bInterfaceNumber", "01");
        let got = find_fm1(0x4C4A, 0xC755, &r);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].product, "NewFw");
    }
}
