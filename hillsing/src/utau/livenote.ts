// Building the sound of a live key press from a voicebank (pure DSP, no Web Audio, so it is testable in Node).
//
// A live note has no known length, so it is built in two parts:
//   * the ATTACK: the syllable's consonants and the start of its vowel, played once from the key press
//     (the vowel cannot land "on the beat" live: it follows the consonant, as when a singer starts a word);
//   * a seamless LOOP of the steady part of the vowel, repeated until the key is released.
// On release a RELEASE is built from the syllable's final consonants ("E d") or the vowel-to-silence
// sample ("E -"). Everything is pitch-shifted to the key's pitch with the same TD-PSOLA sampler as melodies.

import { mixAt, renderSegment, type PsolaSource } from './psola';
import { planUnits, type Unit } from './planner';
import type { SyllablePlan } from '../synth/types';
import type { Voicebank } from './voicebank';

export interface LiveAttack {
  samples: Float32Array;
  /** Seamless loop of the steady vowel (null when the sample has no usable vowel part). */
  loop: Float32Array | null;
  sampleRate: number;
  /** The attack's last `crossfade` seconds overlap the start of the loop: start the loop that long before the attack ends. */
  crossfade: number;
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
const MIN_UNIT_SEC = 0.04;
const CROSSFADE_SEC = 0.015;
const LOOP_SEAM_SEC = 0.03;
const MIN_LOOP_SEC = 0.1;
/** The sustain wanders through the steady vowel and back once per SWEEP_SEC, for SWEEP_CYCLES cycles, so nothing repeats at a rate the ear can pick out. */
const SWEEP_SEC = 2.4;
const SWEEP_CYCLES = 3;

interface Prepared {
  unit: Unit;
  src: PsolaSource;
  start: number;
  end: number;
  fixed: number;
  preutter: number; // seconds
  overlap: number; // seconds
  natural: number; // seconds after the beat
}

async function prepare(bank: Voicebank, units: ReadonlyArray<Unit>, skipPre = 0): Promise<Prepared[]> {
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
    if (out.length === 0 && skipPre > 0) {
      // Release units begin with the vowel they came from: skip most of it, since the vowel is already sounding.
      const skip = Math.min(Math.max(0, preutter - skipPre), (r.end - r.start - 2) / src.sampleRate);
      start += Math.round(skip * src.sampleRate);
      fixed = Math.max(0, fixed - Math.round(skip * src.sampleRate));
      preutter -= skip;
    }
    out.push({
      unit: u, src, start, end: r.end, fixed,
      preutter,
      overlap: Math.max(0.005, e.overlap / 1000),
      natural: Math.max(MIN_UNIT_SEC, (r.end - start) / src.sampleRate - preutter),
    });
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

/** Everything up to and including the start of the vowel, plus a loop for sustaining it. */
export async function buildAttack(bank: Voicebank, plan: SyllablePlan, f0: number, prevVowel: string | null): Promise<LiveAttack | null> {
  const units = planUnits((a) => bank.has(a), plan, { prevVowel, releaseAtEnd: false, melisma: false }).filter((u) => u.role !== 'coda' && u.role !== 'end');
  const list = await prepare(bank, units);
  if (list.length === 0) return null;
  const sr = list[0].src.sampleRate;
  const starts = chain(list);
  const vi = Math.max(0, list.findIndex((p) => p.unit.role === 'vowel'));
  const v = list[vi];
  const xs = Math.round(CROSSFADE_SEC * v.src.sampleRate);
  const vowelStart = v.start + v.fixed;
  const vowelLen = v.end - vowelStart;
  let l0 = vowelStart + Math.round(vowelLen * 0.3);
  let l1 = vowelStart + Math.round(vowelLen * 0.85);
  if ((l1 - l0) / v.src.sampleRate < MIN_LOOP_SEC) {
    l0 = vowelStart;
    l1 = v.end;
  }
  const hasLoop = (l1 - l0) / v.src.sampleRate >= MIN_LOOP_SEC / 2 && vowelLen > 0;

  const total = Math.ceil((starts[vi] + (hasLoop ? l0 - v.start + xs : v.end - v.start) / v.src.sampleRate + 0.05) * sr);
  const out = new Float32Array(Math.max(1, total));
  for (let i = 0; i <= vi; i++) {
    const p = list[i];
    let seg: Float32Array;
    if (i === vi) {
      const end = hasLoop ? Math.min(p.end, l0 + xs) : p.end;
      seg = renderSegment(p.src, { start: p.start, end, fixed: p.fixed, length: end - p.start, f0, matchLevel: true });
    } else {
      const handOver = starts[i + 1] + list[i + 1].overlap;
      const length = Math.max(Math.round((handOver - starts[i]) * sr), Math.round(MIN_UNIT_SEC * sr));
      seg = renderSegment(p.src, { start: p.start, end: p.end, fixed: p.fixed, length, f0, matchLevel: true });
    }
    // join units hold the previous vowel: they keep its pitch, so the caller passes the previous f0 through `f0` of the note before; here all units use f0
    mixAt(out, seg, Math.round(starts[i] * sr), i === 0 ? 0 : Math.round(p.overlap * sr));
  }
  let end = out.length;
  while (end > 1 && Math.abs(out[end - 1]) < 1e-5) end--;
  let loop: Float32Array | null = null;
  if (hasLoop) {
    // Instead of repeating a short slice (7-10 repeats a second: audible as a stutter), render a long stretch of
    // vowel whose source position sweeps slowly back and forth across the steady part. Pitch-synchronous grains
    // make the direction change inaudible, and the result only repeats every few seconds.
    const sr = v.src.sampleRate;
    const length = Math.round(SWEEP_SEC * SWEEP_CYCLES * sr);
    const half = Math.round((SWEEP_SEC / 2) * sr);
    const span = l1 - l0;
    const map = (t: number): number => {
      const ph = (t % (2 * half)) / half; // 0..2
      return l0 + (ph <= 1 ? ph : 2 - ph) * span;
    };
    loop = seamlessLoop(renderSegment(v.src, { start: l0, end: l1, fixed: 0, length, f0, matchLevel: true, map }), Math.round(LOOP_SEAM_SEC * sr));
  }
  return { samples: out.subarray(0, end), loop, sampleRate: sr, crossfade: hasLoop ? CROSSFADE_SEC : 0, aliases: list.map((p) => p.unit.alias) };
}

/** The sound after the key is released: the syllable's final consonants, or the vowel dying away. */
export async function buildRelease(bank: Voicebank, plan: SyllablePlan, f0: number): Promise<LiveRelease | null> {
  const units = planUnits((a) => bank.has(a), plan, { prevVowel: null, releaseAtEnd: true, melisma: false }).filter((u) => u.role === 'coda' || u.role === 'end');
  if (units.length === 0) return null;
  const list = await prepare(bank, units, units[0].role === 'end' ? 0.04 : 0.1);
  if (list.length === 0) return null;
  const sr = list[0].src.sampleRate;
  const starts = chain(list);
  const last = list.length - 1;
  const total = Math.ceil((starts[last] + list[last].preutter + list[last].natural + 0.05) * sr);
  const out = new Float32Array(Math.max(1, total));
  list.forEach((p, i) => {
    const naturalEnd = starts[i] + p.preutter + p.natural;
    const handOver = i < last ? starts[i + 1] + list[i + 1].overlap : naturalEnd;
    const length = Math.max(Math.round((Math.min(handOver, naturalEnd + 0.3) - starts[i]) * sr), Math.round(MIN_UNIT_SEC * sr));
    const seg = renderSegment(p.src, { start: p.start, end: p.end, fixed: p.fixed, length, f0, matchLevel: true });
    if (i === last) {
      const f = Math.min(seg.length >> 1, Math.round(0.03 * sr));
      for (let k = 0; k < f; k++) seg[seg.length - 1 - k] *= k / f;
    }
    mixAt(out, seg, Math.round(starts[i] * sr), i === 0 ? 0 : Math.round(p.overlap * sr));
  });
  return { samples: out, sampleRate: sr, overlap: Math.max(0.06, list[0].overlap), aliases: list.map((p) => p.unit.alias) };
}
