//! Isochronous OUT playback: sending audio to a USB device that has no kernel driver here, the mirror of [`crate::iso`].
//!
//! A queue of URBs is kept in flight; every 1 ms packet carries the frames for that millisecond (44.1 kHz alternates 44 and 45 frames so the
//! average is exact). When the device finishes a URB it is refilled and sent again, so the device's own consumption paces the loop. For an
//! adaptive or synchronous endpoint (the FM-1) the device follows the rate we send. An asynchronous endpoint would also need feedback
//! packets from the device, which are not implemented: it is sent at the nominal rate and may glitch.
//!
//! usbfs packs an isochronous URB's data **back to back** in packet order, using each packet's `length`; unlike capture, where every
//! packet has the same stride, the packets here have different sizes, so [`fill_urb`] lays them out explicitly (and is unit-tested).

use crate::desc::StreamAlt;
use crate::dev::{self, DeviceInfo, UsbError};
use crate::iso::IsoStats;
use crate::sys::{self, IsoPacketDesc, Urb, URB_ISO_ASAP, URB_SIZE, URB_TYPE_ISO};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How many frames go in each packet so that the average over time is exactly the sample rate.
pub struct PacketSizer {
    rate_per_packet: u64,
    acc: u64,
}

impl PacketSizer {
    /// `rate` in Hz; `interval_ms` is the endpoint's packet interval in milliseconds (1 for the usual full-speed audio endpoint).
    pub fn new(rate: u32, interval_ms: u32) -> Self {
        Self::new_us(rate, interval_ms.max(1) * 1000)
    }

    /// Like [`Self::new`] with the packet interval in microseconds (125 for a high-speed USB Audio 2.0 endpoint).
    pub fn new_us(rate: u32, interval_us: u32) -> Self {
        PacketSizer { rate_per_packet: rate as u64 * interval_us.max(1) as u64, acc: 0 }
    }

    pub fn next_frames(&mut self) -> usize {
        self.acc += self.rate_per_packet;
        let f = self.acc / 1_000_000;
        self.acc %= 1_000_000;
        f as usize
    }
}

struct PlayBuf {
    mem: Vec<u64>, // 8-byte aligned storage for the Urb + its iso_frame_desc[]
    data: Vec<u8>,
    npackets: usize,
}

impl PlayBuf {
    fn new(npackets: usize, maxpkt: usize, ep: u8, idx: usize) -> Self {
        let mut b = PlayBuf { mem: vec![0u64; (URB_SIZE + npackets * 12).div_ceil(8)], data: vec![0u8; npackets * maxpkt], npackets };
        let buf_ptr = b.data.as_mut_ptr() as *mut _;
        let urb = b.urb();
        unsafe {
            (*urb).typ = URB_TYPE_ISO;
            (*urb).endpoint = ep;
            (*urb).flags = URB_ISO_ASAP;
            (*urb).buffer = buf_ptr;
            (*urb).number_of_packets = npackets as i32;
            (*urb).usercontext = idx as *mut _;
        }
        b
    }

    fn urb(&mut self) -> *mut Urb {
        self.mem.as_mut_ptr() as *mut Urb
    }

    fn descs_mut(&mut self) -> &mut [IsoPacketDesc] {
        unsafe { std::slice::from_raw_parts_mut((self.mem.as_mut_ptr() as *mut u8).add(URB_SIZE) as *mut IsoPacketDesc, self.npackets) }
    }
}

/// Fill every packet of `buf` with the next frames from `fill` (called with the packet's byte slice and its frame count), packed back to
/// back, and set each packet's length and the URB's total length. Returns the total bytes.
fn fill_urb(buf: &mut PlayBuf, sizer: &mut PacketSizer, frame_bytes: usize, maxpkt: usize, fill: &mut dyn FnMut(&mut [u8], usize)) -> usize {
    let max_frames = (maxpkt / frame_bytes.max(1)).max(1);
    let mut off = 0usize;
    for k in 0..buf.npackets {
        let frames = sizer.next_frames().min(max_frames);
        let bytes = frames * frame_bytes;
        fill(&mut buf.data[off..off + bytes], frames);
        let d = &mut buf.descs_mut()[k];
        d.length = bytes as u32;
        d.actual_length = 0;
        d.status = 0;
        off += bytes;
    }
    unsafe { (*buf.urb()).buffer_length = off as i32 };
    off
}

pub struct IsoPlayback {
    fd: i32,
    iface: u8,
    bufs: Vec<PlayBuf>,
    maxpkt: usize,
    frame_bytes: usize,
    rate: u32,
    interval_us: u32,
    streaming: bool,
}

impl IsoPlayback {
    /// Claim the streaming interface, select the playback alt setting, tell a multi-rate device the rate, and allocate `n_urbs` URBs of
    /// `packets_per_urb` milliseconds each.
    pub fn open(info: &DeviceInfo, alt: &StreamAlt, rate: u32, packets_per_urb: usize, n_urbs: usize) -> Result<Self, UsbError> {
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
        if let Some(clock) = alt.uac2 {
            // the packets are sized for `rate`, so the device must really run at it
            if dev::negotiate_uac2_rate(fd, clock, rate) != Some(rate) {
                dev::release(fd, alt.iface);
                unsafe { libc::close(fd) };
                return Err(UsbError::Io(format!("the device would not run at {rate} Hz")));
            }
        } else if alt.rate_control || alt.rates.len() > 1 {
            let _ = dev::set_sampling_rate(fd, alt.ep, rate);
        }
        let maxpkt = alt.max_packet as usize;
        let bufs = (0..n_urbs.clamp(2, 64)).map(|i| PlayBuf::new(packets_per_urb.max(1), maxpkt, alt.ep, i)).collect();
        Ok(IsoPlayback { fd, iface: alt.iface, bufs, maxpkt, frame_bytes: alt.frame_bytes(), rate, interval_us: alt.interval_us.max(125), streaming: false })
    }

    /// Stream until `stop`. `fill(bytes, frames)` must write exactly `frames` device frames into `bytes` (silence when it has nothing).
    pub fn run(&mut self, stop: &AtomicBool, mut fill: impl FnMut(&mut [u8], usize)) -> Result<IsoStats, UsbError> {
        let mut st = IsoStats::default();
        let mut sizer = PacketSizer::new_us(self.rate, self.interval_us);
        let (maxpkt, frame_bytes) = (self.maxpkt, self.frame_bytes);
        // prime the whole queue (it starts playing as soon as it is submitted), then keep it topped up
        for i in 0..self.bufs.len() {
            fill_urb(&mut self.bufs[i], &mut sizer, frame_bytes, maxpkt, &mut fill);
            let u = self.bufs[i].urb();
            if unsafe { sys::ioctl(self.fd, sys::SUBMITURB, u) } < 0 {
                return Err(dev::os_error("submit urb"));
            }
        }
        self.streaming = true;
        let mut pfd = libc::pollfd { fd: self.fd, events: libc::POLLOUT, revents: 0 };
        'outer: while !stop.load(Ordering::Relaxed) {
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
                let b = &mut self.bufs[idx];
                for d in b.descs_mut().iter() {
                    st.packets += 1;
                    if d.status != 0 {
                        st.bad_status += 1;
                        if st.first_bad_status == 0 {
                            st.first_bad_status = d.status as i32;
                        }
                    }
                }
                st.bytes += fill_urb(b, &mut sizer, frame_bytes, maxpkt, &mut fill) as u64;
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
            std::thread::sleep(Duration::from_millis(5));
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
            unsafe { sys::ioctl(self.fd, sys::SETINTERFACE, &mut si) }; // alt 0: no bandwidth reserved
            dev::release(self.fd, self.iface);
            unsafe { libc::close(self.fd) };
            self.fd = -1;
        }
    }
}

impl Drop for IsoPlayback {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_sizes_average_to_exactly_the_sample_rate() {
        for rate in [44_100u32, 48_000, 96_000, 22_050] {
            let mut s = PacketSizer::new(rate, 1);
            let total: usize = (0..1000).map(|_| s.next_frames()).sum();
            assert_eq!(total, rate as usize, "{rate} Hz over one second of packets");
        }
        let mut s = PacketSizer::new(44_100, 1);
        let first: Vec<usize> = (0..10).map(|_| s.next_frames()).collect();
        assert_eq!(first, vec![44, 44, 44, 44, 44, 44, 44, 44, 44, 45], "nine 44s then a 45");
        let mut s = PacketSizer::new(48_000, 1);
        assert!((0..50).all(|_| s.next_frames() == 48), "48 kHz is a constant 48");
        // a slower packet interval carries proportionally more
        let mut s = PacketSizer::new(48_000, 2);
        assert_eq!(s.next_frames(), 96);
    }

    #[test]
    fn urb_data_is_packed_back_to_back_with_each_packets_own_length() {
        // the FM-1: 2 channels x 24-bit = 6 bytes per frame, 270-byte packets; 10 packets = 10 ms = 441 frames
        let mut buf = PlayBuf::new(10, 270, 0x03, 0);
        let mut sizer = PacketSizer::new(44_100, 1);
        let mut n = 0u8;
        let mut packet_frames = Vec::new();
        let total = fill_urb(&mut buf, &mut sizer, 6, 270, &mut |bytes, frames| {
            assert_eq!(bytes.len(), frames * 6);
            bytes.fill(n); // each packet is a different constant so the layout can be read back
            n += 1;
            packet_frames.push(frames);
        });
        assert_eq!(packet_frames, vec![44, 44, 44, 44, 44, 44, 44, 44, 44, 45]);
        assert_eq!(total, 441 * 6);
        let lens: Vec<u32> = buf.descs_mut().iter().map(|d| d.length).collect();
        assert_eq!(lens, vec![264, 264, 264, 264, 264, 264, 264, 264, 264, 270]);
        assert_eq!(unsafe { (*buf.urb()).buffer_length }, 441 * 6, "the URB length is the sum of the packet lengths");
        // packet k's data starts where packet k-1's ended, not at k * 270
        let mut off = 0usize;
        for (k, l) in lens.iter().enumerate() {
            assert!(buf.data[off..off + *l as usize].iter().all(|b| *b == k as u8), "packet {k} is where the kernel will look for it");
            off += *l as usize;
        }
        assert!(buf.descs_mut().iter().all(|d| d.actual_length == 0 && d.status == 0));
        // refilling continues the sizer's rhythm instead of starting over
        let again = fill_urb(&mut buf, &mut sizer, 6, 270, &mut |_, f| packet_frames.push(f));
        assert_eq!(again, 441 * 6);
    }

    #[test]
    fn packet_sizes_average_the_rate_exactly_at_125_us_too() {
        for rate in [44_100u32, 48_000, 96_000] {
            let mut z = PacketSizer::new_us(rate, 125);
            let total: usize = (0..8000).map(|_| z.next_frames()).sum(); // one second of microframes
            assert_eq!(total, rate as usize, "{rate}");
        }
        let mut z = PacketSizer::new_us(44_100, 125);
        let sizes: Vec<usize> = (0..16).map(|_| z.next_frames()).collect();
        assert!(sizes.iter().all(|s| *s == 5 || *s == 6), "{sizes:?}");
        let mut ms = PacketSizer::new(44_100, 1);
        let v: Vec<usize> = (0..10).map(|_| ms.next_frames()).collect();
        assert_eq!(v.iter().filter(|s| **s == 45).count(), 1, "the 1 ms pattern is unchanged: nine 44s and one 45");
    }

    #[test]
    fn a_packet_never_exceeds_what_the_endpoint_accepts() {
        // an endpoint that takes 100 bytes (16 frames of stereo 24-bit) while the rate asks for 44: capped, not overflowed
        let mut buf = PlayBuf::new(3, 100, 0x01, 0);
        let mut sizer = PacketSizer::new(44_100, 1);
        let mut frames = Vec::new();
        fill_urb(&mut buf, &mut sizer, 6, 100, &mut |bytes, f| {
            assert!(bytes.len() <= 100);
            frames.push(f);
        });
        assert_eq!(frames, vec![16, 16, 16]);
        assert!(buf.descs_mut().iter().all(|d| d.length <= 100));
    }
}
