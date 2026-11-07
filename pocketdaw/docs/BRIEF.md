<!-- Saved verbatim from the user's second brief so a local session can pick up without the chat. -->
<!-- Annotations added later by Claude are in HANDOFF.md, not here. -->

# Pocket DAW for the Anbernic RG35XXSP — Build Brief

**This supersedes the earlier M8-style tracker brief.** The new goal is a lightweight multitrack audio recorder/arranger on an Anbernic RG35XXSP (muOS) that records audio from an M-VAVE FM-1 synth over USB-C, plays tracks back together with a drum track, and doubles as a sampler with the FM-1 as its keyboard. The MIDI hardware findings from the tracker brief still apply and are repeated below.

The author (Sadie) already has Python/pygame apps on this same device (HillChord, HillBeat, Hill Sequencer). **HillBeat's drum architecture and built-in drum pattern presets are to be reused.** Ask the user for the repo/path first, read it, and reuse its launching, SDL init, input handling, UI style, and drum code rather than reinventing them.

---

## 1. Goal

Compose multitrack music with the FM-1 on the SP:
1. **Record multiple audio tracks** from the FM-1 over USB-C and play them back simultaneously.
2. **Drum track** driven by the HillBeat architecture, including all its built-in pattern presets.
3. **Sampler:** record a sample, or load samples from the user's library, and play them as a track. The FM-1's keyboard is used as the attached keyboard that triggers samples.
4. **Project files** (save/load) and **audio export** (mixdown WAV, plus stems).
5. **All 16 additional features listed in section 7.** The user has explicitly asked for every one of them; none are optional or stretch goals.

### Design constraint: portable and fully wired
The user will use this **out and about**, away from home Wi-Fi, a computer, or a phone. Therefore:
- **All audio and MIDI input is wired**, through the SP's single USB-C port (FM-1 and, when needed, a USB microphone or USB audio adapter). Do not design any feature that depends on Wi-Fi, Bluetooth, a phone, or a computer being present.
- **The app must run fully standalone** from the muOS menu using only the SP's buttons and screen. SSH and Wi-Fi are **development and deployment tooling only**, never required for normal operation or file management in the field.
- **Explicitly out of scope:** using an iPhone as a microphone. An iPhone does not present itself as a USB audio device, and the wireless workarounds (phone web page, Wi-Fi upload) violate the wired requirement.
- All projects, samples, and exports live on the SD card and must be usable without any network.

---

## 2. Verified hardware facts (tested on the real device)

**Device:** Anbernic RG35XXSP, Allwinner H700 (quad Cortex-A53 @ ~1.5 GHz), ~1 GB RAM (about 970 MB usable), 640x480 screen, aarch64.
**OS:** muOS 2601.0 "JACARANDA", Linux kernel 4.9.170, `/usr/bin/python3`. SD card is exfat, ~233 GB.
**Network:** Wi-Fi (`wlan0`, was 192.168.1.201, DHCP). SSH works as root; ask the user for the password, never store it.
**Sound cards on the SP:** only `audiocodec` (the SP's own output), `ahubdam`, `ahubhdmi`. There is **no USB audio card**.

**USB-C port works as a USB host (OTG).** The FM-1 enumerates as `lsusb` ID `4c4a:c755`, product `FM-1`, bus 008 (device number changes on every replug).

**The kernel has no USB audio/MIDI driver.** `CONFIG_SND_USB_AUDIO` is not set, no `snd_usb*` modules exist, `amidi`/alsa-utils are not installed, and `pyusb` is not installed. Do not plan on ALSA capture from the FM-1. Do not rebuild the kernel as a first option.

**FM-1 USB layout:** one composite device, five interfaces, all class 0x01 (audio):
- Interfaces 0 and 3: audio control
- Interfaces 1 and 2: audio streaming. **Decoded from the real device's raw descriptor dump (`/sys/bus/usb/devices/8-1/descriptors`)**, USB Audio Class 1.0, USB 1.1 full-speed, config total length 316 bytes:
  - **Interface 2 = CAPTURE (FM-1 to host).** Alt 0 has zero endpoints; **alt 1 has isochronous IN endpoint `0x83`**, asynchronous sync type, `wMaxPacketSize` 270 bytes, interval 1 ms (one packet per USB frame). Audio format: **PCM, 2 channels, 24-bit (3-byte subframes), 44100 Hz**, little-endian. Terminal path: microphone input terminal 4, feature unit 5, selector 7, output terminal 6 (USB streaming).
  - **Interface 1 = PLAYBACK (host to FM-1).** Alt 1 has isochronous OUT endpoint `0x03`, adaptive sync, 270 bytes, 1 ms interval, same format (2 ch, 24-bit, 44100 Hz). This is the path to the FM-1's speaker; it is not needed for the DAW's recording but could allow sending SP audio into the FM-1 later.
  - Data rate on capture: 44100 x 2 x 3 = **264 KB/s**, well inside full-speed isochronous bandwidth. The 270-byte max packet is 45 stereo frames (one extra frame of slack over the nominal 44.1 per ms).
  - **Not yet verified:** that the capture stream actually carries the FM-1's synth output, and that the SP's OHCI host controller delivers isochronous transfers cleanly through usbdevfs on kernel 4.9.170. That is the remaining Phase 0 proof.
- **Interface 4: MIDI streaming**, endpoints `0x04` (OUT) and `0x84` (IN)

**MIDI output is proven** to work without any kernel driver, by writing 4-byte USB-MIDI packets to endpoint `0x04` via usbdevfs ioctls. Working reference:

```python
import os, glob, fcntl, ctypes, struct, time
path = sorted(glob.glob('/dev/bus/usb/008/*'))[-1]   # FRAGILE: find by VID/PID instead
fd = os.open(path, os.O_RDWR)
fcntl.ioctl(fd, 0x8004550F, struct.pack('I', 4))      # USBDEVFS_CLAIMINTERFACE, iface 4
def send(b):
    buf = ctypes.create_string_buffer(bytes(b), len(b))
    req = struct.pack('IIIxxxxQ', 0x04, len(b), 1000, ctypes.addressof(buf))
    fcntl.ioctl(fd, 0xC0185502, req)                  # USBDEVFS_BULK
send([0x09, 0x90, 0x3C, 0x64])   # note on, ch1, middle C
time.sleep(1)
send([0x08, 0x80, 0x3C, 0x00])   # note off
```

That struct is the 64-bit `usbdevfs_bulktransfer` layout (aarch64 only). Find the device by scanning `/sys/bus/usb/devices/*/idVendor` and `idProduct` for `4c4a`/`c755`, then read `busnum` and `devnum` to build `/dev/bus/usb/BBB/DDD`. Re-scan on errors so replugging works. USB-MIDI packet: `[cable<<4 | CIN, status, d1, d2]` (CIN 0x8 note off, 0x9 note on, 0xB CC, 0xC program change, 0xE pitch bend).

## 3. FM-1 facts

- DX7-style 6-operator FM, 12 voices, 128 presets, built-in effects, own battery (2000 mAh), built-in speaker, 27-key keyboard, USB-C for power and data.
- USB-C is a **composite USB Audio + USB MIDI device**; M-VAVE and reviewers describe it as outputting audio digitally to a computer. The engine and effects are **mono**; the stereo output is a mirrored signal. The USB capture stream itself is **stereo, 24-bit, 44.1 kHz** (verified from descriptors), so **downmix to mono during capture** (check that left and right are identical or near-identical, then keep one channel) and halve storage. Make the project sample rate 44100 Hz so no resampling is needed on capture.
- Accepts MIDI notes with velocity, pitch bend, CC, program change (PC 0-127 selects sounds, in newer firmware), and DX7 SysEx banks. There is an official M-VAVE MIDI CC guide.
- What the FM-1 **sends** over MIDI (e.g., whether its own keyboard emits notes over USB) is **unverified**. Verify by reading endpoint `0x84` while pressing keys.
- Community reports MIDI latency on every connection type; expect to calibrate.

---

## 4. The critical unknown: audio capture over USB

Everything in the "record from the FM-1" requirement hinges on capturing the FM-1's USB audio without a kernel driver. This is feasible in principle (USB audio is standard isochronous streaming, and the USB host controller works), but it has not been tried. **Do this first, before building any UI.**

### Phase 0 — investigate and prove capture
1. **Descriptors: DONE (see section 2).** Capture is interface 2, alt 1, isochronous IN endpoint `0x83`, 270-byte packets every 1 ms, 2 ch / 24-bit / 44100 Hz PCM. Still write the small descriptor parser into `fm1tool.py descriptors` so the app reads these values at runtime instead of hardcoding them, and so other firmware revisions are handled.
2. **Prove capture.** Claim interface 2, select **alt setting 1** (alt 0 is zero-bandwidth; use `USBDEVFS_SETINTERFACE`), submit a queue of isochronous IN URBs (`USBDEVFS_SUBMITURB` / `USBDEVFS_REAPURB`; for example 8 to 32 frames per URB with several URBs kept in flight; derive the struct layout and ioctl numbers from `linux/usbdevice_fs.h` for aarch64 on kernel 4.9, and note that the iso frame descriptor array is variable-length), and write raw PCM to a file. Packets can be short or empty, so honor each frame's `actual_length`. Play a note on the FM-1 (MIDI note-on, proven working) while capturing and check the resulting WAV on a computer. Also check whether the stream is silent when the FM-1 is idle and whether left equals right.
3. **Choose the implementation.** Preferred: a **small C helper, cross-compiled for aarch64 and statically linked**, that does isochronous capture and writes PCM into a pipe or shared ring buffer for the Python app. Python/ctypes ISO URB handling is possible but is fragile for timing and GIL pauses. The device probably has no compiler; build on the dev machine and deploy the binary. If cross-compiling is a problem, do the capture in Python with a dedicated thread and a large buffer.
4. **Full-speed USB:** the controller is OHCI (`sunxi-ohci`), full-speed, 12 Mbit/s. The FM-1 capture stream is 264 KB/s (2 ch, 24-bit, 44.1 kHz), comfortably within isochronous bandwidth. The risk is not bandwidth but scheduling jitter and dropped frames, so use a deep URB queue and log any frames with errors or short packets.
5. **If capture cannot be made to work**, report back with evidence and propose alternatives rather than silently changing scope. Fallback options to present: (a) record MIDI performances and render them with a software DX7-style FM engine on the SP (sound will differ from the FM-1), (b) a custom `snd-usb-audio` build (high effort, needs muOS kernel sources and exact config), (c) hardware route with a USB audio interface.

---

## 5. Features

### 5.1 Audio tracks
- N mono tracks (start with 8), each with name, gain, pan, mute, solo, and color.
- **Recording:** arm a track, optional count-in and click, record from the FM-1 USB audio into a WAV file on the SD card (stream to disk; keep a ring buffer in RAM). Loop-record and overdub new takes without destroying old ones (keep takes per track).
- **Latency calibration:** measure round-trip latency (the app sends a MIDI note, records the response, detects the onset) and store an offset applied to recorded audio so overdubs line up with the click and drums. Also expose a manual offset.
- **Playback:** all tracks and the drum track play simultaneously, sample-accurately synchronized to a single transport clock.
- **Monitoring:** option to pass FM-1 audio through to the SP output while recording (latency warning), or rely on the FM-1's own speaker or headphone jack.

### 5.2 Drum track (HillBeat architecture)
- Reuse HillBeat's drum engine, samples, and sequencer data model.
- **Include every built-in pattern preset** from HillBeat; make them selectable per song section.
- Drum track runs on the same transport and tempo as the audio tracks and exports with the mix. Support pattern chaining per bar or section, swing, and per-drum volume.

### 5.3 Sampler
- **Source A: record a sample** from the FM-1's audio stream (the SP likely has no built-in microphone; verify with the user). Trim, normalize, and set loop points.
- **Source A2: a wired USB microphone or USB-C audio adapter** (with a mic or line input) plugged into the same USB-C port. This is the wired way to record voice and real-world sounds into the sampler out and about. It is the same kind of isochronous device as the FM-1, so the capture code must be written generically: read each device's descriptors at runtime (endpoint, packet size, sample rate, bit depth, channels) instead of hardcoding FM-1 values. UAC1 devices are the easy case; UAC2 devices need clock-source descriptor handling. Build this after FM-1 capture is proven. The UI must let the user choose the input source (FM-1 or USB mic) and handle hot-plugging.
- **One device at a time, by design.** The SP has a single USB-C port and the user does **not** need the FM-1 and a mic connected simultaneously, so **do not build hub support or multi-device handling**. Design for clean swapping instead: when a device is unplugged, finish and flush any in-progress take to disk, release the USB interface, and re-scan; when a new device appears, identify it (FM-1 by VID/PID `4c4a:c755`, anything else as a generic USB audio device from its descriptors) and show clearly in the UI which device is active. Typical workflow: record a sample with the mic, unplug it, plug in the FM-1, and play or record with it; recorded samples are saved to the SD card, so nothing is lost on a swap. Note that while the mic is connected, the FM-1 (and its keys as a sample trigger) is not available.
- **Power:** bus-powered devices draw from the SP's battery; show battery level and warn on low battery during recording.
- **Source B: load existing WAVs** from the user's library (Pianobook, Freesound, DecentSampler-derived samples, etc.) — browse the SD card folders.
- **Playback as a track:** assign a sample to a keyboard range (single sample pitched across keys by resampling, or one sample per key/pad), then record the keyboard performance as a sampler track (a note list stored in the project, rendered on playback).
- **FM-1 as the attached keyboard:** read notes from the FM-1's MIDI IN endpoint `0x84` to trigger samples on the SP in real time. This depends on the FM-1 actually transmitting key presses over USB MIDI (unverified; test early). The FM-1 will also sound its own engine on key presses; document how to quiet it (volume, silent preset, or a CC) and whether local control can be turned off.

### 5.4 Project files
- A project is a **folder** containing `project.json` (tempo, time signature, tracks, mixer settings, drum pattern chain, sampler assignments, latency offset, file references) and a `audio/` directory of WAV takes and recorded samples. Include a format version. Use relative paths so projects can be copied between SD cards.
- Autosave, a recovery file, and "save as". Never overwrite recorded audio; use non-destructive edits (trim/gain stored as metadata) where possible.

### 5.5 Export
- **Mixdown WAV** (offline render faster than real time, not a live capture): 16-bit/24-bit, project sample rate. Include a simple master limiter to prevent clipping and a dither option.
- **Stems:** export each track (and the drum bus) as its own WAV.
- Optional compressed formats only if an encoder exists on the device (check for `ffmpeg`/`lame`/`flac` before promising); otherwise WAV only.
- Write exports to a known folder on the SD card; show the path in the UI.

---

## 6. Constraints and architecture

- **Language:** Python 3 + pygame for UI, consistent with the Hill apps; stdlib for USB; a cross-compiled C helper is allowed for audio capture. Check whether `numpy` is available; if not, plan mixing in a small C helper or use `array`/`struct` carefully. Do not assume pip works on the device.
- **Memory:** ~970 MB RAM. The FM-1 delivers 44.1 kHz 24-bit stereo; after downmix, mono 24-bit is ~132 KB/s (~7.9 MB per minute per track), mono 16-bit is ~88 KB/s (~5.3 MB per minute per track). Record to disk as mono 24-bit WAV; decode to 16-bit or float for mixing. Stream from disk for long tracks; do not load everything into RAM.
- **Audio output:** the SP's `audiocodec` card via ALSA/SDL. Use a **single mixing callback** driven by one transport clock instead of starting many independent sounds, so tracks stay in sync. Pick a fixed project sample rate (match the FM-1's native rate to avoid resampling if possible).
- **Threads:** separate real-time-ish threads or processes for (1) USB capture, (2) audio output mixing, (3) UI. UI and file I/O must never block the audio paths. Use large ring buffers and report underruns/overruns in a diagnostics screen.
- **Resilience:** handle FM-1 unplug/replug without crashing or losing the in-progress take (flush recorded audio to disk continuously). Always release USB interfaces on exit (`atexit` and signal handlers).
- **Layers:** `fm1_usb` (discovery, claim/release, MIDI send/recv, audio capture), `engine` (transport, mixer, render), `drums` (from HillBeat), `sampler`, `project` (save/load), `export`, `ui`. Provide **mock backends** (fake FM-1 audio from a WAV file, fake MIDI) so most of the app runs and tests on a normal computer with no hardware.
- **Diagnostics CLI** (`fm1tool.py`): `list`, `descriptors`, `note`, `capture <seconds> <out.wav>`, `midi-in` (print notes from the FM-1), `latency-test`.

---

## 7. Required extra features (all 16 are in scope; the order is build priority)

The user confirmed they want **all** of these. Implement every one. If a feature proves too heavy for Python on this CPU, move its DSP into the C helper or simplify the implementation, and report the tradeoff to the user; do not silently drop a feature.

1. **Metronome/click and count-in**, with tempo, time signature, and tap tempo.
2. **Loop region + punch in/out** recording.
3. **Latency calibration** (listed above; essential for tight overdubs).
4. **Per-track gain/pan/mute/solo, master limiter, peak meters.**
5. **Trim, fade in/out, normalize, reverse** on takes and samples.
6. **Undo/redo** for edits and take management.
7. **Snap-to-grid and slice**: chop a recording into slices on the beat grid and rearrange them.
8. **Simple per-track effects** (low-pass/high-pass filter, delay, small reverb), implemented as cheap DSP; a shared send reverb is cheaper than per-track reverb.
9. **Sample pitch-shift by resampling** (varispeed) across the keyboard, with root-note setting.
10. **Arrangement view** with sections (verse/chorus) that loop and chain, using the drum presets per section.
11. **MIDI note recording from the FM-1's keys** (store notes alongside audio so a part can be re-recorded or retriggered).
12. **FM-1 control from the app:** program change and CC control (volume, effects) over the working MIDI output, with a patch/preset browser.
13. **BPM detection and sample-to-tempo fitting** (simple, resample-based).
14. **WAV import** with automatic sample-rate conversion.
15. **Project browser** with last-modified date, duration, and delete/duplicate.
16. **Battery and storage indicators** on screen (both the SP's and approximate FM-1 recording time left on the SD card).

Still out of scope: time-stretching, MP3 export, and plugin hosting. Note that features 8, 9, and 13 (effects, resampled pitch-shift, tempo fitting) involve per-sample DSP; do them in the C helper or with `numpy` if present, not in pure Python loops.

---

## 8. Controls

Propose a mapping consistent with the existing Hill apps (D-pad navigate, A confirm/edit, B back, Y modifier, X secondary, Start = play/stop, Select = record, L1/R1 = switch views, L2/R2 = nudge/zoom). Verify actual key codes on the device and match the Hill apps. Do not break muOS's menu/exit behavior; the app must exit cleanly and release USB.

---

## 9. Build order

1. **Phase 0:** descriptors dump, USB audio capture proof, FM-1 MIDI IN test (does it transmit key presses?), choose capture implementation.
2. Robust `fm1_usb` module and `fm1tool.py` (MIDI out, MIDI in, capture to WAV, device scan/replug).
3. Output mixer/transport on the SP with a test tone and a few WAVs playing in sync; measure underruns.
4. Record-to-track with click, count-in, and latency calibration.
5. Multitrack playback + mixer UI + project save/load.
6. HillBeat drum track integration with all presets and section chaining.
7. Mixdown and stem export (offline render).
8. Sampler: load/record samples, keyboard mapping, FM-1 keyboard triggering.
9. **All 16 extra features from section 7**, in the priority order listed there, then polish. None are optional; track each in a checklist and report any that had to be simplified.

## 10. Dev and deploy workflow

- The user works over **SSH from an iPhone**. Provide one-command deploy and run scripts (host/user from config or env var; never store the password; the SP's IP changes).
- Provide an SSH-friendly dev launcher and an on-device launcher for muOS's menu. Copy the Hill apps' mechanism for installation and for SDL video/audio environment variables; check the Hill app folders instead of guessing.
- Claude Code cannot hear audio. For every audible check, tell the user exactly what to listen for, or save a WAV and have them play it; verify captures numerically (peak level, duration, silence detection) in the CLI.

## 11. Open questions to resolve early
- ~~Direction, format, and sample rate of the FM-1's USB audio interfaces~~ (resolved from the descriptor dump: capture is interface 2 alt 1, endpoint `0x83`, 2 ch / 24-bit / 44100 Hz).
- Does the capture stream carry the synth output, and does isochronous capture run cleanly on the SP's OHCI controller?
- Which specific compact mic or USB audio adapter (UAC1 preferred, mic/line input) will the user carry? (One device is connected at a time; no hub needed.)
- Does the SP's 3.5mm jack have any capture path at all? (Assume output only; check `cat /proc/asound/pcm` for capture-capable devices.)
- Does the FM-1 send key presses over USB MIDI? Can its local sound be muted while sampling?
- Does the SP have any audio input (microphone)? Assume no unless shown.
- Is `numpy` or any compiler/ffmpeg present on the device? Can a cross-compiled aarch64 helper be built on the user's computer?
- Where are HillBeat's code, presets, and sample library, and how do the Hill apps launch under muOS?
- Preferred project sample rate and bit depth for export.
