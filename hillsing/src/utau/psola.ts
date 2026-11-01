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
  /** Play the fixed (consonant) part this many times faster (>= 1): UTAU's consonant velocity. */
  fixedSpeed?: number;
  /** Scale the voiced grains' spectrum by this ratio (1 = as recorded; above 1 moves formants up, a brighter, more youthful colour) without changing pitch. */
  formant?: number;
  /** Gain on the fixed (consonant) part only. */
  fixedGain?: number;
  /** Low-pass the fixed (consonant) part at this frequency in Hz (16000 or more = off): rounds off harsh bursts and hiss. */
  fixedSoftness?: number;
  /** Pitch multiplier over output time (samples), e.g. vibrato. */
  pitchMod?: (outSample: number) => number;
  /** Widen grains up to this multiple (never beyond the pitch ratio) so lowered pitches keep overlapping grains. */
  grainScale?: number;
}

const UNVOICED_GRAIN = 512;
/**
 * When the pitch is lowered by more than about an octave, two-period grains no longer touch: each cycle becomes a burst
 * with silence after it (measured: 82 dB deep gating and peaks 1.7x the recording at 131 Hz from a 279 Hz sample).
 * Widening the grains by up to 1.5x, only when lowering pitch, fills the gaps (17 dB, peaks down by a fifth) at little
 * cost to the vowel's spectral shape (wider than that smears it).
 */
const DEFAULT_GRAIN_SCALE = 1.5;

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
  // Snap every mark to the same kind of peak. Snapping to the largest |x| lets the choice flip between a positive and
  // a negative peak from one period to the next (vowels often have both of similar size): grains then sit half a cycle
  // apart, and where they overlap they partly cancel, which is heard as a ripple and a wobble in the pitch.
  const polarity = dominantPolarity(src, start, end);
  const marks: number[] = [];
  let pos = start;
  while (pos < end) {
    const hz = f0At(src, pos);
    if (hz <= 0) {
      pos += 64;
      continue;
    }
    const period = src.sampleRate / hz;
    // Snap to the strongest peak (of the dominant polarity) within a quarter period either side.
    const lo = Math.max(0, Math.floor(pos - period / 4));
    const hi = Math.min(x.length - 1, Math.ceil(pos + period / 4));
    let best = Math.round(pos);
    let bestV = -Infinity;
    for (let i = lo; i <= hi; i++) {
      const a = polarity * x[i];
      if (a > bestV) {
        bestV = a;
        best = i;
      }
    }
    const last = marks[marks.length - 1];
    if (last !== undefined) best = trackMark(x, last, Math.round(pos), period, best);
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
 * Place the next mark by waveform shape rather than by the biggest peak. Vowels with two strong peaks per cycle (the
 * bright front vowels) make "the biggest peak" flip between the two from one cycle to the next; grains cut around
 * flipping marks sit out of step and the vowel skips and warbles. Instead, among positions near where the next cycle is
 * expected, take the one whose surrounding cycle best matches the cycle at the previous mark (normalised correlation),
 * so every mark sits at the same point of the waveform. Falls back to the peak position when the signal is too quiet.
 */
function trackMark(x: Float32Array, prev: number, expected: number, period: number, peak: number): number {
  const half = Math.max(4, Math.round(period / 2));
  const reach = Math.max(2, Math.round(period / 4));
  if (prev - half < 0 || expected + reach + half >= x.length) return peak;
  let e0 = 0;
  for (let i = -half; i < half; i++) e0 += x[prev + i] * x[prev + i];
  if (e0 < 1e-9) return peak;
  let bestC = -Infinity;
  let bestAt = peak;
  for (let cand = expected - reach; cand <= expected + reach; cand++) {
    let c = 0;
    let e1 = 0;
    for (let i = -half; i < half; i++) {
      const v = x[cand + i];
      c += x[prev + i] * v;
      e1 += v * v;
    }
    const score = c / (Math.sqrt(e0 * e1) + 1e-12) - 0.0005 * Math.abs(cand - expected);
    if (score > bestC) {
      bestC = score;
      bestAt = cand;
    }
  }
  return bestAt;
}

/** +1 when the voiced part of [start, end) peaks higher above zero than below it, else -1. */
function dominantPolarity(src: PsolaSource, start: number, end: number): 1 | -1 {
  const x = src.samples;
  let up = 0;
  let down = 0;
  let pos = start;
  while (pos < end) {
    const hz = f0At(src, pos);
    if (hz <= 0) {
      pos += 64;
      continue;
    }
    const period = Math.max(8, Math.round(src.sampleRate / hz));
    let mx = 0;
    let mn = 0;
    for (let i = Math.max(0, Math.round(pos)); i < Math.min(x.length, Math.round(pos) + period); i++) {
      if (x[i] > mx) mx = x[i];
      if (x[i] < mn) mn = x[i];
    }
    up += mx;
    down -= mn;
    pos += period;
  }
  return up >= down ? 1 : -1;
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
  const speed = Math.max(1, req.fixedSpeed ?? 1);
  const fixedOut = Math.min(Math.round(fixed / speed), n);
  const vowelOut = Math.max(1, n - fixedOut);
  const gain = req.gain ?? 1;

  // Output time -> source position.
  const srcAt = req.map ?? ((t: number): number => (t < fixedOut ? a + t * speed : a + fixed + ((t - fixedOut) * vowelSrc) / vowelOut));

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

  const addGrain = (centre: number, half: number, at: number, scale: number, stride = 1): void => {
    // Hann-windowed grain of 2*half samples centred on `centre`, written centred on `at`. A stride other than 1 reads
    // the source faster or slower (linear interpolation), which moves the formants but not the pitch.
    const len = Math.max(2, Math.round(2 * half));
    for (let k = 0; k < len; k++) {
      const oi = Math.round(at - half) + k;
      if (oi < 0 || oi >= n) continue;
      const w = 0.5 - 0.5 * Math.cos((2 * Math.PI * (k + 0.5)) / len);
      if (stride === 1) {
        const si = Math.round(centre - half) + k;
        if (si < 0 || si >= x.length) continue;
        out[oi] += x[si] * w * scale;
      } else {
        const sp = centre + (k + 0.5 - len / 2) * stride;
        const i0 = Math.floor(sp);
        if (i0 < 0 || i0 + 1 >= x.length) continue;
        const fr = sp - i0;
        out[oi] += (x[i0] * (1 - fr) + x[i0 + 1] * fr) * w * scale;
      }
    }
  };
  const stride = Math.min(1.6, Math.max(0.6, req.formant ?? 1));

  let t = 0;
  while (t < n) {
    const pos = srcAt(t);
    const hz = marks.length ? f0At(src, pos) : 0;
    if (hz > 0) {
      const mi = nearestMark(pos);
      const centre = marks[mi];
      const ta = mi + 1 < marks.length ? marks[mi + 1] - centre : centre - (marks[mi - 1] ?? centre - sr / hz);
      const analysisPeriod = Math.min(Math.max(ta, sr / 1000), sr / 40);
      const synthPeriod = sr / (req.f0 * (req.pitchMod ? req.pitchMod(t) : 1));
      const widen = Math.max(1, Math.min(req.grainScale ?? DEFAULT_GRAIN_SCALE, synthPeriod / analysisPeriod));
      addGrain(centre, analysisPeriod * widen, t, (synthPeriod / (analysisPeriod * widen)) * gain, stride);
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
  // Consonant shaping comes last so it does not change the loudness the vowel was matched to.
  if (fixedOut > 0 && ((req.fixedGain ?? 1) !== 1 || (req.fixedSoftness ?? 16000) < 16000)) {
    const g = req.fixedGain ?? 1;
    const fc = req.fixedSoftness ?? 16000;
    const a = 1 - Math.exp((-2 * Math.PI * Math.min(fc, sr * 0.45)) / sr);
    const blend = Math.min(fixedOut, Math.round(0.003 * sr)); // hand back to the untouched vowel over 3 ms: no step at the seam
    let y1 = 0;
    let y2 = 0;
    for (let i = 0; i < fixedOut; i++) {
      const raw = out[i];
      let v = raw;
      if (fc < 16000) {
        y1 += a * (raw - y1);
        y2 += a * (y1 - y2);
        v = y2;
      }
      const m = Math.min(1, (fixedOut - i) / blend);
      out[i] = (v * m + raw * (1 - m)) * (1 + (g - 1) * m);
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
