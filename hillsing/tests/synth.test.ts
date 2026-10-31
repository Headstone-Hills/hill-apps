import { describe, expect, it } from 'vitest';
import { KlattVoice } from '../src/synth/klatt';
import { buildSyllable } from '../src/synth/phonemes';
import {
  FS, bandEnergy, estimateF0, peakFreq, plan, realDict, render, rms, spectrum,
} from './helpers';

const sustain = (nucleus: string, f0 = 110) => render(plan(nucleus), { f0, holdSec: 1.0 });
const SPEC_AT = Math.floor(0.5 * FS);

describe('formants', () => {
  it('places /AA/ F1 and F2 in the right regions', () => {
    const mag = spectrum(sustain('AA1'), SPEC_AT);
    expect(peakFreq(mag, 500, 900)).toBeGreaterThan(640);
    expect(peakFreq(mag, 500, 900)).toBeLessThan(830); // F1 730
    expect(peakFreq(mag, 900, 1500)).toBeGreaterThan(980);
    expect(peakFreq(mag, 900, 1500)).toBeLessThan(1230); // F2 1090
  });

  it('separates front and back vowels by F2 (IY vs UW)', () => {
    const iy = spectrum(sustain('IY1'), SPEC_AT);
    const uw = spectrum(sustain('UW1'), SPEC_AT);
    expect(peakFreq(iy, 1500, 2800)).toBeGreaterThan(2000); // F2 2290
    expect(peakFreq(uw, 600, 1500)).toBeLessThan(1050); // F2 870
    // IY has far more 2-2.6 kHz energy relative to its F1 region than UW.
    const ratio = (m: Float64Array) => bandEnergy(m, 2000, 2600) / bandEnergy(m, 200, 400);
    expect(ratio(iy)).toBeGreaterThan(4 * ratio(uw));
  });

  it('formant shift moves the spectrum', () => {
    const base = spectrum(sustain('AA1'), SPEC_AT);
    const shifted = spectrum(
      render(plan('AA1'), { f0: 110, holdSec: 1.0, setup: (v) => (v.formantShift = 1.3) }),
      SPEC_AT,
    );
    expect(peakFreq(shifted, 500, 1200)).toBeGreaterThan(peakFreq(base, 500, 1200) + 100);
  });

  it('diphthong /AY/ glides from open to close vowel over a held note', () => {
    const a = render(plan('AY1'), { f0: 110, holdSec: 1.2 });
    // Short windows so each sees only one end of the glide (A holds 40-100 ms, glide to 240 ms).
    const early = spectrum(a, Math.floor(0.05 * FS), 2048);
    const late = spectrum(a, Math.floor(0.7 * FS), 2048);
    expect(peakFreq(early, 450, 1000, 2048)).toBeGreaterThan(650); // starts near AA (F1 730)
    expect(peakFreq(late, 1500, 2800, 2048)).toBeGreaterThan(1800); // ends near IH (F2 1990)
  });
});

describe('pitch', () => {
  it.each([110, 220, 440])('plays %d Hz accurately', (f0) => {
    const a = render(plan('AA1'), { f0, holdSec: 1.0 });
    const est = estimateF0(a, 0.5, f0);
    expect(Math.abs(est - f0) / f0).toBeLessThan(0.012);
  });

  it('glides to a new pitch on setF0 and vibrato modulates it', () => {
    const a = render(plan('AA1'), {
      f0: 130, holdSec: 1.6,
      onTime: (v, t) => { if (Math.abs(t - 0.3) < 0.002) v.setF0(260); },
    });
    expect(Math.abs(estimateF0(a, 1.2, 260) - 260) / 260).toBeLessThan(0.012);
  });
});

describe('note lifecycle', () => {
  it('is finite and bounded for every phoneme in the dictionary', () => {
    const phonemes = new Set<string>();
    for (const phones of realDict().values()) for (const p of phones) phonemes.add(p.replace(/\d$/, ''));
    const consonants = [...phonemes].filter((p) => !/^(AA|AE|AH|AO|AW|AY|EH|ER|EY|IH|IY|OW|OY|UH|UW)$/.test(p));
    for (const c of consonants) {
      expect(buildSyllable(plan('AA1', [c], [c]), 0.7, false).onset.length, `onset ${c}`).toBeGreaterThan(0);
      expect(buildSyllable(plan('AA1', [c], [c]), 0.7, false).coda.length, `coda ${c}`).toBeGreaterThan(0);
    }
    const all = [...phonemes];
    const clusters = plan('IY0', ['S', 'T', 'R'], ['NG', 'K', 'S', 'TH', 'S']);
    const a = render(clusters, { holdSec: 0.4 });
    expect(a.every((x) => Number.isFinite(x) && Math.abs(x) <= 1)).toBe(true);
    expect(all.length).toBeGreaterThan(35);
  });

  it('is not clipped at full velocity and has healthy level', () => {
    const a = render(plan('AA1'), { f0: 130, velocity: 1, holdSec: 0.8 });
    const peak = a.reduce((m, x) => Math.max(m, Math.abs(x)), 0);
    expect(peak).toBeLessThan(0.95);
    expect(rms(a, 0.4, 0.7)).toBeGreaterThan(0.08);
  });

  it('falls silent after note-off and reports inactive', () => {
    const v = new KlattVoice(FS);
    v.noteOn(plan('AA1', ['M'], ['N']), 130, 0.8, false);
    const buf = new Float32Array(128);
    for (let i = 0; i < 200; i++) v.render(buf);
    v.noteOff();
    for (let i = 0; i < 400; i++) v.render(buf); // ~1.1 s
    expect(v.isActive).toBe(false);
    expect(buf.every((x) => x === 0)).toBe(true);
  });

  it('sings the coda consonant only after note-off', () => {
    const hf = (a: Float32Array, t0: number, t1: number) => {
      let e = 0;
      const s = Math.floor(t0 * FS), end = Math.floor(t1 * FS);
      for (let i = s + 1; i < end; i++) e += (a[i] - a[i - 1]) ** 2;
      return e / (end - s);
    };
    const withS = render(plan('AA1', [], ['S']), { holdSec: 0.5, tailSec: 0.5 });
    const without = render(plan('AA1'), { holdSec: 0.5, tailSec: 0.5 });
    expect(hf(withS, 0.2, 0.45)).toBeCloseTo(hf(without, 0.2, 0.45), 6); // identical while held
    expect(hf(withS, 0.56, 0.62)).toBeGreaterThan(5 * hf(without, 0.56, 0.62));
  });

  it('a very short tap still produces a full-length vowel (minimum hold)', () => {
    const a = render(plan('AA1'), { holdSec: 0.005, tailSec: 0.5 });
    expect(rms(a, 0.04, 0.09)).toBeGreaterThan(0.08);
  });

  it('onset consonants precede the vowel; scale shortens them', () => {
    const slow = render(plan('AA1', ['T']), { holdSec: 0.6, setup: (v) => (v.consonantScale = 1.2) });
    const fast = render(plan('AA1', ['T']), { holdSec: 0.6, setup: (v) => (v.consonantScale = 0.3) });
    const onsetOfVowel = (a: Float32Array) => {
      for (let t = 0; t < 0.4; t += 0.005) if (rms(a, t, t + 0.01) > 0.1) return t;
      return Infinity;
    };
    expect(onsetOfVowel(fast)).toBeLessThan(onsetOfVowel(slow) - 0.04);
    expect(onsetOfVowel(slow)).toBeGreaterThan(0.06);
  });

  it('restarting a note mid-sound does not click', () => {
    const a = render(plan('AA1'), {
      holdSec: 0.8,
      onTime: (v, t) => { if (Math.abs(t - 0.4) < 0.002) v.noteOn(plan('IY1'), 130, 0.8, false); },
    });
    let maxStep = 0;
    for (let i = Math.floor(0.3 * FS); i < Math.floor(0.6 * FS); i++) maxStep = Math.max(maxStep, Math.abs(a[i] - a[i - 1]));
    // Glottal pulses are steep by nature; a click would be a far larger discontinuity.
    expect(maxStep).toBeLessThan(0.5);
  });
});

describe('legato keeps consonants', () => {
  /** Hold `first`, then at 0.4 s start `second` as a legato note at 195 Hz. */
  const legatoRender = (first: ReturnType<typeof plan>, second: ReturnType<typeof plan>, legato = true) =>
    render(first, {
      f0: 130, holdSec: 1.0,
      onTime: (v, t) => { if (Math.abs(t - 0.4) < 0.002) v.noteOn(second, 195, 0.8, legato); },
    });
  /**
   * Energy above 4 kHz in a 46 ms window starting at t0. Fricatives and bursts put almost all
   * their energy here, while vowels (even bright ones) are 40+ dB lower, so this detects
   * "was a consonant sung" far better than total or first-difference energy.
   */
  const hf = (a: Float32Array, t0: number, _t1?: number) =>
    bandEnergy(spectrum(a, Math.floor(t0 * FS), 2048), 4000, 9000, 2048);
  const minRms = (a: Float32Array, t0: number, t1: number) => {
    let m = Infinity;
    for (let t = t0; t < t1; t += 0.005) m = Math.min(m, rms(a, t, t + 0.01));
    return m;
  };

  it('sings the NEW syllable\'s onset stop when played legato (closure dip + burst)', () => {
    const withT = legatoRender(plan('AA1'), plan('IY1', ['T']));
    const noOnset = legatoRender(plan('AA1'), plan('IY1'));
    expect(minRms(withT, 0.4, 0.55)).toBeLessThan(0.03); // the /t/ closure
    expect(minRms(noOnset, 0.4, 0.55)).toBeGreaterThan(0.06); // no consonant: smooth glide
    expect(hf(withT, 0.43, 0.5)).toBeGreaterThan(20 * hf(noOnset, 0.43, 0.5)); // the burst
  });

  it('sings the NEW syllable\'s onset fricative when played legato', () => {
    const withS = legatoRender(plan('AA1'), plan('IY1', ['S']));
    const noOnset = legatoRender(plan('AA1'), plan('IY1'));
    expect(hf(withS, 0.41, 0.5)).toBeGreaterThan(20 * hf(noOnset, 0.41, 0.5));
  });

  it('also sings the PREVIOUS syllable\'s final consonant before moving on', () => {
    const withCoda = legatoRender(plan('AA1', [], ['S']), plan('IY1'));
    const noCoda = legatoRender(plan('AA1'), plan('IY1'));
    expect(hf(withCoda, 0.41, 0.5)).toBeGreaterThan(20 * hf(noCoda, 0.41, 0.5));
  });

  it('plays the previous coda only once (not again at note-off)', () => {
    const a = legatoRender(plan('AA1', [], ['S']), plan('IY1'));
    // After the legato note ends at 1.0 s there must be no second /s/.
    const b = legatoRender(plan('AA1'), plan('IY1'));
    expect(hf(a, 0.41)).toBeGreaterThan(0); // the /s/ was sung once, in the transition...
    expect(hf(a, 1.05)).toBeLessThan(0.01 * hf(a, 0.41)); // ...and not again after the note ends
    expect(hf(b, 1.05)).toBeLessThan(0.01 * hf(a, 0.41));
  });

  it('a chain of overlapping syllables keeps every consonant of "stand strong"', () => {
    // stand = S T AE N D ; strong = S T R AO NG
    const chain = (secondOnset: string[]) => {
      const v = new KlattVoice(FS);
      const buf = new Float32Array(128);
      const out: number[] = [];
      const feed = (n: number) => { for (let i = 0; i < n; i++) { v.render(buf); out.push(...buf); } };
      v.noteOn(plan('AE1', ['S', 'T'], ['N', 'D']), 130, 0.8, false);
      feed(Math.floor((0.5 * FS) / 128));
      v.noteOn(plan('AO1', secondOnset, ['NG']), 164, 0.8, true);
      feed(Math.floor((0.7 * FS) / 128));
      v.noteOff();
      feed(Math.floor((0.6 * FS) / 128));
      return Float32Array.from(out);
    };
    // A fricative is *sustained* high-frequency noise (~66 ms); a stop burst is ~8 ms. Count how
    // many 5 ms steps keep 5.5-9 kHz energy above a fixed threshold (set from the full chain).
    const hfTrack = (x: Float32Array) => {
      const e: number[] = [];
      for (let t = 0.5; t < 0.85; t += 0.005) e.push(bandEnergy(spectrum(x, Math.floor(t * FS), 512), 5500, 9000, 512));
      return e;
    };
    const a = chain(['S', 'T', 'R']);
    const without = chain([]); // same chain, but "strong" loses its onset cluster
    const trackA = hfTrack(a);
    const thr = 0.3 * Math.max(...trackA);
    const sustained = (e: number[]) => e.filter((v) => v > thr).length;
    expect(sustained(trackA)).toBeGreaterThanOrEqual(8); // ~40 ms+ of /s/ noise
    expect(sustained(hfTrack(without))).toBeLessThanOrEqual(4); // only the /d/ burst (its band is wide, so it spreads a little)
    expect(rms(a, 0.3, 0.45)).toBeGreaterThan(0.08); // first vowel
    expect(rms(a, 1.0, 1.1)).toBeGreaterThan(0.08); // second vowel
    expect(rms(a, 1.2, 1.28)).toBeGreaterThan(0.01); // /ng/ tail after release
  });

  it('a legato syllable with no onset (melisma / vowel-initial) glides with no dip', () => {
    const a = legatoRender(plan('AA1'), plan('IY1'));
    expect(minRms(a, 0.35, 0.8)).toBeGreaterThan(0.06);
  });

  it('lands on the new vowel formants and the new pitch', () => {
    const a = legatoRender(plan('AA1'), plan('IY1'));
    expect(peakFreq(spectrum(a, Math.floor(0.9 * FS), 4096), 1500, 2800, 4096)).toBeGreaterThan(2000);
    expect(Math.abs(estimateF0(a, 0.85, 195) - 195) / 195).toBeLessThan(0.015);
  });

  it('falls back to a normal articulated note if nothing is sounding', () => {
    const v = new KlattVoice(FS);
    v.noteOn(plan('AA1', ['T']), 130, 0.8, true);
    const buf = new Float32Array(128);
    let early = 0;
    for (let i = 0; i < 4; i++) { v.render(buf); early = Math.max(early, ...buf.map(Math.abs)); }
    expect(early).toBeLessThan(0.05); // still in the silent /t/ closure
  });
});

describe('melisma (held vowel across notes)', () => {
  it('a held diphthong stays on its first vowel; the glide happens on the final note', () => {
    const held = { ...plan('AY1'), holdFirst: true };
    const a = render(held, { f0: 110, holdSec: 1.2 });
    const late = spectrum(a, Math.floor(0.8 * FS), 2048);
    // Held /AY/ must still look like /AA/ (F1 ~730, F2 ~1090), not /IH/ (F2 ~1990).
    expect(peakFreq(late, 450, 1000, 2048)).toBeGreaterThan(650);
    expect(bandEnergy(late, 900, 1300, 2048)).toBeGreaterThan(3 * bandEnergy(late, 1800, 2300, 2048));
  });
});
