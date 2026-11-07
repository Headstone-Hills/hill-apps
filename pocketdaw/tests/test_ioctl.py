import ctypes, os, struct, sys, unittest
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import fm1_usb as usb


class FakeFcntl(object):
    """Behaves like real fcntl.ioctl: int result only for mutable buffers, else a bytes copy."""
    def __init__(self, incoming=b"", fail_with=None):
        self.incoming, self.fail_with, self.calls = incoming, fail_with, []

    def ioctl(self, fd, req, arg=0, mutate=True):
        self.calls.append((req, bytes(arg)))
        if self.fail_with:
            raise self.fail_with
        if req == usb.USBDEVFS_BULK:
            ep, length, _timeout, ptr = struct.unpack(usb._BULK_FMT, bytes(arg))
            if ep & 0x80:
                n = min(length, len(self.incoming))
                ctypes.memmove(ptr, self.incoming[:n], n)
                return n if isinstance(arg, bytearray) else bytes(arg)
            return length if isinstance(arg, bytearray) else bytes(arg)
        return 0 if isinstance(arg, bytearray) else bytes(arg)


def make_dev(fake):
    usb.fcntl, dev = fake, usb.UsbMidiDevice(usb.DeviceInfo("8-1", 8, 2, "FM-1", 4, 4, 0x84))
    dev.fd = 99
    return dev


class IoctlTests(unittest.TestCase):
    def setUp(self):
        self._orig = usb.fcntl

    def tearDown(self):
        usb.fcntl = self._orig

    def test_helper_returns_int_not_bytes(self):
        import termios
        m, s = os.openpty()
        try:
            r = usb._ioctl(m, termios.TIOCGWINSZ, b"\0" * 8)
        finally:
            os.close(m); os.close(s)
        self.assertIsInstance(r, int)

    def test_midi_in_read_returns_data(self):
        pkt = usb.pkt_note_on(0, 60, 100) + usb.pkt_note_off(0, 60)
        dev = make_dev(FakeFcntl(incoming=pkt))
        self.assertEqual(dev.read(10), pkt)
        dev.fd = None

    def test_midi_in_read_timeout_is_empty(self):
        import errno
        dev = make_dev(FakeFcntl(fail_with=OSError(errno.ETIMEDOUT, "t")))
        self.assertEqual(dev.read(10), b"")
        dev.fd = None

    def test_midi_in_read_error_raises(self):
        import errno
        dev = make_dev(FakeFcntl(fail_with=OSError(errno.ENODEV, "gone")))
        with self.assertRaises(usb.UsbError):
            dev.read(10)
        dev.fd = None

    def test_write_detects_short_write(self):
        fake = FakeFcntl()
        orig = fake.ioctl
        fake.ioctl = lambda fd, req, arg=0, mutate=True: 4 if req == usb.USBDEVFS_BULK else orig(fd, req, arg)
        dev = make_dev(fake)
        with self.assertRaises(usb.UsbError):
            dev.write(usb.pkt_note_on(0, 60, 100) * 2)
        dev.fd = None


if __name__ == "__main__":
    unittest.main()
