import { readFileSync } from 'node:fs';
import { parseDict, makeLookup } from '../src/text/cmudict';
import { KlattVoice } from '../src/synth/klatt';
import type { SyllablePlan } from '../src/synth/types';

export const FS = 44100;

let dictCache: Map<string, string[]> | null = null;
export function realDict(): Map<string, string[]> {
  dictCache ??= parseDict(readFileSync(new URL('../public/cmudict.txt', import.meta.url), 'utf8'));
  return dictCache;
}
export const realLookup = () => makeLookup(realDict());

/** Radix-2 FFT magnitude spectrum of a Hann-windowed segment (length must be a power of 2). */
export function spectrum(x: Float32Array, start: number, n = 8192): Float64Array {
  const re = new Float64Array(n);
  const im = new Float64Array(n);
  for (let i = 0; i < n; i++) re[i] = (x[start + i] ?? 0) * (0.5 - 0.5 * Math.cos((2 * Math.PI * i) / (n - 1)));
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) { [re[i], re[j]] = [re[j], re[i]]; [im[i], im[j]] = [im[j], im[i]]; }
  }
  for (let len = 2; len <= n; len <<= 1) {
    const ang = (-2 * Math.PI) / len;
    for (let i = 0; i < n; i += len) {
      for (let k = 0; k < len / 2; k++) {
        const wr = Math.cos(ang * k), wi = Math.sin(ang * k);
        const ur = re[i + k], ui = im[i + k];
        const vr = re[i + k + len / 2] * wr - im[i + k + len / 2] * wi;
        const vi = re[i + k + len / 2] * wi + im[i + k + len / 2] * wr;
        re[i + k] = ur + vr; im[i + k] = ui + vi;
        re[i + k + len / 2] = ur - vr; im[i + k + len / 2] = ui - vi;
      }
    }
  }
  const mag = new Float64Array(n / 2);
  for (let i = 0; i < n / 2; i++) mag[i] = Math.hypot(re[i], im[i]);
  return mag;
}

/** Frequency (Hz) of the strongest bin between lo and hi. */
export function peakFreq(mag: Float64Array, lo: number, hi: number, n = 8192): number {
  let best = 0, bestV = -1;
  for (let k = Math.floor((lo * n) / FS); k <= Math.ceil((hi * n) / FS); k++) {
    if (mag[k] > bestV) { bestV = mag[k]; best = k; }
  }
  return (best * FS) / n;
}

export function bandEnergy(mag: Float64Array, lo: number, hi: number, n = 8192): number {
  let e = 0;
  for (let k = Math.floor((lo * n) / FS); k <= Math.ceil((hi * n) / FS); k++) e += mag[k] * mag[k];
  return e;
}

export const rms = (a: Float32Array, t0: number, t1: number): number => {
  const s = Math.floor(t0 * FS), e = Math.min(a.length, Math.floor(t1 * FS));
  let t = 0;
  for (let i = s; i < e; i++) t += a[i] * a[i];
  return Math.sqrt(t / Math.max(1, e - s));
};

/** Fundamental frequency by normalised autocorrelation with parabolic interpolation. */
export function estimateF0(x: Float32Array, t0: number, expected: number): number {
  const s = Math.floor(t0 * FS), n = 4096;
  const lo = Math.floor(FS / (expected * 1.2)), hi = Math.ceil(FS / (expected * 0.8));
  const ac = (lag: number) => {
    let c = 0;
    for (let i = 0; i < n; i++) c += x[s + i] * x[s + i + lag];
    return c;
  };
  let best = lo, bestV = -Infinity;
  for (let lag = lo; lag <= hi; lag++) { const v = ac(lag); if (v > bestV) { bestV = v; best = lag; } }
  const a = ac(best - 1), b = ac(best), c = ac(best + 1);
  const shift = (0.5 * (a - c)) / (a - 2 * b + c);
  return FS / (best + shift);
}

export interface RenderOpts {
  f0?: number;
  velocity?: number;
  holdSec: number;
  tailSec?: number;
  setup?: (v: KlattVoice) => void;
  /** Called every 128-sample render quantum with the time so far, to inject events. */
  onTime?: (v: KlattVoice, t: number) => void;
}

export function render(plan: SyllablePlan, o: RenderOpts): Float32Array {
  const v = new KlattVoice(FS);
  o.setup?.(v);
  v.noteOn(plan, o.f0 ?? 130, o.velocity ?? 0.8, false);
  const total = Math.floor(((o.holdSec + (o.tailSec ?? 0.6)) * FS) / 128);
  const offAt = Math.floor((o.holdSec * FS) / 128);
  const out = new Float32Array(total * 128);
  const buf = new Float32Array(128);
  for (let i = 0; i < total; i++) {
    if (i === offAt) v.noteOff();
    o.onTime?.(v, (i * 128) / FS);
    v.render(buf);
    out.set(buf, i * 128);
  }
  return out;
}

export const plan = (nucleus: string, onset: string[] = [], coda: string[] = []): SyllablePlan => ({ onset, nucleus, coda });

/** Spectral centroid (Hz) of a band: a smooth measure of where energy sits. */
export function bandCentroid(mag: Float64Array, lo: number, hi: number, n = 8192): number {
  let num = 0;
  let den = 0;
  for (let k = Math.floor((lo * n) / FS); k <= Math.ceil((hi * n) / FS); k++) {
    const f = (k * FS) / n;
    num += f * mag[k] * mag[k];
    den += mag[k] * mag[k];
  }
  return num / den;
}

/**
 * Harmonic-to-noise ratio (dB) of a steady voiced sound: energy at harmonics 2..kMax versus
 * energy midway between them. Drift, jitter and breath noise all lower it.
 */
export function harmonicToNoiseDb(x: Float32Array, start: number, f0: number, kMax = 15, n = 16384): number {
  const mag = spectrum(x, start, n);
  const bin = (f: number) => Math.round((f * n) / FS);
  let h = 0;
  let nz = 0;
  for (let k = 2; k <= kMax; k++) {
    let peak = 0;
    for (let b = bin(k * f0 * 0.98); b <= bin(k * f0 * 1.02); b++) peak = Math.max(peak, mag[b]);
    h += peak * peak;
    const mid = bin((k + 0.5) * f0);
    let m = 0;
    for (let b = mid - 4; b <= mid + 4; b++) m += mag[b] * mag[b];
    nz += m / 9;
  }
  return 10 * Math.log10(h / nz);
}
