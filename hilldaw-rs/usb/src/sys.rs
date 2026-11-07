//! usbfs ABI. Layouts and request numbers are the 64-bit ones from `linux/usbdevice_fs.h`;
//! `tests` below pin them to values printed by a C program compiled against the real header.

use std::os::raw::{c_int, c_void};

const IOC_NONE: u32 = 0;
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

pub const fn ioc(dir: u32, ty: u8, nr: u8, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr as u32
}

#[repr(C)]
pub struct BulkTransfer {
    pub ep: u32,
    pub len: u32,
    pub timeout: u32,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct SetInterface {
    pub interface: u32,
    pub altsetting: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct IsoPacketDesc {
    pub length: u32,
    pub actual_length: u32,
    pub status: u32,
}

/// `struct usbdevfs_urb`; `iso_frame_desc[]` follows it in memory.
#[repr(C)]
pub struct Urb {
    pub typ: u8,
    pub endpoint: u8,
    pub status: i32,
    pub flags: u32,
    pub buffer: *mut c_void,
    pub buffer_length: i32,
    pub actual_length: i32,
    pub start_frame: i32,
    pub number_of_packets: i32,
    pub error_count: i32,
    pub signr: u32,
    pub usercontext: *mut c_void,
}

pub const URB_SIZE: usize = std::mem::size_of::<Urb>();
pub const URB_TYPE_ISO: u8 = 0;
pub const URB_ISO_ASAP: u32 = 2;

pub const CLAIMINTERFACE: u32 = ioc(IOC_READ, b'U', 15, 4);
pub const RELEASEINTERFACE: u32 = ioc(IOC_READ, b'U', 16, 4);
pub const SETINTERFACE: u32 = ioc(IOC_READ, b'U', 4, std::mem::size_of::<SetInterface>());
pub const BULK: u32 = ioc(IOC_READ | IOC_WRITE, b'U', 2, std::mem::size_of::<BulkTransfer>());
pub const SUBMITURB: u32 = ioc(IOC_READ, b'U', 10, URB_SIZE);
pub const REAPURB: u32 = ioc(IOC_WRITE, b'U', 12, std::mem::size_of::<*mut c_void>());
pub const REAPURBNDELAY: u32 = ioc(IOC_WRITE, b'U', 13, std::mem::size_of::<*mut c_void>());
pub const DISCARDURB: u32 = ioc(IOC_NONE, b'U', 11, 0);

/// Thin ioctl wrapper. Unlike Python's fcntl.ioctl, libc::ioctl returns the kernel's integer result
/// (the byte count for BULK), so the byte-count bug that bit the Python MIDI reader cannot happen here.
///
/// # Safety
/// `arg` must point to the structure the request expects.
pub unsafe fn ioctl<T>(fd: c_int, req: u32, arg: *mut T) -> c_int {
    // `as _` rather than `libc::Ioctl`: that alias exists only on Linux targets, and inferring the request type
    // lets the crate (and its host-side tests) compile on macOS too. usbfs itself is Linux-only at run time.
    libc::ioctl(fd, req as _, arg)
}

pub fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    // Printed by a C program compiled against <linux/usbdevice_fs.h> (see HANDOFF.md).
    #[test]
    fn struct_layout_matches_the_kernel_header() {
        assert_eq!(size_of::<BulkTransfer>(), 24);
        assert_eq!(size_of::<SetInterface>(), 8);
        assert_eq!(size_of::<IsoPacketDesc>(), 12);
        assert_eq!(size_of::<Urb>(), 56);
        assert_eq!(offset_of!(Urb, typ), 0);
        assert_eq!(offset_of!(Urb, endpoint), 1);
        assert_eq!(offset_of!(Urb, status), 4);
        assert_eq!(offset_of!(Urb, flags), 8);
        assert_eq!(offset_of!(Urb, buffer), 16);
        assert_eq!(offset_of!(Urb, buffer_length), 24);
        assert_eq!(offset_of!(Urb, actual_length), 28);
        assert_eq!(offset_of!(Urb, start_frame), 32);
        assert_eq!(offset_of!(Urb, number_of_packets), 36);
        assert_eq!(offset_of!(Urb, error_count), 40);
        assert_eq!(offset_of!(Urb, signr), 44);
        assert_eq!(offset_of!(Urb, usercontext), 48);
    }

    #[test]
    fn ioctl_numbers_match_the_kernel_header() {
        assert_eq!(CLAIMINTERFACE, 0x8004550f);
        assert_eq!(RELEASEINTERFACE, 0x80045510);
        assert_eq!(SETINTERFACE, 0x80085504);
        assert_eq!(BULK, 0xc0185502); // the value proven on the real FM-1 from Python
        assert_eq!(SUBMITURB, 0x8038550a);
        assert_eq!(REAPURB, 0x4008550c);
        assert_eq!(REAPURBNDELAY, 0x4008550d);
        assert_eq!(DISCARDURB, 0x550b);
        assert_eq!((URB_TYPE_ISO, URB_ISO_ASAP), (0, 2));
    }
}
