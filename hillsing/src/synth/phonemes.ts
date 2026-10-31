// Phoneme -> synthesizer parameter segments.
//
// Consonant noise levels were solved numerically against the targets in tests/levels.test.ts.
// Vowel formants come from the voice profile (see voices.ts). Consonant loci, burst frequencies
// and durations are hand-tuned starting points, not measured data: expect to adjust them by
// ear. See Klatt 1980 for the synthesizer these parameters drive.

import type { Segment, SyllablePlan, Targets } from './types';
import { CLASSIC, type Formants, type VoiceProfile } from './voices';

/** Diphthongs glide from the first vowel's formants to the second's. */
const DIPHTHONGS: Record<string, [string, string]> = {
  EY: ['EH', 'IY'],
  AY: ['AA', 'IH'],
  AW: ['AA', 'UH'],
  OY: ['AO', 'IH'],
  OW: ['AO', 'UW'],
};

export function vowelFormants(sym: string, p: VoiceProfile): Partial<Targets> {
  const f = p.vowels[sym] ?? p.vowels.AX;
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

/** Locus formants for a place of articulation, scaled for the voice's vocal-tract size. */
function locus(place: keyof typeof LOCUS, p: VoiceProfile): Partial<Targets> {
  const l = LOCUS[place];
  return { f1: l.f1, f2: l.f2 * p.sizeScale, f3: l.f3 * p.sizeScale, b1: 60, b2: 90, b3: 150 };
}

export interface StopSpec {
  voiced: boolean;
  locus: keyof typeof LOCUS;
  burstF: number;
  burstBw: number;
  burstAmp: number;
}
/** Exported so the level-calibration test can tune/verify them. */
export const STOPS: Record<string, StopSpec> = {
  P: { voiced: false, locus: 'labial', burstF: 1800, burstBw: 3200, burstAmp: 0.368 },
  B: { voiced: true, locus: 'labial', burstF: 1800, burstBw: 3200, burstAmp: 0.352 },
  T: { voiced: false, locus: 'alveolar', burstF: 4000, burstBw: 3200, burstAmp: 0.552 },
  D: { voiced: true, locus: 'alveolar', burstF: 4000, burstBw: 3200, burstAmp: 0.458 },
  K: { voiced: false, locus: 'velar', burstF: 2300, burstBw: 2400, burstAmp: 0.351 },
  G: { voiced: true, locus: 'velar', burstF: 2300, burstBw: 2400, burstAmp: 0.391 },
};

export interface FricSpec {
  voiced: boolean;
  locus: keyof typeof LOCUS;
  ff: number;
  bf: number;
  af: number;
}
export const FRICATIVES: Record<string, FricSpec> = {
  S: { voiced: false, locus: 'alveolar', ff: 5800, bf: 3200, af: 0.379 },
  Z: { voiced: true, locus: 'alveolar', ff: 5800, bf: 3200, af: 0.256 },
  SH: { voiced: false, locus: 'alveolar', ff: 2900, bf: 2400, af: 0.39 },
  ZH: { voiced: true, locus: 'alveolar', ff: 2900, bf: 2400, af: 0.252 },
  F: { voiced: false, locus: 'labial', ff: 3500, bf: 3500, af: 0.141 },
  V: { voiced: true, locus: 'labial', ff: 3500, bf: 3500, af: 0.169 },
  TH: { voiced: false, locus: 'alveolar', ff: 4200, bf: 4000, af: 0.121 },
  DH: { voiced: true, locus: 'alveolar', ff: 4200, bf: 4000, af: 0.151 },
};

interface SonorantSpec {
  f: Formants;
  b1?: number;
  av: number;
  nasalZero?: number;
}
/**
 * Voicing level of sonorants as a fraction of the vowel's loudness at the same pitch (voicing is
 * matched per formant set, so these are real levels): liquids and glides about -6 dB, nasals about
 * -10 dB. At 0.85 a final /m/ was almost as loud as the vowel and then cut to silence at the next
 * closure, heard as a click at the end of the word.
 */
const LIQUID_LEVEL = 0.5;
const NASAL_LEVEL = 0.32;
const SONORANTS: Record<string, SonorantSpec> = {
  L: { f: [360, 1300, 2700], av: LIQUID_LEVEL },
  R: { f: [420, 1300, 1600], av: LIQUID_LEVEL },
  W: { f: [300, 610, 2200], av: LIQUID_LEVEL },
  Y: { f: [280, 2200, 3000], av: LIQUID_LEVEL },
  M: { f: [250, 1100, 2200], b1: 100, av: NASAL_LEVEL, nasalZero: 1000 },
  N: { f: [250, 1500, 2300], b1: 100, av: NASAL_LEVEL, nasalZero: 1700 },
  NG: { f: [250, 1800, 2400], b1: 100, av: NASAL_LEVEL, nasalZero: 2400 },
};

/** Noise levels that are not per-phoneme specs (also tuned by the level-calibration test). */
export const NOISE_LEVELS = { hhAspiration: 3.34, affricateNoise: 0.39 };

/**
 * Overall trim on every consonant noise source (frication, bursts, aspiration, /h/), about
 * -4 dB. The numerically solved levels above were hard to listen to when sung: sustained notes
 * made each syllable's noise stand out as a scratchy hiss. The Consonant level slider scales
 * this further.
 */
const NOISE_TRIM = 0.8;

/**
 * Scale on voicing during voiced obstruents, applied after the loudness match. Calibrated so that at
 * the classic voice's centre pitch they sit where they did before voiced consonants were matched
 * per formant set (the voicing fractions below were tuned against the vowel gain).
 */
const OBSTRUENT_VOICING = 0.32;

/** Multiplier on consonant noise at the end of a syllable (about -4 dB), and on /ch/ and /j/ (about -6 dB). */
const CODA_NOISE = 0.6;
const AFFRICATE_NOISE = 0.5;

const MIN_BURST_MS = 8;
/** Frication / burst centres cannot usefully exceed this at typical sample rates. */
const MAX_NOISE_HZ = 7000;

/**
 * Loudness gain for a vowel table key at the note's pitch. The synth supplies one that measures
 * the real output (vowel loudness depends on where harmonics fall relative to the formants);
 * the default is neutral, which is what the pure timing calculations use.
 */
export type VowelGain = (vowelKey: string) => number;

/**
 * Same idea for a voiced consonant: the gain that brings voicing through these formants (and
 * nasal notch) to the vowel loudness level at the note's pitch. Using the vowel's own gain here
 * made voiced consonants up to 5 dB louder than the vowel at some pitches, where a harmonic lands
 * on the consonant's resonance (heard as a pop at the start of /dʒ/, /b/, /d/, /z/, /l/, /m/ ...).
 */
export type FormantGain = (t: Partial<Targets>) => number;

/** Per-class multipliers on consonant noise: stop bursts, fricatives (and /ch/), aspiration (/h/ and after p t k). */
export interface NoiseMix {
  burst: number;
  fric: number;
  asp: number;
}
export const FLAT_NOISE: NoiseMix = { burst: 1, fric: 1, asp: 1 };

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
  p: VoiceProfile,
  fg: FormantGain,
  mix: NoiseMix,
  /** False for a stop followed by another stop in the same coda ("-pt"): it is held but not released. */
  release = true,
): Segment[] {
  // Word-final noise (the /s/ of "jumps", the /t/ of "kept") is the last thing heard before the
  // voice stops, with nothing after it to mask it: it reads as a hiss/scratch at the end of the
  // syllable, so it is kept well below onset noise. Affricates (/dʒ/, /tʃ/) are quieter still.
  if (position === 'coda') mix = { burst: mix.burst * CODA_NOISE, fric: mix.fric * CODA_NOISE, asp: mix.asp * CODA_NOISE };
  const size = p.sizeScale;
  /** Voiced consonant amplitudes are fractions of the vowel level, for these formants at this pitch. */
  const V = (x: number, t: Partial<Targets>): number => x * fg(t);
  /** Voicing during obstruents (voiced stops, fricatives, affricates): quieter than sonorants. */
  const VO = (x: number, t: Partial<Targets>): number => V(x, t) * OBSTRUENT_VOICING;
  const stop = STOPS[sym];
  if (stop) {
    const loc = locus(stop.locus, p);
    if (!release) {
      return [{ dur: 45 * scale, trans: 10, t: { ...loc, av: stop.voiced ? VO(0.1, loc) : 0, ah: 0, af: 0, nz: 0 } }];
    }
    const burst: Partial<Targets> = {
      ...loc,
      af: stop.burstAmp * NOISE_TRIM * mix.burst,
      ff: Math.min(MAX_NOISE_HZ, stop.burstF * size),
      bf: stop.burstBw,
    };
    if (stop.voiced) {
      return [
        { dur: 40 * scale, trans: 10, t: { ...loc, av: VO(0.15, loc), ah: 0, af: 0, nz: 0 } },
        { dur: Math.max(MIN_BURST_MS, 6 * scale), trans: 3, t: { ...burst, av: VO(0.35, loc) } },
        { dur: 6 * scale, trans: 8, t: { ...burst, af: burst.af! * 0.25, av: VO(0.3, loc) } },
      ];
    }
    const segs: Segment[] = [
      { dur: 50 * scale, trans: 10, t: { ...loc, av: 0, ah: 0, af: 0, nz: 0 } },
      { dur: Math.max(MIN_BURST_MS, 7 * scale), trans: 3, t: burst },
      { dur: 7 * scale, trans: 8, t: { ...burst, af: burst.af! * 0.25 } },
    ];
    // Aspiration after a voiceless onset stop, through the vowel's formants: this is also
    // where the audible consonant-to-vowel formant transition happens.
    if (position === 'onset') {
      segs.push({ dur: 30 * scale, trans: 25, t: { ...nextVowel, av: 0, ah: 1.15 * NOISE_TRIM * mix.asp, af: 0 } });
    } else {
      segs.push({ dur: 14 * scale, trans: 8, t: { ...loc, af: 0, ah: 0.52 * NOISE_TRIM * mix.asp } });
    }
    return segs;
  }

  const fric = FRICATIVES[sym];
  if (fric) {
    const floc = locus(fric.locus, p);
    return [
      {
        dur: (fric.voiced ? 75 : 95) * scale,
        trans: 22,
        t: {
          ...floc,
          av: fric.voiced ? VO(0.2, floc) : 0,
          ah: 0,
          af: fric.af * NOISE_TRIM * mix.fric,
          ff: Math.min(MAX_NOISE_HZ, fric.ff * size),
          bf: fric.bf,
          nz: 0,
        },
      },
    ];
  }

  if (sym === 'CH' || sym === 'JH') {
    const voiced = sym === 'JH';
    const loc = locus('alveolar', p);
    return [
      { dur: 35 * scale, trans: 8, t: { ...loc, av: voiced ? VO(0.15, loc) : 0, ah: 0, af: 0, nz: 0 } },
      {
        dur: 60 * scale,
        trans: 9,
        t: { ...loc, av: voiced ? VO(0.35, loc) : 0, af: NOISE_LEVELS.affricateNoise * NOISE_TRIM * mix.fric * AFFRICATE_NOISE, ff: Math.min(MAX_NOISE_HZ, 2900 * size), bf: 2400 },
      },
    ];
  }

  if (sym === 'HH') {
    return [{ dur: 60 * scale, trans: 10, t: { ...nextVowel, av: 0, ah: NOISE_LEVELS.hhAspiration * NOISE_TRIM * mix.asp, af: 0, nz: 0 } }];
  }

  const son = SONORANTS[sym];
  if (son) {
    const formants: Partial<Targets> = {
      f1: son.f[0],
      f2: son.f[1] * size,
      f3: son.f[2] * size,
      b1: son.b1 ?? 60,
      b2: 90,
      b3: 150,
      nz: son.nasalZero ? 1 : 0,
      ...(son.nasalZero ? { fz: son.nasalZero * size } : {}),
    };
    return [
      {
        dur: (position === 'onset' ? 55 : 75) * scale,
        trans: 20,
        t: { ...formants, av: V(son.av, formants), ah: 0, af: 0 },
      },
    ];
  }
  return []; // unknown symbol: skip
}

const MIN_VOWEL_HOLD_MS = 60;
const FRESH_VOWEL_TRANS_MS = 40;
const LEGATO_VOWEL_TRANS_MS = 80;

function nucleusSegments(
  nucleus: string,
  transIn: number,
  holdFirst: boolean,
  p: VoiceProfile,
  gainFor: VowelGain,
): Segment[] {
  const key = vowelKey(nucleus);
  const vowelOn = (k: string) => ({ av: gainFor(k), ah: 0, af: 0, nz: 0 });
  const diph = DIPHTHONGS[key];
  if (diph && !holdFirst) {
    return [
      { dur: 100, trans: transIn, t: { ...vowelFormants(diph[0], p), ...vowelOn(diph[0]) } },
      {
        dur: 0,
        trans: 140,
        t: { ...vowelFormants(diph[1], p), av: gainFor(diph[1]) },
        hold: true,
        minHold: 50,
      },
    ];
  }
  // A held diphthong (melisma) sits on its first element; the glide happens on the last note.
  const target = diph ? diph[0] : key;
  return [
    {
      dur: 0,
      trans: transIn,
      t: { ...vowelFormants(target, p), ...vowelOn(target) },
      hold: true,
      minHold: MIN_VOWEL_HOLD_MS,
    },
  ];
}

/** Final fade to silence after the last phoneme. */
export const TAIL_SEGMENT: Segment = { dur: 90, trans: 70, t: { av: 0, ah: 0, af: 0, nz: 0 } };

/**
 * Build the segment lists for one syllable. `scale` shortens or lengthens consonants (1 =
 * natural-ish; lower puts the vowel closer to the key press).
 *
 * `legato` only changes how the vowel is entered: the onset consonants are always built, so
 * they are never lost when notes overlap. (The synth plays the previous syllable's coda and
 * this syllable's onset between the two vowels.) A legato syllable with *no* onset (a melisma
 * continuation, or a vowel-initial word) glides slowly from the previous vowel instead.
 */
export function buildSyllable(
  plan: SyllablePlan,
  scale: number,
  legato: boolean,
  profile: VoiceProfile = CLASSIC,
  gainFor: VowelGain = () => 1,
  mix: NoiseMix = FLAT_NOISE,
  formantGain: FormantGain = () => 1,
): SyllableSegments {
  const key = vowelKey(plan.nucleus);
  const nextVowel = vowelFormants(key, profile);
  const onset = plan.onset.flatMap((s) => consonant(s, 'onset', nextVowel, scale, profile, formantGain, mix));
  const coda = plan.coda.flatMap((s, i) =>
    consonant(s, 'coda', nextVowel, scale, profile, formantGain, mix, !(STOPS[s] && STOPS[plan.coda[i + 1]])),
  );
  const glide = legato && onset.length === 0;
  const nucleus = nucleusSegments(
    plan.nucleus,
    glide ? LEGATO_VOWEL_TRANS_MS : FRESH_VOWEL_TRANS_MS,
    plan.holdFirst === true,
    profile,
    gainFor,
  );
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
