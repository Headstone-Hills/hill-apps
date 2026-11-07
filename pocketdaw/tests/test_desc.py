import os, sys, unittest
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import usb_desc as ud


def iface(num, alt, neps, cls=1, sub=2):
    return bytes([9, 4, num, alt, neps, cls, sub, 0, 0])


def as_general():
    return bytes([7, 0x24, 1, 4, 1, 1, 0])


def fmt1(ch, sub, bits, rates):
    b = bytearray([8 + 3 * len(rates), 0x24, 2, 1, ch, sub, bits, len(rates)])
    for r in rates:
        b += bytes([r & 255, (r >> 8) & 255, r >> 16])
    return bytes(b)


def ep_iso(addr, attrs, maxpkt, interval=1):
    return bytes([9, 5, addr, attrs, maxpkt & 255, maxpkt >> 8, interval, 0, 0])


# SYNTHETIC fixture built from the values decoded in the build brief (not a real dump).
# Replace/add tests/fixtures/fm1_descriptors.bin from the device when available.
FM1_LIKE = (
    iface(0, 0, 0, 1, 1) + iface(1, 0, 0) + iface(1, 1, 1) + as_general() + fmt1(2, 3, 24, [44100])
    + ep_iso(0x03, 0x09, 270) + iface(2, 0, 0) + iface(2, 1, 1) + as_general()
    + fmt1(2, 3, 24, [44100]) + ep_iso(0x83, 0x05, 270) + iface(3, 0, 0, 1, 1)
    + iface(4, 0, 2, 1, 3)
)


class DescTests(unittest.TestCase):
    def test_fm1_like(self):
        s = ud.parse_audio_streams(FM1_LIKE)
        self.assertEqual(len(s), 2)
        cap = ud.pick_capture(s)
        self.assertEqual((cap.iface, cap.alt, cap.ep, cap.max_packet), (2, 1, 0x83, 270))
        self.assertEqual((cap.channels, cap.subframe, cap.bits, cap.rates), (2, 3, 24, [44100]))
        self.assertEqual(cap.sync, "async")
        self.assertEqual(cap.direction, "in")
        play = [x for x in s if x.direction == "out"][0]
        self.assertEqual((play.iface, play.ep, play.sync), (1, 0x03, "adaptive"))

    def test_midi_interface_ignored(self):
        self.assertTrue(all(x.iface != 4 for x in ud.parse_audio_streams(FM1_LIKE)))

    def test_generic_mic_prefers_44100_and_small_frame(self):
        mic = (iface(1, 1, 1) + as_general() + fmt1(1, 2, 16, [48000, 44100]) + ep_iso(0x81, 0x05, 100)
               + iface(1, 2, 1) + as_general() + fmt1(2, 2, 16, [48000, 44100]) + ep_iso(0x81, 0x05, 200))
        cap = ud.pick_capture(ud.parse_audio_streams(mic))
        self.assertEqual((cap.alt, cap.channels), (1, 1))

    def test_no_capture_and_truncated(self):
        self.assertIsNone(ud.pick_capture([]))
        ud.parse_audio_streams(FM1_LIKE[:-5])      # must not raise
        ud.parse_audio_streams(b"\x00\x00\xff")


if __name__ == "__main__":
    unittest.main()
