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

describe('legato', () => {
  const legatoRender = (second: ReturnType<typeof plan>, secondLegato: boolean) =>
    render(plan('AA1'), {
      f0: 130, holdSec: 1.0,
      onTime: (v, t) => { if (Math.abs(t - 0.4) < 0.002) v.noteOn(second, 195, 0.8, secondLegato); },
    });

  it('glides into the new vowel without a silent consonant gap', () => {
    const a = legatoRender(plan('IY1', ['T']), true);
    let min = Infinity;
    for (let t = 0.35; t < 0.8; t += 0.01) min = Math.min(min, rms(a, t, t + 0.01));
    expect(min).toBeGreaterThan(0.06);
  });

  it('the same syllable played fresh does articulate its onset (control for the test above)', () => {
    const a = legatoRender(plan('IY1', ['T']), false);
    let min = Infinity;
    for (let t = 0.4; t < 0.55; t += 0.005) min = Math.min(min, rms(a, t, t + 0.005));
    expect(min).toBeLessThan(0.03);
  });

  it('lands on the new vowel formants and the new pitch', () => {
    const a = legatoRender(plan('IY1'), true);
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
