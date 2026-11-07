# Phase 0 results — run on the real RG35XXSP + FM-1 (2026-11-08)

Source: the user's pasted terminal output from `phase0.sh` (SSH from iPhone). muOS 2601.0, kernel 4.9.170, Python 3.11.8.

## Verdict
**Isochronous audio capture from the FM-1 works through usbfs, with no kernel driver**, using `helper/isocap` (static aarch64 C).

| Check | Result |
|---|---|
| Descriptor parse (`usb_desc.py`) | Matches the brief exactly: iface 2 alt 1, ep 0x83, IN, async, 270 B, 2 ch / 24-bit / 44100 Hz. Playback iface 1 ep 0x03 adaptive. Blob is 334 bytes (= 316 config + 18 device descriptor). Device node `/dev/bus/usb/008/002`. |
| Capture, 4 s, FM-1 playing middle C | 4000 packets, **0 empty, 0 bad status, 0 resubmit failures**. Packet sizes 264-270 B (44-45 frames). **176,400 frames = exactly 4.000 s** at 44.1 kHz, i.e. no dropped frames. (The "99% of nominal" line only reflects ~30 ms of process start-up in the wall-clock denominator.) |
| Signal | Peak -20.0 dBFS, RMS -33.5 dBFS; 21% of frames below -70 dBFS (the note plays for ~2 s of the 4 s). |
| Idle capture, 3 s | **Exactly digital silence** (every sample 0). 132,652 frames in 3.01 s. Clean noise floor, nothing leaks into the stream when the synth is quiet. |
| Left vs right | Not bit-identical while sounding: max difference 3253 LSB of 2^23 = about **-68 dBFS**, peaks -20.0 / -20.1 dBFS. Effectively mirrored; mono downmix by averaging is safe (loses nothing audible). Idle capture was identical. |
| MIDI IN (ep 0x84), 8 s | **WORKS.** The first run (nothing received) was invalid: `UsbMidiDevice.read()` had a bug (`fcntl.ioctl()` with a bytes arg returns the buffer, not the byte count). After the fix (`_ioctl` helper, `tests/test_ioctl.py`), a 20 s run read 306 times (107 timeouts, 0 errors) and received **212 MIDI packets**: `ch1 NOTE ON/OFF`, **every velocity exactly 90**, notes 42-78, often 2-3 overlapping notes with note-offs arriving out of order. Whether those came from the keys or the FM-1's internal sequencer was not stated by the user; constant velocity 90 suggests fixed-velocity keys or a sequence. No CCs/program changes seen. |
| VBUS | `regulator.20 axp2202-drivevbus` reads **disabled** while the FM-1 is attached and fully working. So that regulator is not what the kernel reports as supplying the FM-1; 5 V arrives by some other path. `/sys/devices/platform/soc/usbc0` has `otg_role` (rw) and `usb_host`/`usb_device`/`usb_null` (mode 0400), driven by an "otg manager" driver. Unexplored; nothing was written. |

## Also learned
- `/sys/devices/platform/soc/usbc0/otg_role` reads `usb_host`. `/sys/class/power_supply/` has `axp2202-battery` and `axp2202-usb`; there is **no `current_now`** attribute, so battery draw cannot be measured that way.
- The real descriptor blob is saved as `tests/fixtures/fm1_descriptors.bin` and checked by `RealDeviceTests`.

## Not yet shown
- Longer captures (60 s+): the user's 60 s run was interrupted (three commands pasted at once). Rerun it on its own. Also capture under UI/mixer load.
- Whether recorded audio's timing relative to MIDI/playback is stable (latency calibration, feature 3).
- UAC2 / generic USB mics.
