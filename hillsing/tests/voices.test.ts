import { describe, expect, it } from 'vitest';
import { KlattVoice } from '../src/synth/klatt';
import { CLASSIC, getVoice, VOICES, type VoiceProfile } from '../src/synth/voices';
import { FS, bandCentroid, bandEnergy, harmonicToNoiseDb, peakFreq, plan, rms, spectrum } from './helpers';

const hz = (note: number) => 440 * Math.pow(2, (note - 69) / 12);

function renderWith(
  voice: VoiceProfile,
  p: ReturnType<typeof plan>,
  f0: number,
  holdSec: number,
  setup?: (v: KlattVoice) => void,
  tailSec = 0.4,
): Float32Array {
  const v = new KlattVoice(FS);
  v.setProfile(voice);
  setup?.(v);
  v.noteOn(p, f0, 0.8, false);
  const total = Math.floor(((holdSec + tailSec) * FS) / 128);
  const out = new Float32Array(total * 128);
  const buf = new Float32Array(128);
  for (let i = 0; i < total; i++) {
    if (i === Math.floor((holdSec * FS) / 128)) v.noteOff();
    v.render(buf);
    out.set(buf, i * 128);
  }
  return out;
}

const VOWELS = ['IY', 'IH', 'EH', 'AE', 'AA', 'AO', 'UH', 'UW', 'AH', 'ER'];
const db = (a: number, b: number) => 20 * Math.log10(a / b);

describe('voice profiles', () => {
  it('offers at least six distinct voices with unique ids and names', () => {
    expect(VOICES.length).toBeGreaterThanOrEqual(6);
    expect(new Set(VOICES.map((v) => v.id)).size).toBe(VOICES.length);
    expect(new Set(VOICES.map((v) => v.name)).size).toBe(VOICES.length);
    expect(getVoice('nope')).toBe(CLASSIC);
    expect(getVoice('female').id).toBe('female');
  });

  it('every voice defines every vowel, with sane ordered formants', () => {
    for (const v of VOICES) {
      for (const k of [...VOWELS, 'AX']) {
        const f = v.vowels[k];
        expect(f, `${v.id}/${k}`).toBeDefined();
        expect(f[0], `${v.id}/${k}`).toBeLessThan(f[1]);
        expect(f[1], `${v.id}/${k}`).toBeLessThan(f[2]);
        expect(f[2]).toBeLessThan(4500);
      }
    }
  });

  it('sustained vowels are equally loud (within 1 dB) in every voice at its centre pitch', () => {
    const levels: number[] = [];
    for (const v of VOICES) {
      for (const k of VOWELS) {
        const a = renderWith(v, plan(k + '1'), hz(v.centerNote), 1.0);
        levels.push(rms(a, 0.5, 0.9));
      }
    }
    const mean = levels.reduce((s, x) => s + x, 0) / levels.length;
    for (const l of levels) expect(Math.abs(db(l, mean))).toBeLessThan(1.0);
  });

  it('no voice clips or produces NaN, even on consonant-heavy syllables at a high pitch', () => {
    for (const v of VOICES) {
      const a = renderWith(v, plan('AY1', ['S', 'T', 'R'], ['NG', 'K', 'S']), hz(v.centerNote + 12), 0.5);
      expect(a.every((x) => Number.isFinite(x) && Math.abs(x) <= 1), v.id).toBe(true);
      expect(a.reduce((m, x) => Math.max(m, Math.abs(x)), 0), v.id).toBeLessThan(0.97);
    }
  });

  it('formants follow vocal-tract size: /AA/ F1 region rises deep < classic < female < child', () => {
    const where = (id: string) => {
      const a = renderWith(getVoice(id), plan('AA1'), 110, 1.0);
      return bandCentroid(spectrum(a, Math.floor(0.5 * FS)), 350, 1400);
    };
    const [deep, classic, female, child] = ['deep', 'classic', 'female', 'child'].map(where);
    expect(deep).toBeLessThan(classic - 30);
    expect(classic).toBeLessThan(female - 30);
    expect(female).toBeLessThan(child - 30);
  });

  it('consonants scale with the voice: /s/ noise sits higher for a child than for a deep voice', () => {
    const centroid = (id: string) => {
      const a = renderWith(getVoice(id), plan('AA1', ['S']), 130, 0.6);
      const m = spectrum(a, Math.floor(0.01 * FS), 2048);
      let num = 0, den = 0;
      for (let k = 0; k < m.length; k++) { const f = (k * FS) / 2048; if (f > 2500) { num += f * m[k]; den += m[k]; } }
      return num / den;
    };
    expect(centroid('child')).toBeGreaterThan(centroid('deep') + 300);
  });

  it('bandwidth scale does what it says: narrower formants give a sharper F1 peak (same voice otherwise)', () => {
    const sharp = (bw: number) => {
      const a = renderWith({ ...CLASSIC, bandwidthScale: bw }, plan('AA1'), 110, 1.0);
      const m = spectrum(a, Math.floor(0.5 * FS));
      return bandEnergy(m, 650, 810) / bandEnergy(m, 1250, 1600);
    };
    expect(sharp(0.6)).toBeGreaterThan(sharp(1.0));
    expect(sharp(1.0)).toBeGreaterThan(sharp(1.4));
  });

  it('harmonic-to-noise ratio orders robot (steady, clean) > classic > breathy (airy)', () => {
    const hnr = (id: string) => harmonicToNoiseDb(renderWith(getVoice(id), plan('AA1'), 110, 1.4), Math.floor(0.6 * FS), 110);
    const [robot, classic, breathy] = ['robot', 'classic', 'breathy'].map(hnr);
    expect(robot).toBeGreaterThan(classic);
    expect(classic).toBeGreaterThan(breathy + 3);
  });

  it('robot has no pitch drift: its pitch is steady across a long note', () => {
    const f0s = (id: string) => {
      const a = renderWith(getVoice(id), plan('AA1'), 130, 3.0, undefined, 0.1);
      const est = (t: number) => {
        const m = spectrum(a, Math.floor(t * FS), 16384);
        return peakFreq(m, 3 * 130 * 0.9, 3 * 130 * 1.1, 16384) / 3;
      };
      const vals = [0.5, 1.0, 1.5, 2.0, 2.5].map(est);
      return Math.max(...vals) - Math.min(...vals);
    };
    expect(f0s('robot')).toBeLessThan(0.35);
    expect(f0s('robot')).toBeLessThanOrEqual(f0s('child'));
  });
});

describe('live voice controls', () => {
  const sNoise = (setup: (v: KlattVoice) => void) => {
    const a = renderWith(CLASSIC, plan('AA1', ['S']), 130, 0.6, setup);
    return rms(a, 0.025, 0.05);
  };
  it('consonant level scales consonant noise', () => {
    const base = sNoise(() => {});
    expect(sNoise((v) => (v.consonantLevel = 2)) / base).toBeGreaterThan(1.7);
    expect(sNoise((v) => (v.consonantLevel = 0.5)) / base).toBeLessThan(0.65);
  });
  it('breathiness adds noise between the harmonics', () => {
    const hnr = (setup: (v: KlattVoice) => void) =>
      harmonicToNoiseDb(renderWith(CLASSIC, plan('AA1'), 110, 1.4, setup), Math.floor(0.6 * FS), 110);
    const base = hnr(() => {});
    expect(hnr((v) => (v.breathiness = 0.3))).toBeLessThan(base - 4);
    expect(hnr((v) => (v.breathiness = 0.8))).toBeLessThan(hnr((v) => (v.breathiness = 0.3)));
  });
  it('brightness moves energy upward (and darker moves it down)', () => {
    const share = (setup: (v: KlattVoice) => void) => {
      const m = spectrum(renderWith(CLASSIC, plan('AA1'), 130, 1.0, setup), Math.floor(0.5 * FS));
      return bandEnergy(m, 3000, 8000) / bandEnergy(m, 100, 3000);
    };
    const base = share(() => {});
    expect(share((v) => (v.brightness = 1))).toBeGreaterThan(1.5 * base);
    expect(share((v) => (v.brightness = -1))).toBeLessThan(0.7 * base);
  });
  it('switching voice with setProfile changes the next note', () => {
    const v = new KlattVoice(FS);
    const out = (profile: VoiceProfile) => {
      v.setProfile(profile);
      v.stop();
      v.noteOn(plan('IY1'), 110, 0.8, false);
      const o = new Float32Array(FS);
      const buf = new Float32Array(128);
      for (let i = 0; i + 128 <= FS; i += 128) { v.render(buf); o.set(buf, i); }
      return peakFreq(spectrum(o, Math.floor(0.5 * FS)), 1700, 3600);
    };
    expect(out(getVoice('child'))).toBeGreaterThan(out(getVoice('classic')));
  });
});
