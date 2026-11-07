//! FM-1 USB layer for HillDAW (Linux usbfs, no kernel audio/MIDI driver required).
//!
//! * [`sys`]   raw usbfs ABI (structs + ioctl numbers, checked against `linux/usbdevice_fs.h` in tests)
//! * [`desc`]  USB Audio Class 1.0 descriptor parser
//! * [`midi`]  USB-MIDI packet builders/decoder
//! * [`dev`]   device discovery (sysfs, by VID/PID) and the MIDI endpoint handle
//! * [`iso`]   isochronous audio capture
//! * [`wav`]   WAV writing and numeric level analysis (we cannot listen, so we measure)

#[cfg(not(target_pointer_width = "64"))]
compile_error!("the usbfs struct layouts here are the 64-bit ones (aarch64 / x86_64)");

pub mod audio;
pub mod desc;
pub mod dev;
pub mod iso;
pub mod iso_out;
pub mod midi;
pub mod sys;
pub mod wav;

pub const FM1_VID: u16 = 0x4C4A;
pub const FM1_PID: u16 = 0xC755;
