"""FM-1 USB-MIDI hardware layer (standard library only).

The RG35XXSP kernel has no USB-audio/MIDI driver, so we talk to the FM-1's
usbfs device node directly: claim the MIDI-streaming interface and write
4-byte USB-MIDI event packets to its bulk-OUT endpoint with ioctls.

Layers in this file:
  * packet builders / decoder       (pure functions)
  * device discovery                (scan /sys/bus/usb/devices by VID/PID)
  * UsbMidiDevice                   (claim / bulk write / release)
  * Transport classes               (UsbTransport, MockTransport, ConsoleTransport)

Everything above the Transport interface is hardware-agnostic, so the whole
app runs with MockTransport on a normal computer.
"""

from __future__ import annotations

import ctypes
import fcntl
import os
import struct
import threading
import time

FM1_VID = 0x4C4A
FM1_PID = 0xC755
SYSFS_USB = "/sys/bus/usb/devices"
DEFAULT_MIDI_IFACE = 4          # verified on the real unit
DEFAULT_EP_OUT = 0x04
DEFAULT_EP_IN = 0x84

# --- ioctl numbers (computed, and cross-checked against the verified values) --
_IOC_WRITE, _IOC_READ = 1, 2


def _ioc(direction, typ, nr, size):
    return (direction << 30) | (size << 16) | (ord(typ) << 8) | nr


_PTR64 = ctypes.sizeof(ctypes.c_void_p) == 8
# struct usbdevfs_bulktransfer { uint ep; uint len; uint timeout; void *data; }
_BULK_FMT = "=IIIxxxxQ" if _PTR64 else "=IIII"
_BULK_SIZE = struct.calcsize(_BULK_FMT)
USBDEVFS_CLAIMINTERFACE = _ioc(_IOC_READ, "U", 15, 4)
USBDEVFS_RELEASEINTERFACE = _ioc(_IOC_READ, "U", 16, 4)
USBDEVFS_BULK = _ioc(_IOC_READ | _IOC_WRITE, "U", 2, _BULK_SIZE)
if _PTR64:  # the values proven on the device (aarch64)
    assert USBDEVFS_CLAIMINTERFACE == 0x8004550F
    assert USBDEVFS_RELEASEINTERFACE == 0x80045510
    assert USBDEVFS_BULK == 0xC0185502


def _ioctl(fd, request, arg):
    """ioctl returning the kernel's integer result.

    fcntl.ioctl() given an immutable bytes arg returns a *copy of the buffer*, not the ioctl's
    return value (USBDEVFS_BULK returns the transferred byte count). A bytearray makes it
    return the int. (This bit midi-in once: every read looked empty.)"""
    return fcntl.ioctl(fd, request, bytearray(arg))


# --- USB-MIDI packets ---------------------------------------------------------
CIN_NOTE_OFF, CIN_NOTE_ON, CIN_CC, CIN_PC, CIN_PB, CIN_SINGLE = 0x08, 0x09, 0x0B, 0x0C, 0x0E, 0x0F


def pkt_note_on(ch, note, vel):
    return bytes([CIN_NOTE_ON, 0x90 | (ch & 15), note & 127, max(1, min(127, vel))])


def pkt_note_off(ch, note, vel=0):
    return bytes([CIN_NOTE_OFF, 0x80 | (ch & 15), note & 127, vel & 127])


def pkt_cc(ch, cc, val):
    return bytes([CIN_CC, 0xB0 | (ch & 15), cc & 127, val & 127])


def pkt_pc(ch, prog):
    return bytes([CIN_PC, 0xC0 | (ch & 15), prog & 127, 0])


def pkt_pitchbend(ch, value14):
    v = max(0, min(16383, int(value14)))
    return bytes([CIN_PB, 0xE0 | (ch & 15), v & 127, (v >> 7) & 127])


def pkt_realtime(byte):
    return bytes([CIN_SINGLE, byte, 0, 0])


def decode_packets(data):
    """Human-readable decode of concatenated USB-MIDI packets (for logs/tests)."""
    out = []
    for i in range(0, len(data) - len(data) % 4, 4):
        cin, st, d1, d2 = data[i] & 15, data[i + 1], data[i + 2], data[i + 3]
        ch = (st & 15) + 1
        kind = st & 0xF0
        if cin == CIN_NOTE_ON and kind == 0x90:
            out.append("ch%d NOTE ON %d vel %d" % (ch, d1, d2))
        elif cin == CIN_NOTE_OFF and kind == 0x80:
            out.append("ch%d NOTE OFF %d" % (ch, d1))
        elif cin == CIN_CC:
            out.append("ch%d CC %d = %d" % (ch, d1, d2))
        elif cin == CIN_PC:
            out.append("ch%d PROGRAM %d" % (ch, d1))
        elif cin == CIN_PB:
            out.append("ch%d BEND %d" % (ch, d1 | (d2 << 7)))
        else:
            out.append("raw %02x %02x %02x %02x" % (data[i], st, d1, d2))
    return out


def panic_packets(channels=range(16), all_notes_off=True, sound_off=True):
    """CC123 (all notes off) + CC120 (all sound off) on each channel."""
    out = bytearray()
    for ch in channels:
        if all_notes_off:
            out += pkt_cc(ch, 123, 0)
        if sound_off:
            out += pkt_cc(ch, 120, 0)
    return bytes(out)


# --- Discovery ----------------------------------------------------------------
class DeviceInfo(object):
    def __init__(self, sysname, busnum, devnum, product, iface, ep_out, ep_in=0x84, sysdir=""):
        self.sysname, self.busnum, self.devnum = sysname, busnum, devnum
        self.product, self.iface, self.ep_out, self.ep_in = product, iface, ep_out, ep_in
        self.sysdir = sysdir

    @property
    def path(self):
        return "/dev/bus/usb/%03d/%03d" % (self.busnum, self.devnum)

    def __repr__(self):
        return "<FM1 %s %s iface=%d ep=0x%02x>" % (self.sysname, self.path, self.iface, self.ep_out)


def _read(path, default=None):
    try:
        with open(path, "r") as f:
            return f.read().strip()
    except (OSError, IOError):
        return default


def _find_midi_interface(devdir, sysname):
    """Return (interface number, bulk-OUT endpoint address) of the MIDI-streaming
    interface (class 01, subclass 03), falling back to the verified FM-1 layout."""
    try:
        names = sorted(os.listdir(devdir))
    except OSError:
        return DEFAULT_MIDI_IFACE, DEFAULT_EP_OUT, DEFAULT_EP_IN
    for n in names:
        if not n.startswith(sysname + ":"):
            continue
        d = os.path.join(devdir, n)
        if _read(os.path.join(d, "bInterfaceClass")) != "01" or \
           _read(os.path.join(d, "bInterfaceSubClass")) != "03":
            continue
        num = _read(os.path.join(d, "bInterfaceNumber"))
        out_ep, in_ep = None, DEFAULT_EP_IN
        try:
            for e in sorted(os.listdir(d)):
                if not e.startswith("ep_"):
                    continue
                addr = int(_read(os.path.join(d, e, "bEndpointAddress")), 16)
                if _read(os.path.join(d, e, "direction")) == "out":
                    out_ep = addr
                else:
                    in_ep = addr
            if out_ep is not None:
                return int(num, 16), out_ep, in_ep
        except (OSError, ValueError, TypeError):
            pass
    return DEFAULT_MIDI_IFACE, DEFAULT_EP_OUT, DEFAULT_EP_IN


def find_devices(vid=FM1_VID, pid=FM1_PID, sysfs=SYSFS_USB):
    """Scan sysfs for every device matching VID/PID. Never hardcodes the bus."""
    found = []
    try:
        entries = sorted(os.listdir(sysfs))
    except OSError:
        return found
    for name in entries:
        d = os.path.join(sysfs, name)
        v, p = _read(os.path.join(d, "idVendor")), _read(os.path.join(d, "idProduct"))
        if v is None or p is None:
            continue
        try:
            if int(v, 16) != vid or int(p, 16) != pid:
                continue
            bus, dev = int(_read(os.path.join(d, "busnum"))), int(_read(os.path.join(d, "devnum")))
        except (ValueError, TypeError):
            continue
        iface, ep, ep_in = _find_midi_interface(d, name)
        found.append(DeviceInfo(name, bus, dev, _read(os.path.join(d, "product"), "FM-1"),
                                iface, ep, ep_in, d))
    return found


class UsbError(Exception):
    pass


class UsbMidiDevice(object):
    """An open, claimed usbfs handle to the FM-1's MIDI-OUT endpoint."""

    def __init__(self, info, timeout_ms=100):
        self.info, self.timeout_ms = info, timeout_ms
        self.fd = None
        self._claimed = False

    def open(self):
        try:
            self.fd = os.open(self.info.path, os.O_RDWR)
            _ioctl(self.fd, USBDEVFS_CLAIMINTERFACE, struct.pack("I", self.info.iface))
            self._claimed = True
        except OSError as e:
            self.close()
            raise UsbError("open/claim failed: %s" % e)

    def write(self, data):
        if self.fd is None:
            raise UsbError("not open")
        n = len(data) - len(data) % 4
        if n == 0:
            return
        buf = ctypes.create_string_buffer(bytes(data[:n]), n)
        req = struct.pack(_BULK_FMT, self.info.ep_out, n, self.timeout_ms, ctypes.addressof(buf))
        try:
            sent = _ioctl(self.fd, USBDEVFS_BULK, req)
        except OSError as e:
            raise UsbError("bulk write failed: %s" % e)
        if sent >= 0 and sent != n:
            raise UsbError("short write %d/%d" % (sent, n))

    def read(self, timeout_ms=100, length=64):
        """Bulk-IN read of USB-MIDI packets from the FM-1. Returns b"" on timeout."""
        import errno
        if self.fd is None:
            raise UsbError("not open")
        buf = ctypes.create_string_buffer(length)
        req = struct.pack(_BULK_FMT, self.info.ep_in, length, timeout_ms, ctypes.addressof(buf))
        try:
            n = _ioctl(self.fd, USBDEVFS_BULK, req)
        except OSError as e:
            if e.errno == errno.ETIMEDOUT:
                return b""
            raise UsbError("bulk read failed: %s" % e)
        return bytes(buf.raw[:max(0, n)])

    def close(self):
        fd, self.fd = self.fd, None
        if fd is None:
            return
        if self._claimed:
            try:
                _ioctl(fd, USBDEVFS_RELEASEINTERFACE, struct.pack("I", self.info.iface))
            except OSError:
                pass
            self._claimed = False
        try:
            os.close(fd)
        except OSError:
            pass


# --- Transports -----------------------------------------------------------------
class Transport(object):
    """What the sequencer talks to. send() must never raise and never block long."""
    name = "base"

    def __init__(self):
        self.packets_sent = 0
        self.writes = 0
        self.errors = 0
        self.last_error = ""
        self.last_write_us = 0

    @property
    def connected(self):
        return True

    def send(self, data):
        raise NotImplementedError

    def status(self):
        return "connected" if self.connected else "disconnected"

    def start(self):
        pass

    def close(self):
        pass


class UsbTransport(Transport):
    """Real FM-1 over usbfs, with a background reconnect thread."""
    name = "usb"

    def __init__(self, vid=FM1_VID, pid=FM1_PID, sysfs=SYSFS_USB, retry_s=1.0):
        Transport.__init__(self)
        self.vid, self.pid, self.sysfs, self.retry_s = vid, pid, sysfs, retry_s
        self._lock = threading.Lock()
        self._dev = None
        self._stop = threading.Event()
        self._thread = None
        self.info = None

    @property
    def connected(self):
        return self._dev is not None

    def start(self):
        self.try_connect()
        if self._thread is None:
            self._thread = threading.Thread(target=self._watch, name="fm1-usb-watch", daemon=True)
            self._thread.start()

    def try_connect(self):
        """Find and open the FM-1 if not already open. Returns True if connected."""
        with self._lock:
            if self._dev is not None:
                return True
            for info in find_devices(self.vid, self.pid, self.sysfs):
                dev = UsbMidiDevice(info)
                try:
                    dev.open()
                except UsbError as e:
                    self.last_error = str(e)
                    continue
                self._dev, self.info = dev, info
                self.last_error = ""
                return True
        return False

    def _watch(self):
        while not self._stop.wait(self.retry_s):
            if self._dev is None:
                self.try_connect()
            else:
                # detect replug (new devnum) even if no write has failed yet
                cur = find_devices(self.vid, self.pid, self.sysfs)
                if not cur or (self.info and cur[0].devnum != self.info.devnum):
                    self._drop("device changed")

    def _drop(self, why):
        with self._lock:
            dev, self._dev = self._dev, None
            self.last_error = why
        if dev:
            dev.close()

    def send(self, data):
        dev = self._dev
        if dev is None or not data:
            return False
        t0 = time.monotonic_ns()
        try:
            with self._lock:
                if self._dev is None:
                    return False
                self._dev.write(data)
        except UsbError as e:
            self.errors += 1
            self._drop(str(e))
            return False
        self.last_write_us = (time.monotonic_ns() - t0) // 1000
        self.writes += 1
        self.packets_sent += len(data) // 4
        return True

    def status(self):
        if self.connected:
            return "FM-1 %s" % (self.info.path if self.info else "")
        return "FM-1 not found" + ((" (%s)" % self.last_error) if self.last_error else "")

    def close(self):
        self._stop.set()
        self._drop("closed")


class MockTransport(Transport):
    """Records everything; used by tests and by the desktop dev run."""
    name = "mock"

    def __init__(self, connected=True):
        Transport.__init__(self)
        self._connected = connected
        self.log = []          # (monotonic_ns, bytes)
        self.fail_next = 0

    @property
    def connected(self):
        return self._connected

    def set_connected(self, v):
        self._connected = v

    def send(self, data):
        if not self._connected or not data:
            return False
        if self.fail_next:
            self.fail_next -= 1
            self._connected = False
            self.errors += 1
            return False
        self.log.append((time.monotonic_ns(), bytes(data)))
        self.writes += 1
        self.packets_sent += len(data) // 4
        return True

    def messages(self):
        out = []
        for _, d in self.log:
            out.extend(decode_packets(d))
        return out

    def status(self):
        return "mock (connected)" if self._connected else "mock (disconnected)"


class ConsoleTransport(MockTransport):
    """Mock that also prints decoded MIDI."""
    name = "console"

    def send(self, data):
        ok = MockTransport.send(self, data)
        if ok:
            for line in decode_packets(data):
                print("[midi] " + line, flush=True)
        return ok


def make_transport(kind="auto"):
    """kind: usb | mock | console | auto (usb on Linux if a usbfs tree exists, else mock)."""
    if kind == "usb":
        return UsbTransport()
    if kind == "mock":
        return MockTransport()
    if kind == "console":
        return ConsoleTransport()
    if os.path.isdir(SYSFS_USB) and os.path.isdir("/dev/bus/usb"):
        return UsbTransport()
    return MockTransport()
