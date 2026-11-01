// Time-domain pitch-synchronous overlap-add (TD-PSOLA): change a recorded sample's pitch and length.
//
// Voiced stretches of the source are cut into two-period grains centred on pitch marks (found from the
// bank's .frq pitch track and snapped to waveform peaks); the grains are re-spaced at the target pitch
// and overlap-added, repeating or skipping grains to change the length. Unvoiced stretches (consonant
// noise, silence) use fixed-size grains at their natural spacing, so they are neither pitch-shifted nor
// made buzzy. This is the simple, real-time-friendly alternative to the WORLD vocoder real UTAU
// resamplers use: good for moderate shifts (about a fourth up or down), rougher beyond that.

import type { Frq } from './files';

export interface PsolaSource {
  samples: Float32Array;
  sampleRate: number;
  frq: Frq | null;
}

export interface SegmentRequest {
  /** Source region in samples: [start, end). */
  start: number;
  end: number;
  /** The first `fixed` samples of the region play at natural speed (the consonant); the rest is stretched to fit. */
  fixed: number;
  /** Output length in samples. */
  length: number;
  /** Target pitch in Hz for voiced parts. */
  f0: number;
  /** Extra gain applied while writing (1 = unity). */
  gain?: number;
  /** Scale the result so the stretched part has the same loudness as the same part of the source (pitch shifting changes it). */
  matchLevel?: boolean;
  /** Override the output-time -> source-position mapping (samples), e.g. to wander back and forth through a vowel. */
  map?: (outSample: number) => number;
}

const UNVOICED_GRAIN = 512;

/** Source f0 (Hz, 0 = unvoiced) at a sample position, from the frame track, or from autocorrelation when there is no .frq. */
function f0At(src: PsolaSource, pos: number): number {
  if (src.frq) {
    const i = Math.min(src.frq.f0.length - 1, Math.max(0, Math.floor(pos / src.frq.interval)));
    return src.frq.f0[i] ?? 0;
  }
  return 0;
}

/** Pitch marks (sample positions of waveform peaks, one per period) over [start, end). Unvoiced gaps get none. */
export function pitchMarks(src: PsolaSource, start: number, end: number): number[] {
  const x = src.samples;
  const marks: number[] = [];
  let pos = start;
  while (pos < end) {
    const hz = f0At(src, pos);
    if (hz <= 0) {
      pos += 64;
      continue;
    }
    const period = src.sampleRate / hz;
    // Snap to the strongest peak within a quarter period either side.
    const lo = Math.max(0, Math.floor(pos - period / 4));
    const hi = Math.min(x.length - 1, Math.ceil(pos + period / 4));
    let best = Math.round(pos);
    let bestV = -1;
    for (let i = lo; i <= hi; i++) {
      const a = Math.abs(x[i]);
      if (a > bestV) {
        bestV = a;
        best = i;
      }
    }
    const last = marks[marks.length - 1];
    if (last === undefined || best - last >= period * 0.6) {
      marks.push(best);
      pos = best + period;
    } else {
      pos += period / 2;
    }
  }
  return marks;
}

/**
 * Render one segment of a sample at a new pitch and length. The consonant part keeps its speed;
 * the vowel part is stretched uniformly. Returns exactly `req.length` samples.
 */
export function renderSegment(src: PsolaSource, req: SegmentRequest): Float32Array {
  const out = new Float32Array(Math.max(0, Math.round(req.length)));
  const x = src.samples;
  const sr = src.sampleRate;
  const a = Math.max(0, Math.min(x.length, req.start));
  const b = Math.max(a + 1, Math.min(x.length, req.end));
  const fixed = Math.max(0, Math.min(req.fixed, b - a - 1));
  const vowelSrc = b - a - fixed;
  const n = out.length;
  if (n === 0) return out;
  const fixedOut = Math.min(fixed, n);
  const vowelOut = Math.max(1, n - fixedOut);
  const gain = req.gain ?? 1;

  // Output time -> source position.
  const srcAt = req.map ?? ((t: number): number => (t < fixedOut ? a + t : a + fixed + ((t - fixedOut) * vowelSrc) / vowelOut));

  const marks = pitchMarks(src, a, b);
  const nearestMark = (pos: number): number => {
    // Binary search for the mark closest to pos.
    let lo = 0, hi = marks.length - 1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (marks[mid] < pos) lo = mid + 1;
      else hi = mid;
    }
    if (lo > 0 && Math.abs(marks[lo - 1] - pos) <= Math.abs(marks[lo] - pos)) lo--;
    return lo;
  };

  const addGrain = (centre: number, half: number, at: number, scale: number): void => {
    // Hann-windowed grain of 2*half samples centred on `centre`, written centred on `at`.
    const len = Math.max(2, Math.round(2 * half));
    for (let k = 0; k < len; k++) {
      const si = Math.round(centre - half) + k;
      const oi = Math.round(at - half) + k;
      if (si < 0 || si >= x.length || oi < 0 || oi >= n) continue;
      const w = 0.5 - 0.5 * Math.cos((2 * Math.PI * (k + 0.5)) / len);
      out[oi] += x[si] * w * scale;
    }
  };

  let t = 0;
  while (t < n) {
    const pos = srcAt(t);
    const hz = marks.length ? f0At(src, pos) : 0;
    if (hz > 0) {
      const mi = nearestMark(pos);
      const centre = marks[mi];
      const ta = mi + 1 < marks.length ? marks[mi + 1] - centre : centre - (marks[mi - 1] ?? centre - sr / hz);
      const analysisPeriod = Math.min(Math.max(ta, sr / 1000), sr / 40);
      const synthPeriod = sr / req.f0;
      addGrain(centre, analysisPeriod, t, (synthPeriod / analysisPeriod) * gain);
      t += synthPeriod;
    } else {
      // Unvoiced / silent: natural-rate grains with 50 % overlap (a Hann window at hop N/2 sums to 1).
      const hop = UNVOICED_GRAIN / 2;
      addGrain(pos, UNVOICED_GRAIN / 2, t, gain);
      t += hop;
    }
  }
  if (req.matchLevel && fixedOut < n) {
    const rmsOf = (v: Float32Array, i0: number, i1: number): number => {
      let s = 0;
      for (let i = i0; i < i1; i++) s += v[i] * v[i];
      return Math.sqrt(s / Math.max(1, i1 - i0));
    };
    const want = rmsOf(x, a + fixed, b) * gain;
    const got = rmsOf(out, fixedOut, n);
    if (want > 1e-6 && got > 1e-6) {
      const k = Math.min(2, Math.max(0.5, want / got));
      for (let i = 0; i < n; i++) out[i] *= k;
    }
  }
  return out;
}

/** In-place version of mixInto: write `seg` into `out` at `at`, fading it in over `fade` samples while the existing sound fades out. */
export function mixAt(out: Float32Array, seg: Float32Array, at: number, fade: number): void {
  const f = Math.max(0, Math.round(fade));
  for (let i = 0; i < seg.length; i++) {
    const oi = at + i;
    if (oi < 0 || oi >= out.length) continue;
    if (i < f) {
      const w = (i / f) * (Math.PI / 2);
      out[oi] = out[oi] * Math.cos(w) + seg[i] * Math.sin(w);
    } else {
      out[oi] += seg[i];
    }
  }
}

/** Equal-power-ish crossfade of `b` into `a` starting at `at`, over `fade` samples. Grows `a` if needed. */
export function mixInto(a: Float32Array, b: Float32Array, at: number, fade: number): Float32Array {
  const start = Math.max(0, Math.round(at));
  const total = Math.max(a.length, start + b.length);
  const out = new Float32Array(total);
  out.set(a);
  const f = Math.max(0, Math.min(Math.round(fade), b.length, a.length - start));
  for (let i = 0; i < b.length; i++) {
    const oi = start + i;
    if (i < f) {
      const w = i / f;
      out[oi] = out[oi] * Math.cos((w * Math.PI) / 2) + b[i] * Math.sin((w * Math.PI) / 2);
    } else {
      out[oi] += b[i];
    }
  }
  return out;
}

/** Fundamental frequency of a steady signal by autocorrelation (for tests and checks). */
export function estimateF0(x: Float32Array, sr: number, minHz = 70, maxHz = 900): number {
  const minLag = Math.floor(sr / maxHz);
  const maxLag = Math.min(Math.floor(sr / minHz), x.length >> 1);
  let best = 0, bestLag = 0;
  for (let lag = minLag; lag <= maxLag; lag++) {
    let s = 0;
    for (let i = 0; i + lag < x.length; i++) s += x[i] * x[i + lag];
    if (s > best) {
      best = s;
      bestLag = lag;
    }
  }
  return bestLag ? sr / bestLag : 0;
}
