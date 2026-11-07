import os, sys, tempfile, unittest
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import wavcheck as wc


class WavCheckTests(unittest.TestCase):
    def test_tone_24bit_stereo(self):
        raw = wc.synth_tone(0.5, 440, amp=0.5)
        r = wc.analyze_pcm(raw, 2, 3, 44100)
        self.assertAlmostEqual(r["seconds"], 0.5, places=2)
        self.assertAlmostEqual(r["peak_db"][0], -6.0, delta=0.1)
        self.assertAlmostEqual(r["rms_db"][0], -9.0, delta=0.2)     # sine rms = peak - 3 dB
        self.assertTrue(r["lr_identical"])
        self.assertFalse(r["silent"])

    def test_silence(self):
        r = wc.analyze_pcm(b"\0" * 6 * 4410, 2, 3)
        self.assertTrue(r["silent"])
        self.assertEqual(r["silent_fraction"], 1.0)

    def test_lr_differ(self):
        l = wc.synth_tone(0.1, 440, 1)
        r_ = wc.synth_tone(0.1, 660, 1)
        inter = b"".join(l[i:i + 3] + r_[i:i + 3] for i in range(0, len(l), 3))
        r = wc.analyze_pcm(inter, 2, 3)
        self.assertFalse(r["lr_identical"])
        self.assertFalse(r["lr_near_identical"])

    def test_misaligned_and_empty(self):
        self.assertEqual(wc.analyze_pcm(b"\1" * 7, 2, 3)["misaligned_bytes"], 1)
        self.assertEqual(wc.analyze_pcm(b"", 2, 3)["frames"], 0)

    def test_roundtrip_wav_and_16bit(self):
        raw = wc.synth_tone(0.1, 1000, subframe=2)
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "t.wav")
            wc.write_wav(p, raw, 2, 2)
            raw2, ch, sw, rate = wc.read_wav(p)
        self.assertEqual((raw2, ch, sw, rate), (raw, 2, 2, 44100))
        self.assertAlmostEqual(wc.analyze_pcm(raw, 2, 2)["peak_db"][0], -6.0, delta=0.1)


if __name__ == "__main__":
    unittest.main()
