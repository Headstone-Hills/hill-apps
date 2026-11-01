// Building the sound of a live key press from a voicebank (pure DSP, no Web Audio, so it is testable in Node).
//
// A live note has no known length, so it is built in two parts:
//   * the ATTACK: the syllable's consonants and the start of its vowel, played once from the key press
//     (the vowel cannot land "on the beat" live: it follows the consonant, as when a singer starts a word);
//   * a seamless LOOP of the steady part of the vowel, repeated until the key is released.
// On release a RELEASE is built from the syllable's final consonants ("E d") or the vowel-to-silence
// sample ("E -"). Everything is pitch-shifted to the key's pitch with the same TD-PSOLA sampler as melodies.

import { mixAt, renderSegment, type PsolaSource } from './psola';
import { steadyWindow } from './spectral';
import { planUnits, type Unit } from './planner';
import type { SyllablePlan } from '../synth/types';
import type { Voicebank } from './voicebank';

/** The tuning knobs that reach the sampler: formant colour and consonant level and tone. */
export interface Shaping {
  formant?: number;
  fixedGain?: number;
  fixedSoftness?: number;
}

export interface LiveAttack {
  samples: Float32Array;
  /**
   * The sustain: seven seconds of the steady vowel (null when the sample has none). Built the first time it is read,
   * so a caller can start the attack sounding first and build the sustain while it plays.
   */
  readonly loop: Float32Array | null;
  /** Whether there is a sustain (known without building it). */
  hasLoop: boolean;
  sampleRate: number;
  /** The attack's last `crossfade` seconds overlap the start of the loop: start the loop that long before the attack ends. */
  crossfade: number;
  /** Loudness (RMS) of the sustain, so the release can be matched to it. */
  readonly loopRms: number;
  /**
   * Seconds into the sustain at which to start playing it so its waveform lines up with the end of the attack
   * (under a crossfade, two waveforms out of step cancel and the level dips). The sustain loops, so any offset is fine.
   */
  readonly loopOffset: number;
  aliases: string[];
}

export interface LiveRelease {
  samples: Float32Array;
  sampleRate: number;
  /** How long the sound before it should fade out, seconds. */
  overlap: number;
  aliases: string[];
}

const MAX_CONSONANT_UNIT_SEC = 0.16;
/**
 * Live playing starts a word with its consonant, so the vowel arrives after it. Playing the consonants 1.5x faster
 * (what UTAU's velocity does) brings the vowel in sooner without changing pitch or vowel length.
 */
export const LIVE_CONSONANT_SPEED = 1.5;
/** Sound shorter than this at the start is closure silence (before a "t" or "k" burst): trim it, it only feels like lag. */
const MAX_LEAD_SILENCE_SEC = 0.006;
const KEEP_LEAD_SEC = 0.003;
/** The first milliseconds ramp in so a sample cut mid-sound does not click. */
const ATTACK_FADE_IN_SEC = 0.003;
/** The very end of a final consonant fades in a few ms only: its burst sits right at the end of the sample. */
const CODA_END_FADE_SEC = 0.01;
const VOWEL_END_FADE_SEC = 0.03;
/** The bank's vowel-to-silence samples fade out in about 120 ms; stretched by this much (pitch unchanged) a word dies away more gently. */
const END_FADE_STRETCH = 1.7;
const MIN_UNIT_SEC = 0.04;
/** How long before a final consonant its sample starts (a live release should reach the consonant quickly). */
const CODA_LEAD_IN_SEC = 0.012;
/** Crossfade from the held vowel into a final consonant's sample; short, so the consonant is not faded in. */
const CODA_CROSSFADE_SEC = 0.02;
/** The silence before a stop's burst (the closure) is cut down to this when releasing a live note. */
const CLOSURE_KEEP_SEC = 0.012;
const CLOSURE_MIN_SEC = 0.025;
const CROSSFADE_SEC = 0.015;
const LOOP_SEAM_SEC = 0.03;
const MIN_LOOP_SEC = 0.1;
/** The sustain wanders through the steady vowel and back once per SWEEP_SEC, for SWEEP_CYCLES cycles, so nothing repeats at a rate the ear can pick out. */
const SWEEP_SEC = 2.4;
const SWEEP_CYCLES = 3;
const SWEEP_FRACTION = 0.5;

interface Prepared {
  unit: Unit;
  src: PsolaSource;
  start: number;
  end: number;
  fixed: number;
  preutter: number; // seconds
  overlap: number; // seconds
  natural: number; // seconds after the beat
  /** Speed-up of the fixed (consonant) part. */
  speed: number;
}

/**
 * Seconds from `start` at which a sample's steady vowel begins to fade (its level drops 3 dB below its opening
 * level), or its whole length if it never does. Ending samples ("oU -") hold the vowel at full level and then
 * die away; a live release should begin just before the fade, because the sustain it follows is already that vowel.
 */
export function decayStartSec(x: Float32Array, sampleRate: number, start: number, end: number): number {
  const frame = Math.round(0.01 * sampleRate);
  const levels: number[] = [];
  for (let a = start; a + frame <= end; a += frame) {
    let e = 0;
    for (let i = a; i < a + frame; i++) e += x[i] * x[i];
    levels.push(20 * Math.log10(Math.sqrt(e / frame) + 1e-6));
  }
  if (levels.length < 8) return (end - start) / sampleRate;
  const head = levels.slice(0, 6).sort((p, q) => p - q);
  const ref = head[3];
  for (let i = 3; i + 1 < levels.length; i++) if (levels[i] < ref - 3 && levels[i + 1] < ref - 3) return (i * frame) / sampleRate;
  return (end - start) / sampleRate;
}

async function prepare(bank: Voicebank, units: ReadonlyArray<Unit>, leadIn = -1, consonantSpeed = 1): Promise<Prepared[]> {
  const out: Prepared[] = [];
  for (const u of units) {
    const e = bank.entry(u.alias);
    if (!e) continue;
    const src = await bank.source(e.wavPath);
    if (!src) continue;
    const r = bank.region(e, src);
    let preutter = Math.max(0, e.preutter / 1000);
    let start = r.start;
    let fixed = r.fixed;
    if (out.length === 0 && leadIn >= 0) {
      // Release units begin with the vowel they came from: skip the steady part (the sustain is already that vowel)
      // and start `leadIn` seconds before the vowel starts to fade or the consonant begins.
      const fade = decayStartSec(src.samples, src.sampleRate, r.start, r.end);
      const skip = Math.min(Math.max(0, fade - leadIn), (r.end - r.start - 2) / src.sampleRate);
      start += Math.round(skip * src.sampleRate);
      fixed = Math.max(0, fixed - Math.round(skip * src.sampleRate));
      preutter -= skip;
    }
    // A faster consonant brings the beat (the preutterance) in with it. Joins carry the previous vowel: left alone.
    const speed = u.role === 'join' ? 1 : Math.max(1, consonantSpeed);
    const natural = Math.max(MIN_UNIT_SEC, (r.end - start) / src.sampleRate - preutter);
    const fixedSec = fixed / src.sampleRate;
    const beat = preutter <= fixedSec ? preutter / speed : fixedSec / speed + (preutter - fixedSec);
    out.push({ unit: u, src, start, end: r.end, fixed, preutter: beat, overlap: Math.max(0.005, e.overlap / 1000), natural, speed });
  }
  return out;
}

/** Sound start time of each unit (seconds from the first unit's start), chained like a melody's units. */
function chain(list: ReadonlyArray<Prepared>): number[] {
  const starts: number[] = [];
  let beat = list[0].preutter;
  list.forEach((p, i) => {
    if (i > 0) beat += Math.min(MAX_CONSONANT_UNIT_SEC, list[i - 1].natural);
    starts.push(beat - p.preutter);
  });
  return starts;
}

/** Scale a buffer down so its largest sample is at most `limit` (never up). Keeps low notes from running past full scale. */
export function capPeak(x: Float32Array, limit = 0.95): void {
  let pk = 0;
  for (let i = 0; i < x.length; i++) pk = Math.max(pk, Math.abs(x[i]));
  if (pk > limit) {
    const k = limit / pk;
    for (let i = 0; i < x.length; i++) x[i] *= k;
  }
}

/**
 * How far into `loop` to start so its waveform best matches `tail` (normalised cross-correlation over `tail`'s length,
 * trying offsets up to a little over one pitch period). The loop wraps, so starting later is harmless.
 */
export function alignOffset(tail: Float32Array, loop: Float32Array, maxLag: number): number {
  let best = 0;
  let bestC = -Infinity;
  const n = Math.min(tail.length, loop.length);
  let et = 0;
  for (let i = 0; i < n; i++) et += tail[i] * tail[i];
  for (let lag = 0; lag <= maxLag; lag++) {
    let c = 0;
    let el = 0;
    for (let i = 0; i < n; i++) {
      const y = loop[(lag + i) % loop.length];
      c += tail[i] * y;
      el += y * y;
    }
    c /= Math.sqrt(et * el) + 1e-12;
    if (c > bestC) {
      bestC = c;
      best = lag;
    }
  }
  return best;
}

/** The loop's end blended into its start, so repeating it has no seam. */
export function seamlessLoop(x: Float32Array, seamSamples: number): Float32Array {
  const seam = Math.max(1, Math.min(seamSamples, x.length >> 2));
  const n = x.length - seam;
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) out[i] = x[i];
  for (let i = 0; i < seam; i++) {
    const w = (i / seam) * (Math.PI / 2);
    out[i] = x[n + i] * Math.cos(w) + x[i] * Math.sin(w);
  }
  return out;
}

/** Everything up to and including the start of the vowel, plus a sustain for holding it. */
export async function buildAttack(
  bank: Voicebank,
  plan: SyllablePlan,
  f0: number,
  prevVowel: string | null,
  opts: { consonantSpeed?: number; /** Fraction of the steady stretch the sustain sweeps across (centred). */ sweepFraction?: number; shaping?: Shaping } = {},
): Promise<LiveAttack | null> {
  const units = planUnits((a) => bank.has(a), plan, { prevVowel, releaseAtEnd: false, melisma: false }).filter((u) => u.role !== 'coda' && u.role !== 'end');
  const list = await prepare(bank, units, -1, opts.consonantSpeed ?? LIVE_CONSONANT_SPEED);
  if (list.length === 0) return null;
  const sr = list[0].src.sampleRate;
  const starts = chain(list);
  const vi = Math.max(0, list.findIndex((p) => p.unit.role === 'vowel'));
  const v = list[vi];
  const xs = Math.round(CROSSFADE_SEC * v.src.sampleRate);
  const vowelStart = v.start + v.fixed;
  const vowelLen = v.end - vowelStart;
  // Hold the steadiest stretch of the vowel (a diphthong settles on its first element), not a fixed slice of it.
  let l0 = vowelStart;
  let l1 = v.end;
  if (vowelLen > 0) {
    const w = steadyWindow(v.src.samples, vowelStart, v.end, v.src.sampleRate);
    // Sweep only the middle half of the steady stretch: the vowel's colour is closest to its centre there, and the
    // attack plays up to the start of it.
    const len = w.end - w.start;
    const minLen = Math.round(MIN_LOOP_SEC * v.src.sampleRate);
    const mid = w.start + len / 2;
    const half = Math.max((len * (opts.sweepFraction ?? SWEEP_FRACTION)) / 2, minLen / 2);
    l0 = Math.max(vowelStart, Math.round(mid - half));
    l1 = Math.min(v.end, Math.round(mid + half));
  }
  const hasLoop = (l1 - l0) / v.src.sampleRate >= MIN_LOOP_SEC / 2 && vowelLen > 0;

  // The vowel unit's length in the output: its consonant part shortened by the speed-up, the rest at natural speed.
  const vEnd = hasLoop ? Math.min(v.end, l0 + xs) : v.end;
  const vowelOut = vEnd - v.start - v.fixed + Math.round(v.fixed / v.speed);
  const total = Math.ceil((starts[vi] + vowelOut / v.src.sampleRate + 0.05) * sr);
  const out = new Float32Array(Math.max(1, total));
  for (let i = 0; i <= vi; i++) {
    const p = list[i];
    let seg: Float32Array;
    if (i === vi) {
      seg = renderSegment(p.src, { start: p.start, end: vEnd, fixed: p.fixed, length: vowelOut, f0, matchLevel: true, fixedSpeed: p.speed, ...opts.shaping });
    } else {
      const handOver = starts[i + 1] + list[i + 1].overlap;
      const length = Math.max(Math.round((handOver - starts[i]) * sr), Math.round(MIN_UNIT_SEC * sr));
      seg = renderSegment(p.src, { start: p.start, end: p.end, fixed: p.fixed, length, f0, matchLevel: true, fixedSpeed: p.speed, ...opts.shaping });
    }
    mixAt(out, seg, Math.round(starts[i] * sr), i === 0 ? 0 : Math.round(p.overlap * sr));
  }
  let end = out.length;
  while (end > 1 && Math.abs(out[end - 1]) < 1e-5) end--;
  // Trim silence at the very start (the closure before a "t" or "k"): it is not a sound, only lag after the key press.
  let peak = 0;
  for (let i = 0; i < end; i++) peak = Math.max(peak, Math.abs(out[i]));
  let first = 0;
  while (first < end && Math.abs(out[first]) < peak * 0.01) first++;
  const trim = first > MAX_LEAD_SILENCE_SEC * sr ? first - Math.round(KEEP_LEAD_SEC * sr) : 0;
  const samples = out.subarray(trim, end);
  const fadeIn = Math.min(samples.length >> 1, Math.round(ATTACK_FADE_IN_SEC * sr));
  for (let k = 0; k < fadeIn; k++) samples[k] *= k / fadeIn;
  capPeak(samples);

  let made = false;
  let loop: Float32Array | null = null;
  let rms = 0;
  let offset = 0;
  const makeLoop = (): void => {
    if (made) return;
    made = true;
    if (!hasLoop) return;
    // Instead of repeating a short slice (7-10 repeats a second: audible as a stutter), render a long stretch of
    // vowel whose source position sweeps slowly back and forth across the steady part. Pitch-synchronous grains
    // make the direction change inaudible, and the result only repeats every few seconds.
    const vsr = v.src.sampleRate;
    const length = Math.round(SWEEP_SEC * SWEEP_CYCLES * vsr);
    const half = Math.round((SWEEP_SEC / 2) * vsr);
    const span = l1 - l0;
    const map = (t: number): number => {
      const ph = (t % (2 * half)) / half; // 0..2
      return l0 + (ph <= 1 ? ph : 2 - ph) * span;
    };
    loop = seamlessLoop(renderSegment(v.src, { start: l0, end: l1, fixed: 0, length, f0, matchLevel: true, map, formant: opts.shaping?.formant }), Math.round(LOOP_SEAM_SEC * vsr));
    capPeak(loop);
    let e = 0;
    for (let i = 0; i < loop.length; i++) e += loop[i] * loop[i];
    rms = Math.sqrt(e / loop.length);
    const xsa = Math.min(samples.length >> 1, Math.round(CROSSFADE_SEC * sr));
    offset = alignOffset(samples.subarray(samples.length - xsa), loop, Math.ceil((1.25 * sr) / f0)) / sr;
  };
  return {
    samples,
    hasLoop,
    sampleRate: sr,
    crossfade: hasLoop ? CROSSFADE_SEC : 0,
    aliases: list.map((p) => p.unit.alias),
    get loop() {
      makeLoop();
      return loop;
    },
    get loopRms() {
      makeLoop();
      return rms;
    },
    get loopOffset() {
      makeLoop();
      return offset;
    },
  };
}

/**
 * Cut the silence between a vowel and the burst of the stop that follows it ("I t" has a closure of 50 ms or more)
 * down to a few milliseconds, so a live release reaches its consonant sooner. Silence that runs to the end of the
 * sample is the tail, not a closure, and is left alone.
 */
export function trimClosure(x: Float32Array, sampleRate: number): Float32Array {
  const frame = Math.max(1, Math.round(0.001 * sampleRate));
  const env: number[] = [];
  for (let s = 0; s + frame <= x.length; s += frame) {
    let e = 0;
    for (let i = s; i < s + frame; i++) e += x[i] * x[i];
    env.push(Math.sqrt(e / frame));
  }
  const lead = env.slice(0, 20).sort((a, b) => a - b)[10] ?? 0;
  const quiet = lead * 0.03;
  let seen = false;
  for (let f = 0; f < env.length; f++) {
    if (env[f] > quiet * 3) seen = true;
    if (!seen || env[f] >= quiet) continue;
    let g = f;
    while (g < env.length && env[g] < quiet) g++;
    const len = (g - f) * 0.001;
    if (g >= env.length - 2) return x; // silent to the end: a tail
    if (len < CLOSURE_MIN_SEC) {
      f = g;
      continue;
    }
    // Remove the middle of the gap, keeping a little silence on each side, with a short crossfade over the join.
    const keepA = f + Math.round((CLOSURE_KEEP_SEC * 1000) / 2);
    const keepB = g - Math.round((CLOSURE_KEEP_SEC * 1000) / 2);
    const cutFrom = keepA * frame;
    const cutTo = keepB * frame;
    const out = new Float32Array(x.length - (cutTo - cutFrom));
    out.set(x.subarray(0, cutFrom));
    out.set(x.subarray(cutTo), cutFrom);
    const fade = Math.min(Math.round(0.002 * sampleRate), cutFrom, out.length - cutFrom);
    for (let i = 0; i < fade; i++) {
      const w = i / fade;
      out[cutFrom - fade + i] = x[cutFrom - fade + i] * (1 - w) + x[cutTo - fade + i] * w;
    }
    return out;
  }
  return x;
}

/** The sound after the key is released: the syllable's final consonants, or the vowel dying away. */
export async function buildRelease(bank: Voicebank, plan: SyllablePlan, f0: number, targetRms = 0, shaping: Shaping = {}): Promise<LiveRelease | null> {
  const units = planUnits((a) => bank.has(a), plan, { prevVowel: null, releaseAtEnd: true, melisma: false }).filter((u) => u.role === 'coda' || u.role === 'end');
  if (units.length === 0) return null;
  const list = await prepare(bank, units, units[0].role === 'end' ? 0.03 : CODA_LEAD_IN_SEC);
  if (list.length === 0) return null;
  for (const p of list) {
    if (p.unit.role !== 'end') continue;
    p.fixed = 0; // let the whole fade stretch, not just the part after the recorded consonant length
    p.natural *= END_FADE_STRETCH;
  }
  const sr = list[0].src.sampleRate;
  const starts = chain(list);
  const last = list.length - 1;
  const total = Math.ceil((starts[last] + list[last].preutter + list[last].natural + 0.05) * sr);
  const out = new Float32Array(Math.max(1, total));
  list.forEach((p, i) => {
    const naturalEnd = starts[i] + p.preutter + p.natural;
    const handOver = i < last ? starts[i + 1] + list[i + 1].overlap : naturalEnd;
    const length = Math.max(Math.round((Math.min(handOver, naturalEnd + 0.3) - starts[i]) * sr), Math.round(MIN_UNIT_SEC * sr));
    const seg = renderSegment(p.src, { start: p.start, end: p.end, fixed: p.fixed, length, f0, matchLevel: true, ...shaping });
    if (i === last) {
      const f = Math.min(seg.length >> 1, Math.round((p.unit.role === 'end' ? VOWEL_END_FADE_SEC : CODA_END_FADE_SEC) * sr));
      for (let k = 0; k < f; k++) seg[seg.length - 1 - k] *= k / f;
    }
    mixAt(out, seg, Math.round(starts[i] * sr), i === 0 ? 0 : Math.round(p.overlap * sr));
  });
  const overlap = units[0].role === 'end' ? Math.max(0.06, list[0].overlap) : CODA_CROSSFADE_SEC;
  const sound = units[0].role === 'end' ? out : trimClosure(out, sr);
  // Match the release's opening (the vowel it continues) to the loudness of the sustain it follows, so the
  // handover has no jump in volume: each recording sits at its own level.
  if (targetRms > 0) {
    const n = Math.min(sound.length, Math.round(Math.max(0.08, overlap) * sr));
    let e = 0;
    for (let i = 0; i < n; i++) e += sound[i] * sound[i];
    const got = Math.sqrt(e / Math.max(1, n));
    if (got > 1e-6) {
      const k = Math.min(2.5, Math.max(0.4, targetRms / got));
      for (let i = 0; i < sound.length; i++) sound[i] *= k;
    }
  }
  return { samples: sound, sampleRate: sr, overlap, aliases: list.map((p) => p.unit.alias) };
}
