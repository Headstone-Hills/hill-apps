/** Phonemes of one syllable, as ARPAbet symbols (vowel keeps its stress digit). */
export interface SyllablePlan {
  onset: string[];
  nucleus: string;
  coda: string[];
  /**
   * Melisma: this syllable continues on further notes, so a diphthong holds its first vowel
   * instead of gliding (the glide happens on the last note, as singers do).
   */
  holdFirst?: boolean;
}

/** Synthesizer control parameters (a small subset of Klatt 1980's). Frequencies in Hz. */
export interface Targets {
  f1: number;
  f2: number;
  f3: number;
  b1: number;
  b2: number;
  b3: number;
  /** Voicing amplitude, 0..1. */
  av: number;
  /** Aspiration (noise through the vocal tract) amplitude, 0..1. */
  ah: number;
  /** Frication (parallel noise branch) amplitude, 0..1. */
  af: number;
  /** Frication resonator centre frequency and bandwidth. */
  ff: number;
  bf: number;
  /** Nasal anti-resonance frequency and its mix amount (0 = off). */
  fz: number;
  nz: number;
}

export const TARGET_KEYS: ReadonlyArray<keyof Targets> = [
  'f1', 'f2', 'f3', 'b1', 'b2', 'b3', 'av', 'ah', 'af', 'ff', 'bf', 'fz', 'nz',
];

export const DEFAULT_TARGETS: Targets = {
  f1: 500, f2: 1500, f3: 2500, b1: 60, b2: 90, b3: 150,
  av: 0, ah: 0, af: 0, ff: 3000, bf: 1000, fz: 1000, nz: 0,
};

/**
 * A timed movement of the control parameters. Times in milliseconds. Parameters not named
 * in `t` keep their current value, so segments only describe what changes.
 */
export interface Segment {
  /** Time to stay in this segment (ignored when `hold`). */
  dur: number;
  /** Time over which parameters move linearly to the targets. */
  trans: number;
  t: Partial<Targets>;
  /** Stay here until note-off (the sustained vowel). */
  hold?: boolean;
  /** Minimum time to stay in a hold segment after note-off is requested. */
  minHold?: number;
}

/**
 * A voice command scheduled at a time in seconds (engine time). The sequencer applies it at
 * the exact sample, inside the AudioWorklet, so playback timing does not depend on the main
 * thread.
 */
export type SeqCommand =
  | { t: number; kind: 'on'; plan: SyllablePlan; f0: number; velocity: number; legato: boolean }
  | { t: number; kind: 'off' };
