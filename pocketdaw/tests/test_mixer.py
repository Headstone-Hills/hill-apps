import os, sys, tempfile, time, unittest
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import numpy as np
import audio_out
from mixer import Mixer, Click, BarTone, Tone, ClipSource, load_wav, resample_linear, RATE
import wavcheck


def render_chunked(m, total, chunk):
    return b"".join(m.render(s, min(chunk, total - s)) for s in range(0, total, chunk))


class MixerTests(unittest.TestCase):
    def make(self):
        m = Mixer()
        m.add(Click(120, 4)); m.add(BarTone(120, 4)); m.add(Tone(333.0, 0.1))
        return m

    def test_chunk_size_does_not_change_the_audio(self):
        total = RATE * 2
        a = render_chunked(self.make(), total, 1024)
        b = render_chunked(self.make(), total, 777)
        c = render_chunked(self.make(), total, 4096)
        self.assertEqual(a, b)
        self.assertEqual(a, c)

    def test_click_lands_on_the_beat_to_the_sample(self):
        m = Mixer(); m.add(Click(120, 4, amp=0.8))
        f = m.render_float(0, RATE * 2)[:, 0]
        loud = np.where(np.abs(f) > 0.05)[0]
        starts = loud[np.concatenate([[True], np.diff(loud) > 100])]
        beat = RATE * 60 // 120                      # 22050
        # a sine burst's first sample is exactly 0, so it is detected at most 1 sample late
        for got, want in zip(starts[:4], [0, beat, 2 * beat, 3 * beat]):
            self.assertIn(int(got) - want, (0, 1))
        self.assertEqual(len(starts), 4)

    def test_clip_offset_and_loop(self):
        data = np.ones((100, 2), dtype=np.float32)
        c = ClipSource(data, offset=1000)
        out = c.render(950, 200, RATE)
        self.assertTrue(np.all(out[:50] == 0) and np.all(out[50:150] == 1) and np.all(out[150:] == 0))
        self.assertIsNone(c.render(0, 500, RATE))
        self.assertIsNone(c.render(2000, 500, RATE))
        lp = ClipSource(data, offset=10, loop=True)
        self.assertIsNone(lp.render(0, 10, RATE))                     # not started yet
        self.assertTrue(np.all(lp.render(5, 10, RATE)[:5] == 0))       # starts mid-chunk
        self.assertTrue(np.all(lp.render(500, 300, RATE) == 1))

    def test_two_clips_stay_aligned(self):
        m = Mixer()
        d = np.zeros((10, 2), dtype=np.float32); d[0] = 0.5
        m.add(ClipSource(d, offset=3000)); m.add(ClipSource(d, offset=3000 + RATE))
        f = m.render_float(0, 2 * RATE)[:, 0]
        self.assertEqual(list(np.nonzero(f)[0]), [3000, 3000 + RATE])

    def test_clipping_and_gain(self):
        m = Mixer(); m.add(Tone(100, 2.0)); m.master = 1.0
        f = m.render_float(0, 1000)
        self.assertLessEqual(float(np.max(np.abs(f))), 1.0)
        m2 = Mixer(); s = m2.add(Tone(100, 0.4)); s.gain = 0.5
        self.assertAlmostEqual(float(np.max(np.abs(m2.render_float(0, 441)))), 0.2, places=2)

    def test_int16_output_length(self):
        self.assertEqual(len(self.make().render(0, 100)), 100 * 4)

    def test_wav_roundtrip_and_resample(self):
        tone = wavcheck.synth_tone(0.2, 440, channels=1, subframe=3, rate=22050)
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "t.wav")
            wavcheck.write_wav(p, tone, 1, 3, 22050)
            a, r = load_wav(p)
        self.assertEqual((a.shape[1], r), (2, 22050))
        self.assertAlmostEqual(float(np.max(a)), 0.5, places=2)
        b = resample_linear(a, 22050, 44100)
        self.assertAlmostEqual(len(b) / len(a), 2.0, places=2)


class OutputTests(unittest.TestCase):
    def test_file_output_is_exact_length(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "o.wav")
            o = audio_out.FileOutput(Mixer(), p, 12345, period_frames=1000)
            o.start(); o._thread.join()
            raw, ch, sw, rate = wavcheck.read_wav(p)
        self.assertEqual((len(raw) // 4, ch, sw, rate), (12345, 2, 2, RATE))

    def test_aplay_backend_paces_and_counts(self):
        m = Mixer(); m.add(Tone(440, 0.2))
        o = audio_out.AplayOutput(m, period_frames=512, cmd=["cat"])
        o.start()
        time.sleep(0.3)
        o.stop()
        st = o.stats()
        self.assertGreater(st["chunks"], 5)
        self.assertEqual(st["errors"], [])
        self.assertEqual(st["frame"], st["chunks"] * 512)

    def test_aplay_backend_survives_dead_player(self):
        o = audio_out.AplayOutput(Mixer(), period_frames=256, cmd=["true"])
        o.start()
        time.sleep(0.3)
        o.stop()
        self.assertTrue(any("aplay pipe" in e for e in o.stats()["errors"]))

    def test_aplay_argv(self):
        o = audio_out.AplayOutput(Mixer(), buffer_ms=80, device="hw:0,0")
        a = o._argv()
        self.assertEqual(a[0], "aplay")
        for x in ("S16_LE", "44100", "--buffer-time=80000", "hw:0,0"):
            self.assertIn(x, a)

    def test_pygame_backend_with_dummy_driver(self):
        try:
            import pygame  # noqa
        except ImportError:
            self.skipTest("pygame not installed")
        os.environ["SDL_AUDIODRIVER"] = "dummy"
        o = audio_out.PygameQueueOutput(Mixer(), period_frames=1024)
        o.start()
        time.sleep(0.5)
        o.stop()
        st = o.stats()
        self.assertGreater(st["chunks"], 3)
        self.assertEqual(st["errors"], [])


if __name__ == "__main__":
    unittest.main()
