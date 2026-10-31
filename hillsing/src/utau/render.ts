// Render a whole sung melody (notes + English syllables) with a UTAU voicebank into one audio buffer.
//
// Each syllable becomes a chain of alias "units" (see planner.ts). A unit's oto.ini entry says where its
// sound starts in the recording (offset), how long its consonant is (kept at natural speed), where the
// beat lands inside it (preutterance) and how long it crossfades with the previous unit (overlap). So a
// unit scheduled for beat time t starts sounding at t - preutterance, and the previous unit is faded out
// over the unit's overlap. The vowel unit is scheduled on the note's start ("vowel on the beat"), onset
// units before it, coda units so they finish inside the note.

import { mixAt, renderSegment, type PsolaSource } from './psola';
import { planUnits, type Unit, xsampaVowel } from './planner';
import type { Voicebank } from './voicebank';
import type { LyricSyllable } from '../text/lyrics';

export interface SungNoteLike {
  /** Seconds. */
  start: number;
  end: number;
  /** MIDI note number. */
  note: number;
  velocity: number;
  /** Index into the syllable list. */
  syllable: number;
  legato: boolean;
}

export interface UtauRender {
  samples: Float32Array;
  sampleRate: number;
  /** Buffer time of score time 0 (room before the first beat for pre-roll sounds). */
  leadSec: number;
  /** One line per word: the aliases that were used, for the on-screen alias display. */
  log: string[];
  /** Aliases the plan wanted but the bank did not have (diagnostics). */
  missing: string[];
}

const GAP_CONNECTED_SEC = 0.12; // notes closer than this flow into each other (vowel carried across)
const MIN_UNIT_SEC = 0.04;
const MAX_CONSONANT_UNIT_SEC = 0.16;
const TAIL_SEC = 0.25;
const hzOf = (note: number): number => 440 * Math.pow(2, (note - 69) / 12);

interface Placed {
  unit: Unit;
  src: PsolaSource;
  region: { start: number; end: number; fixed: number };
  preutter: number; // seconds
  overlap: number; // seconds
  beat: number; // scheduled beat time, seconds (score time)
  f0: number;
  gain: number;
  /** Natural length after the beat, seconds (how long this unit wants to last if nothing follows). */
  natural: number;
}

export async function renderUtau(
  bank: Voicebank,
  notes: ReadonlyArray<SungNoteLike>,
  syllables: ReadonlyArray<LyricSyllable>,
): Promise<UtauRender> {
  const missing = new Set<string>();
  const log: string[] = [];
  const placed: Placed[] = [];
  const has = (a: string): boolean => bank.has(a);
  let sr = 44100;

  // ---- 1. plan units and schedule beats --------------------------------------------------------------------
  const perNote: Placed[][] = [];
  let wordLog: string[] = [];
  let wordIndex = -1;
  for (let k = 0; k < notes.length; k++) {
    const n = notes[k];
    const syl = syllables[n.syllable];
    if (!syl) {
      perNote.push([]);
      continue;
    }
    const prev = k > 0 ? notes[k - 1] : undefined;
    const next = k + 1 < notes.length ? notes[k + 1] : undefined;
    const connectedPrev = !!prev && (n.legato || n.start - prev.end < GAP_CONNECTED_SEC);
    const connectedNext = !!next && (next.legato || next.start - n.end < GAP_CONNECTED_SEC);
    const prevSyl = prev ? syllables[prev.syllable] : undefined;
    const nextSyl = next ? syllables[next.syllable] : undefined;
    const prevVowel = connectedPrev && prevSyl && prevSyl.coda.length === 0 ? xsampaVowel(prevSyl.nucleus) : null;
    const wordEnds = !nextSyl || nextSyl.wordIndex !== syl.wordIndex; // a melisma shares its word's index, so it is not an ending
    const units = planUnits(has, { onset: syl.onset, nucleus: syl.nucleus, coda: syl.coda }, {
      prevVowel,
      releaseAtEnd: wordEnds && !connectedNext,
      melisma: !!syl.melisma,
    });
    if (units.length === 0) {
      // Report what the plan would have needed, for the diagnostics.
      missing.add(`${syl.onset.join('')}${syl.nucleus}${syl.coda.join('')}`);
      perNote.push([]);
      continue;
    }
    const f0 = hzOf(n.note);
    const gain = 0.4 + 0.6 * n.velocity;
    const list: Placed[] = [];
    for (const u of units) {
      const e = bank.entry(u.alias)!;
      const src = await bank.source(e.wavPath);
      if (!src) {
        missing.add(u.alias);
        continue;
      }
      sr = src.sampleRate;
      const region = bank.region(e, src);
      const preutter = Math.max(0, e.preutter / 1000);
      const overlap = Math.max(0.005, e.overlap / 1000);
      const natural = Math.max(MIN_UNIT_SEC, (region.end - region.start) / src.sampleRate - preutter);
      // A join is the previous vowel closing into this syllable, so it keeps the previous note's pitch.
      const unitF0 = u.role === 'join' && prev ? hzOf(prev.note) : f0;
      list.push({ unit: u, src, region, preutter, overlap, beat: 0, f0: unitF0, gain, natural });
    }
    // Schedule: the vowel unit on the note's start; onset units end where it begins; coda units finish inside the note.
    const vi = list.findIndex((p) => p.unit.role === 'vowel');
    if (vi >= 0) {
      list[vi].beat = n.start;
      for (let i = vi - 1, t = n.start; i >= 0; i--) {
        if (list[i].unit.role === 'join') continue; // placed below, from the sound that follows it
        const d = Math.min(MAX_CONSONANT_UNIT_SEC, list[i].natural);
        t -= d;
        list[i].beat = t;
      }
      // The join (previous vowel closing into the first consonant) puts its consonant where the next sample begins.
      const j = list.findIndex((p) => p.unit.role === 'join');
      const after = list.find((p) => p.unit.role !== 'join');
      if (j >= 0 && after) list[j].beat = after.beat - after.preutter;
    }
    const codas = list.filter((p) => p.unit.role === 'coda' || p.unit.role === 'end');
    if (codas.length) {
      const lens = codas.map((p) => Math.min(MAX_CONSONANT_UNIT_SEC, p.natural));
      const total = lens.reduce((a, b) => a + b, 0);
      let t = Math.max(n.start + 0.08, n.end - total);
      codas.forEach((p, i) => {
        p.beat = t;
        t += lens[i];
      });
      // A release ("E -") starts when the note ends rather than inside it.
      for (const p of codas) if (p.unit.role === 'end') p.beat = Math.max(p.beat, n.end - 0.02);
    }
    perNote.push(list);
    // log by word
    if (syl.wordIndex !== wordIndex) {
      if (wordLog.length) log.push(wordLog.join('  '));
      wordLog = [];
      wordIndex = syl.wordIndex;
    }
    wordLog.push(`${syl.label}: ${list.map((p) => p.unit.alias).join(' + ')}`);
  }
  if (wordLog.length) log.push(wordLog.join('  '));
  for (const l of perNote) placed.push(...l);
  placed.sort((a, b) => a.beat - a.preutter - (b.beat - b.preutter));

  // ---- 2. render each unit and crossfade into the buffer ----------------------------------------------------------
  const earliest = placed.length ? Math.min(...placed.map((p) => p.beat - p.preutter)) : 0;
  const lead = Math.max(0, -earliest) + 0.02;
  const latest = placed.length ? Math.max(...placed.map((p) => p.beat + p.natural)) : 0;
  const total = Math.ceil((lead + Math.max(latest, notes.length ? notes[notes.length - 1].end : 0) + TAIL_SEC) * sr);
  const out = new Float32Array(Math.max(1, total));

  for (let i = 0; i < placed.length; i++) {
    const p = placed[i];
    const startT = p.beat - p.preutter + lead;
    const nextP = placed[i + 1];
    // This unit lasts until the next one has fully taken over (its start plus its overlap), or its natural length.
    const naturalEnd = p.beat + p.natural + lead;
    const handOver = nextP ? nextP.beat - nextP.preutter + lead + nextP.overlap : naturalEnd;
    const endT = nextP && nextP.beat - nextP.preutter + lead < naturalEnd + 0.3 ? Math.max(handOver, startT + MIN_UNIT_SEC) : naturalEnd;
    const length = Math.max(Math.round((endT - startT) * sr), Math.round(MIN_UNIT_SEC * sr));
    const seg = renderSegment(p.src, {
      start: p.region.start,
      end: p.region.end,
      fixed: p.region.fixed,
      length,
      f0: p.f0,
      gain: p.gain,
      matchLevel: true,
    });
    // Fade the very end of the phrase's last unit out so it does not click.
    if (!nextP) {
      const f = Math.min(seg.length >> 1, Math.round(0.03 * sr));
      for (let k = 0; k < f; k++) seg[seg.length - 1 - k] *= k / f;
    }
    mixAt(out, seg, Math.round(startT * sr), i === 0 ? 0 : Math.round(p.overlap * sr));
  }

  // Keep the peak below clipping.
  let pk = 0;
  for (let i = 0; i < out.length; i++) pk = Math.max(pk, Math.abs(out[i]));
  if (pk > 0.95) for (let i = 0; i < out.length; i++) out[i] *= 0.95 / pk;
  return { samples: out, sampleRate: sr, leadSec: lead, log, missing: [...missing] };
}
