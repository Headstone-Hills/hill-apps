// Phoneme -> synthesizer parameter segments.
//
// Vowel formants are approximate averages for adult male English speakers (Peterson &
// Barney 1952-style values, as commonly tabulated; see Klatt 1980 for the synthesizer these
// parameters drive). Consonant loci, burst frequencies and durations are hand-tuned
// starting points, not measured data: expect to adjust them by ear.

import type { Segment, SyllablePlan, Targets } from './types';

type Formants = [number, number, number];

const VOWELS: Record<string, Formants> = {
  IY: [270, 2290, 3010],
  IH: [390, 1990, 2550],
  EH: [530, 1840, 2480],
  AE: [660, 1720, 2410],
  AA: [730, 1090, 2440],
  AO: [570, 840, 2410],
  UH: [440, 1020, 2240],
  UW: [300, 870, 2240],
  AH: [640, 1190, 2390],
  AX: [500, 1500, 2500], // schwa (unstressed AH0)
  ER: [490, 1350, 1690],
};

/** Diphthongs glide from the first vowel's formants to the second's. */
const DIPHTHONGS: Record<string, [string, string]> = {
  EY: ['EH', 'IY'],
  AY: ['AA', 'IH'],
  AW: ['AA', 'UH'],
  OY: ['AO', 'IH'],
  OW: ['AO', 'UW'],
};

/**
 * Loudness compensation per vowel. A cascade formant synth makes close vowels (IY, UW)
 * quieter than open ones (AA); these factors equalise sustained-vowel RMS, measured by
 * rendering each vowel at 130 Hz.
 */
const VOWEL_GAIN: Record<string, number> = {
  IY: 1.78, IH: 1.8, EH: 1.78, AE: 1.77, AA: 1, AO: 1, UH: 1.62, UW: 1.95, AH: 1.41, AX: 1.43, ER: 1.23,
};

function vowelFormants(sym: string): Partial<Targets> {
  const f = VOWELS[sym] ?? VOWELS.AX;
  return { f1: f[0], f2: f[1], f3: f[2], b1: 60, b2: 90, b3: 150 };
}

/** Vowel phoneme (with stress digit) -> table key. */
function vowelKey(nucleus: string): string {
  const base = nucleus.replace(/\d$/, '');
  if (base === 'AH' && nucleus.endsWith('0')) return 'AX';
  return base;
}

const LOCUS = {
  labial: { f1: 250, f2: 900, f3: 2200 },
  alveolar: { f1: 250, f2: 1700, f3: 2600 },
  velar: { f1: 250, f2: 1500, f3: 2300 },
} as const;

interface StopSpec {
  voiced: boolean;
  locus: keyof typeof LOCUS;
  burstF: number;
  burstBw: number;
  burstAmp: number;
}
const STOPS: Record<string, StopSpec> = {
  P: { voiced: false, locus: 'labial', burstF: 1800, burstBw: 2500, burstAmp: 0.8 },
  B: { voiced: true, locus: 'labial', burstF: 1800, burstBw: 2500, burstAmp: 0.6 },
  T: { voiced: false, locus: 'alveolar', burstF: 4500, burstBw: 1800, burstAmp: 0.5 },
  D: { voiced: true, locus: 'alveolar', burstF: 4500, burstBw: 1800, burstAmp: 0.4 },
  K: { voiced: false, locus: 'velar', burstF: 2300, burstBw: 1000, burstAmp: 0.5 },
  G: { voiced: true, locus: 'velar', burstF: 2300, burstBw: 1000, burstAmp: 0.4 },
};

interface FricSpec {
  voiced: boolean;
  locus: keyof typeof LOCUS;
  ff: number;
  bf: number;
  af: number;
}
const FRICATIVES: Record<string, FricSpec> = {
  S: { voiced: false, locus: 'alveolar', ff: 5800, bf: 1500, af: 0.25 },
  Z: { voiced: true, locus: 'alveolar', ff: 5800, bf: 1500, af: 0.14 },
  SH: { voiced: false, locus: 'alveolar', ff: 2900, bf: 1100, af: 0.48 },
  ZH: { voiced: true, locus: 'alveolar', ff: 2900, bf: 1100, af: 0.28 },
  F: { voiced: false, locus: 'labial', ff: 3500, bf: 3500, af: 0.22 },
  V: { voiced: true, locus: 'labial', ff: 3500, bf: 3500, af: 0.16 },
  TH: { voiced: false, locus: 'alveolar', ff: 4200, bf: 4000, af: 0.16 },
  DH: { voiced: true, locus: 'alveolar', ff: 4200, bf: 4000, af: 0.12 },
};

interface SonorantSpec {
  f: Formants;
  b1?: number;
  av: number;
  nasalZero?: number;
}
const SONORANTS: Record<string, SonorantSpec> = {
  L: { f: [360, 1300, 2700], av: 0.85 },
  R: { f: [420, 1300, 1600], av: 0.85 },
  W: { f: [300, 610, 2200], av: 0.85 },
  Y: { f: [280, 2200, 3000], av: 0.85 },
  M: { f: [250, 1100, 2200], b1: 100, av: 0.85, nasalZero: 1000 },
  N: { f: [250, 1500, 2300], b1: 100, av: 0.85, nasalZero: 1700 },
  NG: { f: [250, 1800, 2400], b1: 100, av: 0.85, nasalZero: 2400 },
};

const MIN_BURST_MS = 8;

export interface SyllableSegments {
  onset: Segment[];
  nucleus: Segment[];
  coda: Segment[];
}

function consonant(
  sym: string,
  position: 'onset' | 'coda',
  nextVowel: Partial<Targets>,
  scale: number,
): Segment[] {
  const stop = STOPS[sym];
  if (stop) {
    const locus = LOCUS[stop.locus];
    const loc: Partial<Targets> = { ...locus, b1: 60, b2: 90, b3: 150 };
    const burst: Partial<Targets> = { ...loc, af: stop.burstAmp, ff: stop.burstF, bf: stop.burstBw };
    if (stop.voiced) {
      return [
        { dur: 40 * scale, trans: 10, t: { ...loc, av: 0.15, ah: 0, af: 0, nz: 0 } },
        { dur: Math.max(MIN_BURST_MS, 9 * scale), trans: 2, t: { ...burst, av: 0.35 } },
      ];
    }
    const segs: Segment[] = [
      { dur: 50 * scale, trans: 10, t: { ...loc, av: 0, ah: 0, af: 0, nz: 0 } },
      { dur: Math.max(MIN_BURST_MS, 12 * scale), trans: 1, t: burst },
    ];
    // Aspiration after a voiceless onset stop, through the vowel's formants: this is also
    // where the audible consonant-to-vowel formant transition happens.
    if (position === 'onset') {
      segs.push({ dur: 30 * scale, trans: 25, t: { ...nextVowel, av: 0, ah: 0.55, af: 0 } });
    } else {
      segs.push({ dur: 14 * scale, trans: 3, t: { ...loc, af: 0, ah: 0.25 } });
    }
    return segs;
  }

  const fric = FRICATIVES[sym];
  if (fric) {
    return [
      {
        dur: (fric.voiced ? 75 : 95) * scale,
        trans: 15,
        t: {
          ...LOCUS[fric.locus], b1: 60, b2: 90, b3: 150,
          av: fric.voiced ? 0.2 : 0, ah: 0, af: fric.af, ff: fric.ff, bf: fric.bf, nz: 0,
        },
      },
    ];
  }

  if (sym === 'CH' || sym === 'JH') {
    const voiced = sym === 'JH';
    const loc: Partial<Targets> = { ...LOCUS.alveolar, b1: 60, b2: 90, b3: 150 };
    return [
      { dur: 35 * scale, trans: 8, t: { ...loc, av: voiced ? 0.15 : 0, ah: 0, af: 0, nz: 0 } },
      {
        dur: 70 * scale,
        trans: 2,
        t: { ...loc, av: voiced ? 0.35 : 0, af: 0.5, ff: 2900, bf: 1100 },
      },
    ];
  }

  if (sym === 'HH') {
    return [{ dur: 60 * scale, trans: 10, t: { ...nextVowel, av: 0, ah: 1.6, af: 0, nz: 0 } }];
  }

  const son = SONORANTS[sym];
  if (son) {
    return [
      {
        dur: (position === 'onset' ? 55 : 75) * scale,
        trans: 20,
        t: {
          f1: son.f[0], f2: son.f[1], f3: son.f[2],
          b1: son.b1 ?? 60, b2: 90, b3: 150,
          av: son.av, ah: 0, af: 0,
          nz: son.nasalZero ? 1 : 0,
          ...(son.nasalZero ? { fz: son.nasalZero } : {}),
        },
      },
    ];
  }
  return []; // unknown symbol: skip
}

const MIN_VOWEL_HOLD_MS = 60;
const FRESH_VOWEL_TRANS_MS = 40;
const LEGATO_VOWEL_TRANS_MS = 80;

function nucleusSegments(nucleus: string, transIn: number): Segment[] {
  const key = vowelKey(nucleus);
  const vowelOn = (k: string) => ({ av: VOWEL_GAIN[k] ?? 1, ah: 0, af: 0, nz: 0 });
  const diph = DIPHTHONGS[key];
  if (diph) {
    return [
      { dur: 100, trans: transIn, t: { ...vowelFormants(diph[0]), ...vowelOn(diph[0]) } },
      { dur: 0, trans: 140, t: { ...vowelFormants(diph[1]), av: VOWEL_GAIN[diph[1]] ?? 1 }, hold: true, minHold: 50 },
    ];
  }
  return [
    {
      dur: 0,
      trans: transIn,
      t: { ...vowelFormants(key), ...vowelOn(key) },
      hold: true,
      minHold: MIN_VOWEL_HOLD_MS,
    },
  ];
}

/** Final fade to silence after the last phoneme. */
export const TAIL_SEGMENT: Segment = { dur: 90, trans: 70, t: { av: 0, ah: 0, af: 0, nz: 0 } };

/**
 * Build the segment lists for one syllable. `legato` omits the onset (no consonant
 * re-articulation) and glides into the new vowel more slowly. `scale` shortens or
 * lengthens consonants (1 = natural-ish; lower puts the vowel closer to the key press).
 */
export function buildSyllable(plan: SyllablePlan, scale: number, legato: boolean): SyllableSegments {
  const nextVowel = vowelFormants(vowelKey(plan.nucleus));
  const onset = legato ? [] : plan.onset.flatMap((s) => consonant(s, 'onset', nextVowel, scale));
  const coda = plan.coda.flatMap((s) => consonant(s, 'coda', nextVowel, scale));
  const nucleus = nucleusSegments(plan.nucleus, legato ? LEGATO_VOWEL_TRANS_MS : FRESH_VOWEL_TRANS_MS);
  // Onset consonants sweep into the vowel over the nucleus transition; stops that already
  // swept during aspiration make that transition short but still smooth.
  return { onset, nucleus, coda };
}

export interface SyllableTiming {
  /** Time the onset consonants take before the vowel starts. */
  onsetSec: number;
  /** Time the coda consonants take after note-off (excluding the final fade). */
  codaSec: number;
  /** Shortest time the vowel is held once reached, even for a tap. */
  minVowelSec: number;
}

/** How long a syllable's consonants take. Used to start consonants early / end them inside a note. */
export function syllableTiming(plan: SyllablePlan, scale: number): SyllableTiming {
  const segs = buildSyllable(plan, scale, false);
  const total = (list: Segment[]): number => list.reduce((t, s) => t + s.dur, 0) / 1000;
  return { onsetSec: total(segs.onset), codaSec: total(segs.coda), minVowelSec: MIN_VOWEL_HOLD_MS / 1000 };
}
