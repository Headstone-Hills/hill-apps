//! Isochronous IN capture (the FM-1's audio), replacing the C helper.
//!
//! A deep queue of URBs is kept in flight; each completed URB is walked packet by packet, honouring
//! every packet's `actual_length` (packets may be short or empty), and the payload is handed to a sink.

use crate::dev::{self, DeviceInfo, UsbError};
use crate::desc::StreamAlt;
use crate::sys::{self, IsoPacketDesc, Urb, URB_ISO_ASAP, URB_SIZE, URB_TYPE_ISO};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Default, Clone)]
pub struct IsoStats {
    pub packets: u64,
    pub empty: u64,
    pub bad_status: u64,
    pub first_bad_status: i32,
    pub bytes: u64,
    pub min_len: u32,
    pub max_len: u32,
    pub urbs: u64,
    pub resubmit_fail: u64,
    pub disconnected: bool,
}

impl IsoStats {
    pub fn summary(&self) -> String {
        format!(
            "packets={} empty={} bad_status={} first_bad_status={} bytes={} min_len={} max_len={} urbs={} resubmit_fail={}{}",
            self.packets, self.empty, self.bad_status, self.first_bad_status, self.bytes, self.min_len, self.max_len,
            self.urbs, self.resubmit_fail, if self.disconnected { " DISCONNECTED" } else { "" }
        )
    }
}

/// Walk one completed URB's packets. `data` holds packet k at `k * maxpkt`. Pure; unit-tested.
pub fn process_packets(descs: &mut [IsoPacketDesc], data: &[u8], maxpkt: usize, st: &mut IsoStats, sink: &mut dyn FnMut(&[u8])) {
    for (k, d) in descs.iter_mut().enumerate() {
        st.packets += 1;
        if d.status != 0 {
            st.bad_status += 1;
            if st.first_bad_status == 0 {
                st.first_bad_status = d.status as i32;
            }
        }
        let n = (d.actual_length as usize).min(maxpkt);
        if n == 0 {
            st.empty += 1;
        } else {
            if st.min_len == 0 || (n as u32) < st.min_len {
                st.min_len = n as u32;
            }
            st.max_len = st.max_len.max(n as u32);
            sink(&data[k * maxpkt..k * maxpkt + n]);
            st.bytes += n as u64;
        }
        d.actual_length = 0;
        d.status = 0;
    }
}

struct UrbBuf {
    mem: Vec<u64>, // 8-byte aligned storage for the Urb + its iso_frame_desc[]
    data: Vec<u8>,
    nframes: usize,
}

impl UrbBuf {
    fn new(nframes: usize, maxpkt: usize, ep: u8, idx: usize) -> Self {
        let mut b = UrbBuf { mem: vec![0u64; (URB_SIZE + nframes * 12).div_ceil(8)], data: vec![0u8; nframes * maxpkt], nframes };
        let buf_ptr = b.data.as_mut_ptr() as *mut _;
        let urb = b.urb();
        unsafe {
            (*urb).typ = URB_TYPE_ISO;
            (*urb).endpoint = ep;
            (*urb).flags = URB_ISO_ASAP;
            (*urb).buffer = buf_ptr;
            (*urb).buffer_length = (nframes * maxpkt) as i32;
            (*urb).number_of_packets = nframes as i32;
            (*urb).usercontext = idx as *mut _;
        }
        for d in b.descs_mut() {
            d.length = maxpkt as u32;
        }
        b
    }
    fn urb(&mut self) -> *mut Urb {
        self.mem.as_mut_ptr() as *mut Urb
    }
    fn descs_mut(&mut self) -> &mut [IsoPacketDesc] {
        unsafe { std::slice::from_raw_parts_mut((self.mem.as_mut_ptr() as *mut u8).add(URB_SIZE) as *mut IsoPacketDesc, self.nframes) }
    }
}

pub struct IsoCapture {
    fd: i32,
    iface: u8,
    bufs: Vec<UrbBuf>,
    maxpkt: usize,
    streaming: bool,
    /// the sample rate the device is streaming at (what was asked for, or for a USB Audio 2.0 device what it settled on)
    pub rate: u32,
}

impl IsoCapture {
    /// Claim the streaming interface, select the alt setting (alt 0 has no bandwidth), allocate URBs.
    pub fn open(info: &DeviceInfo, alt: &StreamAlt, frames_per_urb: usize, n_urbs: usize) -> Result<Self, UsbError> {
        Self::open_with_rate(info, alt, None, frames_per_urb, n_urbs)
    }

    /// Like [`Self::open`], and for a device that offers several rates, tell it which one to stream at (best effort: a device that
    /// refuses keeps its default, and the caller can see which rate it really delivers from the packet sizes).
    pub fn open_with_rate(info: &DeviceInfo, alt: &StreamAlt, rate: Option<u32>, frames_per_urb: usize, n_urbs: usize) -> Result<Self, UsbError> {
        let fd = dev::open_node(info)?;
        if let Err(e) = dev::claim(fd, alt.iface) {
            unsafe { libc::close(fd) };
            return Err(e);
        }
        let mut si = sys::SetInterface { interface: alt.iface as u32, altsetting: alt.alt as u32 };
        if unsafe { sys::ioctl(fd, sys::SETINTERFACE, &mut si) } < 0 {
            let e = dev::os_error("set alt setting");
            dev::release(fd, alt.iface);
            unsafe { libc::close(fd) };
            return Err(e);
        }
        let mut actual = rate.unwrap_or_else(|| alt.rates.first().copied().unwrap_or(0));
        if let Some(clock) = alt.uac2 {
            // USB Audio 2.0: the rate is set on the clock source, and must read back; whatever the device settles on is what is captured
            match dev::negotiate_uac2_rate(fd, clock, actual.max(1)) {
                Some(r) => actual = r,
                None => {
                    dev::release(fd, alt.iface);
                    unsafe { libc::close(fd) };
                    return Err(UsbError::Io("the device accepted none of 44.1, 48, 96, 88.2 or 192 kHz".into()));
                }
            }
        } else if let Some(r) = rate.filter(|_| alt.rates.len() > 1) {
            let _ = dev::set_sampling_rate(fd, alt.ep, r);
        }
        let maxpkt = alt.max_packet as usize;
        let bufs = (0..n_urbs.clamp(1, 64)).map(|i| UrbBuf::new(frames_per_urb.max(1), maxpkt, alt.ep, i)).collect();
        Ok(IsoCapture { fd, iface: alt.iface, bufs, maxpkt, streaming: false, rate: actual })
    }

    /// Capture until `stop` is set or `seconds` elapse (0 = until stopped). Payload goes to `sink`.
    pub fn run(&mut self, stop: &AtomicBool, seconds: f64, mut sink: impl FnMut(&[u8])) -> Result<IsoStats, UsbError> {
        let mut st = IsoStats::default();
        for i in 0..self.bufs.len() {
            let u = self.bufs[i].urb();
            if unsafe { sys::ioctl(self.fd, sys::SUBMITURB, u) } < 0 {
                return Err(dev::os_error("submit urb"));
            }
        }
        self.streaming = true;
        let start = Instant::now();
        let limit = Duration::from_secs_f64(seconds);
        let mut pfd = libc::pollfd { fd: self.fd, events: libc::POLLOUT, revents: 0 };
        'outer: while !stop.load(Ordering::Relaxed) && (seconds <= 0.0 || start.elapsed() < limit) {
            let pr = unsafe { libc::poll(&mut pfd, 1, 100) };
            if pr < 0 && sys::errno() != libc::EINTR {
                return Err(dev::os_error("poll"));
            }
            if pr <= 0 {
                continue;
            }
            loop {
                let mut u: *mut Urb = std::ptr::null_mut();
                if unsafe { sys::ioctl(self.fd, sys::REAPURBNDELAY, &mut u) } < 0 {
                    match sys::errno() {
                        libc::EAGAIN => break,
                        libc::ENODEV => {
                            st.disconnected = true;
                            break 'outer;
                        }
                        _ => return Err(dev::os_error("reap urb")),
                    }
                }
                st.urbs += 1;
                let idx = unsafe { (*u).usercontext as usize };
                let maxpkt = self.maxpkt;
                let b = &mut self.bufs[idx];
                let data = std::mem::take(&mut b.data);
                process_packets(b.descs_mut(), &data, maxpkt, &mut st, &mut sink);
                b.data = data;
                unsafe { (*u).start_frame = 0 };
                if unsafe { sys::ioctl(self.fd, sys::SUBMITURB, u) } < 0 {
                    st.resubmit_fail += 1;
                }
            }
        }
        self.shutdown();
        Ok(st)
    }

    fn shutdown(&mut self) {
        if self.streaming {
            for b in self.bufs.iter_mut() {
                let u = b.urb();
                unsafe { sys::ioctl(self.fd, sys::DISCARDURB, u) };
            }
            for _ in 0..self.bufs.len() {
                let mut u: *mut Urb = std::ptr::null_mut();
                if unsafe { sys::ioctl(self.fd, sys::REAPURBNDELAY, &mut u) } < 0 {
                    break;
                }
            }
            self.streaming = false;
        }
        if self.fd >= 0 {
            let mut si = sys::SetInterface { interface: self.iface as u32, altsetting: 0 };
            unsafe { sys::ioctl(self.fd, sys::SETINTERFACE, &mut si) }; // stop streaming
            dev::release(self.fd, self.iface);
            unsafe { libc::close(self.fd) };
            self.fd = -1;
        }
    }
}

impl Drop for IsoCapture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_empty_packets_are_honoured_and_counted() {
        let maxpkt = 8;
        let mut data = vec![0xEEu8; 4 * maxpkt]; // 0xEE = stale buffer content that must not leak out
        data[..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        data[2 * maxpkt..2 * maxpkt + 8].copy_from_slice(&[9; 8]);
        let mut d = [
            IsoPacketDesc { length: 8, actual_length: 6, status: 0 },
            IsoPacketDesc { length: 8, actual_length: 0, status: 0 },
            IsoPacketDesc { length: 8, actual_length: 8, status: 0 },
            IsoPacketDesc { length: 8, actual_length: 3, status: (-75i32) as u32 },
        ];
        let mut out = Vec::new();
        let mut st = IsoStats::default();
        process_packets(&mut d, &data, maxpkt, &mut st, &mut |p| out.extend_from_slice(p));
        assert_eq!(&out[..6], &[1, 2, 3, 4, 5, 6]);
        assert_eq!(out.len(), 6 + 8 + 3);
        assert_eq!(out[6..14], [9; 8]);
        assert_eq!(out[14..], [0xEE; 3]); // the bad-status packet still delivers its actual_length bytes
        assert_eq!((st.packets, st.empty, st.bad_status, st.first_bad_status), (4, 1, 1, -75));
        assert_eq!((st.min_len, st.max_len, st.bytes), (3, 8, 17));
        assert!(d.iter().all(|x| x.actual_length == 0 && x.status == 0), "descriptors are reset for resubmit");
    }

    #[test]
    fn urb_is_initialised_for_iso_in() {
        let mut b = UrbBuf::new(16, 270, 0x83, 5);
        let u = b.urb();
        unsafe {
            assert_eq!(((*u).typ, (*u).endpoint, (*u).flags), (URB_TYPE_ISO, 0x83, URB_ISO_ASAP));
            assert_eq!(((*u).number_of_packets, (*u).buffer_length, (*u).usercontext as usize), (16, 16 * 270, 5));
        }
        assert!(b.descs_mut().iter().all(|d| d.length == 270));
        assert_eq!(b.descs_mut().len(), 16);
    }
}
