import { describe, expect, it } from 'vitest';
import {
  crushBlock, defaultFxState, dbToGain, EFFECTS, formatParam, FxModel, FX_PRESETS, impulseResponse, limiterCurve, mergeFxState,
  sliderToValue, softClipCurve, valueToSlider,
} from '../src/audio/fxparams';

describe('effect definitions', () => {
  it('lists the seven effects in chain order, each parameter with a default inside its range', () => {
    expect(EFFECTS.map((e) => e.id)).toEqual(['bitcrush', 'distortion', 'eq', 'compressor', 'chorus', 'delay', 'reverb']);
    for (const e of EFFECTS) {
      expect(e.params.length).toBeGreaterThan(0);
      for (const q of e.params) {
        expect(q.value, `${e.id}.${q.id}`).toBeGreaterThanOrEqual(q.min);
        expect(q.value, `${e.id}.${q.id}`).toBeLessThanOrEqual(q.max);
        if (q.log) expect(q.min).toBeGreaterThan(0);
      }
    }
  });

  it('starts with every effect off, so the sound is unchanged until something is turned on', () => {
    const s = defaultFxState();
    expect(Object.values(s.enabled).every((on) => on === false)).toBe(true);
  });
});

describe('settings model', () => {
  const memory = () => {
    const m = new Map<string, string>();
    return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
  };

  it('clamps values, ignores unknown names, and fills in what a saved state lacks', () => {
    const s = mergeFxState(defaultFxState(), {
      enabled: { reverb: true, nonsense: true },
      values: { reverb: { size: 99, mix: -3, bogus: 1 }, delay: { time: 500 } },
    });
    expect(s.enabled.reverb).toBe(true);
    expect(s.values.reverb.size).toBe(6); // clamped to the maximum
    expect(s.values.reverb.mix).toBe(0);
    expect(s.values.reverb.damping).toBe(6000); // untouched default
    expect(s.values.delay.time).toBe(500);
    expect('bogus' in s.values.reverb).toBe(false);
    expect(mergeFxState(defaultFxState(), 'garbage')).toEqual(defaultFxState());
  });

  it('saves every change and restores it, notifying listeners', () => {
    const store = memory();
    const m = new FxModel(store);
    const seen: Array<string | null> = [];
    m.onChange((id) => seen.push(id));
    m.setEnabled('delay', true);
    m.setParam('delay', 'time', 480.4);
    m.setParam('delay', 'nope', 1); // unknown parameter: ignored
    expect(seen).toEqual(['delay', 'delay']);
    const again = new FxModel(store);
    expect(again.state.enabled.delay).toBe(true);
    expect(again.state.values.delay.time).toBeCloseTo(480.4);
  });

  it('survives unusable storage', () => {
    const broken = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
    const m = new FxModel(broken);
    expect(() => m.setEnabled('eq', true)).not.toThrow();
    expect(m.state.enabled.eq).toBe(true);
  });

  it('applies presets as complete states, and "clean" turns everything off', () => {
    const m = new FxModel(null);
    for (const pr of FX_PRESETS) {
      m.applyPreset(pr.id);
      for (const e of EFFECTS) for (const q of e.params) expect(m.state.values[e.id][q.id]).toBeGreaterThanOrEqual(q.min);
    }
    m.applyPreset('hall');
    expect(m.state.enabled.reverb).toBe(true);
    m.applyPreset('clean');
    expect(Object.values(m.state.enabled).some(Boolean)).toBe(false);
  });
});

describe('slider mapping and labels', () => {
  const freq = EFFECTS[1].params.find((q) => q.id === 'tone')!;
  const bits = EFFECTS[0].params.find((q) => q.id === 'bits')!;
  it('maps log sliders so the middle is the geometric mean, and round-trips', () => {
    expect(sliderToValue(freq, 0)).toBe(500);
    expect(sliderToValue(freq, 1)).toBe(12000);
    expect(sliderToValue(freq, 0.5)).toBeCloseTo(Math.sqrt(500 * 12000), -1);
    for (const v of [600, 2000, 6000, 11000]) expect(Math.abs(sliderToValue(freq, valueToSlider(freq, v)) - v)).toBeLessThanOrEqual(1);
    expect(sliderToValue(bits, 0.5)).toBe(9); // linear and stepped
  });
  it('formats values for people', () => {
    expect(formatParam(freq, 6000)).toBe('6.0 kHz');
    expect(formatParam(freq, 800)).toBe('800 Hz');
    expect(formatParam(EFFECTS[5].params[3], 0.3)).toBe('30%'); // delay mix
    expect(formatParam(EFFECTS[4].params[0], 0.8)).toBe('0.80 Hz'); // chorus rate
    expect(formatParam(bits, 8)).toBe('8 bits');
  });
});

describe('effect maths', () => {
  it('soft clipping is odd-symmetric, monotonic, and maps full scale to full scale', () => {
    const c = softClipCurve(1025);
    expect(c[0]).toBeCloseTo(-1, 6);
    expect(c[1024]).toBeCloseTo(1, 6);
    expect(c[512]).toBeCloseTo(0, 6);
    for (let i = 1; i < c.length; i++) expect(c[i]).toBeGreaterThanOrEqual(c[i - 1]);
    expect(c[1024 - 100] + c[100]).toBeCloseTo(0, 6);
    expect(c[768]).toBeGreaterThan(0.5 * (768 / 1024) * 2 - 0.5); // saturating: above the straight line near the knee
  });

  it('the output safety curve is unchanged below 0.8, never exceeds full scale, and bends smoothly past it', () => {
    const c = limiterCurve(2049);
    const at = (x: number): number => c[Math.round(((x / 2 + 1) / 2) * 2048)];
    expect(at(0.5)).toBeCloseTo(0.5, 2);
    expect(at(-0.7)).toBeCloseTo(-0.7, 2);
    expect(at(0.9)).toBeLessThan(0.9);
    expect(at(0.9)).toBeGreaterThan(0.8);
    expect(at(2)).toBeLessThanOrEqual(1);
    expect(at(-2)).toBeGreaterThanOrEqual(-1);
    for (let i = 1; i < c.length; i++) expect(c[i]).toBeGreaterThanOrEqual(c[i - 1]);
  });

  it('builds a stereo reverb tail that decays about 60 dB, with pre-delay silence and decorrelated channels', () => {
    const sr = 22050;
    const [l, r] = impulseResponse(sr, 2, 6000, 50, 7);
    expect(l.length).toBe(Math.round(0.05 * sr) + 2 * sr);
    const pre = Math.round(0.05 * sr);
    expect(l.slice(0, pre).every((v) => v === 0)).toBe(true);
    const rms = (x: Float32Array, a: number, b: number) => Math.sqrt(x.slice(a, b).reduce((s, v) => s + v * v, 0) / (b - a));
    const early = rms(l, pre, pre + 2000), late = rms(l, l.length - 2000, l.length);
    expect(20 * Math.log10(late / early)).toBeLessThan(-40);
    let corr = 0;
    for (let i = pre; i < pre + 20000; i++) corr += l[i] * r[i];
    corr /= 20000 * rms(l, pre, pre + 20000) * rms(r, pre, pre + 20000);
    expect(Math.abs(corr)).toBeLessThan(0.1);
    expect(impulseResponse(sr, 1, 6000, 0, 3)[0]).toEqual(impulseResponse(sr, 1, 6000, 0, 3)[0]); // repeatable
  });

  it('a longer reverb is darker at its end when damping is lower', () => {
    const sr = 22050;
    const hf = (x: Float32Array) => { let h = 0; for (let i = 2; i < x.length; i++) { const d = x[i] - 2 * x[i - 1] + x[i - 2]; h += d * d; } return h / x.length; };
    const bright = impulseResponse(sr, 1, 12000, 0, 1)[0];
    const dark = impulseResponse(sr, 1, 1500, 0, 1)[0];
    expect(hf(dark)).toBeLessThan(hf(bright));
  });

  it('bitcrush quantises to the requested number of bits', () => {
    const x = Float32Array.from({ length: 1000 }, (_, i) => Math.sin(i / 20) * 0.9);
    crushBlock(x, 3, 1, { phase: 0, held: 0 });
    const levels = new Set([...x].map((v) => Math.round(v * 4)));
    expect(levels.size).toBeLessThanOrEqual(9); // 3 bits: at most 2^3 + 1 distinct steps
    for (const v of x) expect(Math.abs(v * 4 - Math.round(v * 4))).toBeLessThan(1e-6);
  });

  it('rate reduction holds each value for about `rate` samples, and carries its state across blocks', () => {
    const state = { phase: 0, held: 0 };
    const a = Float32Array.from({ length: 12 }, (_, i) => (i + 1) / 20);
    crushBlock(a, 16, 4, state);
    expect(a[0]).toBe(a[1]); // held
    const changes = a.reduce((n, v, i) => n + (i > 0 && v !== a[i - 1] ? 1 : 0), 0);
    expect(changes).toBeGreaterThanOrEqual(2);
    expect(changes).toBeLessThanOrEqual(4);
    // splitting a signal into blocks gives the same result as one block
    const whole = Float32Array.from({ length: 40 }, (_, i) => Math.sin(i / 3));
    const split = Float32Array.from(whole);
    crushBlock(whole, 6, 3.5, { phase: 0, held: 0 });
    const st = { phase: 0, held: 0 };
    crushBlock(split.subarray(0, 13), 6, 3.5, st);
    crushBlock(split.subarray(13), 6, 3.5, st);
    expect([...split]).toEqual([...whole]);
    // unity settings leave a signal (nearly) untouched
    const clean = Float32Array.from({ length: 100 }, (_, i) => Math.sin(i / 7) * 0.5);
    const ref = Float32Array.from(clean);
    crushBlock(clean, 16, 1, { phase: 0, held: 0 });
    for (let i = 0; i < clean.length; i++) expect(Math.abs(clean[i] - ref[i])).toBeLessThan(1e-4);
  });

  it('converts decibels to gain', () => {
    expect(dbToGain(0)).toBe(1);
    expect(dbToGain(-6.0206)).toBeCloseTo(0.5, 4);
    expect(dbToGain(20)).toBeCloseTo(10, 6);
  });
});
