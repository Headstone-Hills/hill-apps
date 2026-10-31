// Guards the loudness balance between vowels and consonants. The targets are design choices
// (dB relative to a sustained /AA/ at the same velocity), solved numerically, then trimmed
// (NOISE_TRIM in phonemes.ts) and rolled off above ~6.5 kHz (NOISE_ROLLOFF_HZ in klatt.ts) because
// the original noise sounded scratchy when sung; if the synth's
// noise path or gain staging changes, this fails instead of consonants silently getting
// quieter or louder than the vowels.
import { describe, expect, it } from 'vitest';
import { KlattVoice } from '../src/synth/klatt';
import { CLASSIC } from '../src/synth/voices';
import { FS, plan, rms } from './helpers';

function render(p: ReturnType<typeof plan>, hold = 0.6): Float32Array {
  const v = new KlattVoice(FS);
  v.setProfile(CLASSIC);
  v.noteOn(p, 130.8, 0.8, false);
  const out = new Float32Array(Math.floor((hold + 0.2) * FS));
  const buf = new Float32Array(128);
  for (let i = 0; i + 128 <= out.length; i += 128) {
    v.render(buf);
    out.set(buf, i);
  }
  return out;
}
const REF = rms(render(plan('AA1'), 1.0), 0.5, 0.9);
const db = (x: number) => 20 * Math.log10(x / REF);

describe('loudness balance', () => {
  it('the sustained /AA/ reference sits at the design level (RMS 0.19)', () => {
    expect(REF).toBeGreaterThan(0.19 * 0.89);
    expect(REF).toBeLessThan(0.19 * 1.12);
  });

  const fric: Record<string, number> = { S: -15.9, SH: -12, F: -22.6, TH: -24.7, Z: -18.2, ZH: -15.3, V: -19.4, DH: -19.8 };
  for (const [c, target] of Object.entries(fric)) {
    it(`fricative /${c}/ is ${target} dB re the vowel (+-1.5)`, () => {
      expect(db(rms(render(plan('AA1', [c])), 0.025, 0.05))).toBeCloseTo(target, -0.176); // within ~1.5
      expect(Math.abs(db(rms(render(plan('AA1', [c])), 0.025, 0.05)) - target)).toBeLessThan(1.5);
    });
  }

  const burst: Record<string, [number, number, number]> = {
    T: [-11.9, 0.035, 0.0434], K: [-14, 0.035, 0.0434], P: [-13.3, 0.035, 0.0434],
    D: [-12.7, 0.028, 0.037], G: [-12.4, 0.028, 0.037], B: [-13.6, 0.028, 0.037],
  };
  for (const [c, [target, t0, t1]] of Object.entries(burst)) {
    it(`stop burst /${c}/ is ${target} dB re the vowel (+-1.5)`, () => {
      expect(Math.abs(db(rms(render(plan('AA1', [c])), t0, t1)) - target)).toBeLessThan(1.5);
    });
  }

  it('/h/ and the /ch/ affricate are at their targets (+-1.5)', () => {
    expect(Math.abs(db(rms(render(plan('AA1', ['HH'])), 0.02, 0.04)) - -17.1)).toBeLessThan(1.5);
    expect(Math.abs(db(rms(render(plan('AA1', ['CH'])), 0.05, 0.08)) - -18.2)).toBeLessThan(1.5); // affricates are kept 6 dB below the other noise (AFFRICATE_NOISE)
  });

  it('aspiration after voiceless stops stays audible but below the vowel', () => {
    for (const c of ['T', 'K', 'P']) {
      const a = db(rms(render(plan('AA1', [c])), 0.044, 0.07));
      expect(a, c).toBeLessThan(-6);
      expect(a, c).toBeGreaterThan(-24);
    }
  });
});

describe('spectral shape (what makes consonants sound like themselves)', () => {
  it('/s/ noise is high-frequency; /sh/ is lower; neither is a broadband hiss', async () => {
    const { spectrum, bandEnergy } = await import('./helpers');
    const share = (c: string, lo: number, hi: number) => {
      const a = render(plan('AA1', [c]));
      const m = spectrum(a, Math.floor(0.02 * FS), 2048);
      return bandEnergy(m, lo, hi, 2048) / bandEnergy(m, 100, 10000, 2048);
    };
    expect(share('S', 4500, 8000)).toBeGreaterThan(0.6); // /s/: mostly above 4.5 kHz
    expect(share('S', 100, 2500)).toBeLessThan(0.1); // ...and almost nothing below 2.5 kHz
    expect(share('SH', 100, 2000)).toBeLessThan(0.15);
    expect(share('SH', 2000, 5500)).toBeGreaterThan(0.55); // /sh/: centred lower than /s/
    expect(share('SH', 4500, 8000)).toBeLessThan(share('S', 4500, 8000));
  });

  it('vowels keep energy above 4 kHz (not muffled): within 55 dB of the peak at 4-6 kHz', async () => {
    const { spectrum } = await import('./helpers');
    const a = render(plan('AA1'), 1.0);
    const m = spectrum(a, Math.floor(0.4 * FS), 16384);
    const peakDb = (lo: number, hi: number) => {
      let p = 0;
      for (let k = Math.floor((lo * 16384) / FS); k <= Math.ceil((hi * 16384) / FS); k++) p = Math.max(p, m[k]);
      return 20 * Math.log10(p + 1e-12);
    };
    const top = Math.max(peakDb(500, 1000), peakDb(1000, 2000));
    expect(peakDb(4000, 5000) - top).toBeGreaterThan(-55);
    expect(peakDb(5000, 6000) - top).toBeGreaterThan(-63); // (was -60 while vowels were slightly saturated, which added energy up here)
    expect(peakDb(4000, 5000) - top).toBeLessThan(-30); // ...but still falls off like a voice
  });
});
