// A small FFT and spectral-shape profile, used to find the steadiest stretch of a sample vowel (where it is safe
// to hold it). Pure functions, no Web Audio.

/** In-place radix-2 FFT of power-of-two length. */
export function fft(re: Float64Array, im: Float64Array): void {
  const n = re.length;
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      [re[i], re[j]] = [re[j], re[i]];
      [im[i], im[j]] = [im[j], im[i]];
    }
  }
  for (let len = 2; len <= n; len <<= 1) {
    const ang = (-2 * Math.PI) / len;
    for (let i = 0; i < n; i += len) {
      for (let k = 0; k < len / 2; k++) {
        const wr = Math.cos(ang * k), wi = Math.sin(ang * k);
        const a = i + k, b = i + k + len / 2;
        const vr = re[b] * wr - im[b] * wi, vi = re[b] * wi + im[b] * wr;
        re[b] = re[a] - vr;
        im[b] = im[a] - vi;
        re[a] += vr;
        im[a] += vi;
      }
    }
  }
}

export interface FrameProfile {
  /** Spectral shape: dB in 250 Hz bands from 250 Hz to 3.5 kHz, mean removed (so loudness does not count). */
  shape: number[];
  /** Loudness of the frame, dB. */
  level: number;
}

/** Shape and level of the 1024-sample frame starting at `start`. */
export function frameProfile(x: Float32Array, start: number, sampleRate: number): FrameProfile {
  const N = 1024;
  const re = new Float64Array(N), im = new Float64Array(N);
  let e = 0;
  for (let i = 0; i < N; i++) {
    const v = (x[start + i] ?? 0) * (0.5 - 0.5 * Math.cos((2 * Math.PI * i) / (N - 1)));
    re[i] = v;
    e += v * v;
  }
  fft(re, im);
  const shape: number[] = [];
  for (let f = 250; f < 3500; f += 250) {
    let p = 0, c = 0;
    for (let k = Math.floor((f * N) / sampleRate); k < Math.floor(((f + 250) * N) / sampleRate); k++) {
      p += re[k] * re[k] + im[k] * im[k];
      c++;
    }
    shape.push(10 * Math.log10(p / Math.max(1, c) + 1e-12));
  }
  // Bands more than 35 dB under the strongest are noise-floor: they swing wildly and say nothing about the vowel.
  const top = Math.max(...shape);
  const floored = shape.map((v) => Math.max(v, top - 35));
  const m = floored.reduce((a, b) => a + b, 0) / floored.length;
  return { shape: floored.map((v) => v - m), level: 10 * Math.log10(e / N + 1e-12) };
}

const shapeDistance = (a: number[], b: number[]): number => Math.sqrt(a.reduce((s, v, i) => s + (v - b[i]) ** 2, 0) / a.length);

export interface SteadyWindow {
  /** Sample range inside [start, end). */
  start: number;
  end: number;
  /** How far the vowel's colour moves inside the window (dB of spectral-shape distance from its middle). */
  drift: number;
}

/**
 * The earliest stretch of at least `minSec` in which the vowel keeps its colour and loudness (spectral shape within
 * `maxDriftDb` of the stretch's middle, loudness within 3 dB), or, when the vowel never settles (a diphthong), the
 * stretch of `minSec` that moves least. Holding that stretch instead of a fixed slice keeps "ow" from sliding o-u-o-u.
 */
export function steadyWindow(x: Float32Array, start: number, end: number, sampleRate: number, minSec = 0.12, maxDriftDb = 3): SteadyWindow {
  const hop = 512;
  const frames: Array<{ at: number; p: FrameProfile }> = [];
  for (let a = start; a + 1024 <= end; a += hop) frames.push({ at: a, p: frameProfile(x, a, sampleRate) });
  const need = Math.max(2, Math.ceil((minSec * sampleRate - 1024) / hop) + 1);
  if (frames.length < need) return { start, end, drift: 0 };
  const evaluate = (i: number, n: number): { drift: number; spread: number } => {
    const mid = frames[i + (n >> 1)].p.shape;
    let drift = 0, lo = Infinity, hi = -Infinity;
    for (let k = i; k < i + n; k++) {
      drift = Math.max(drift, shapeDistance(frames[k].p.shape, mid));
      lo = Math.min(lo, frames[k].p.level);
      hi = Math.max(hi, frames[k].p.level);
    }
    return { drift, spread: hi - lo };
  };
  let best = { i: 0, drift: Infinity };
  for (let i = 0; i + need <= frames.length; i++) {
    const r = evaluate(i, need);
    if (r.drift <= maxDriftDb && r.spread <= 3) {
      // earliest steady start; then extend the window as long as it stays steady
      let n = need;
      while (i + n < frames.length) {
        const g = evaluate(i, n + 1);
        if (g.drift > maxDriftDb || g.spread > 3) break;
        n++;
      }
      return { start: frames[i].at, end: frames[i + n - 1].at + 1024, drift: evaluate(i, n).drift };
    }
    if (r.drift < best.drift) best = { i, drift: r.drift };
  }
  return { start: frames[best.i].at, end: frames[best.i + need - 1].at + 1024, drift: best.drift };
}
