// Voice profiles: data-only presets for the formant synthesizer.
//
// These are *parameter sets for one synthesis engine*, not different TTS engines. They change
// the vowel formant table (vocal-tract size / sex / age), the glottal pulse shape, formant
// bandwidths, breathiness and natural drift. Expect a family resemblance between them.
//
// Formant values: "classic" are approximate adult-male averages; "female" and "child" follow the
// pattern of the commonly tabulated Peterson & Barney (1952) averages for women and children. They
// are written from memory of those tables and were NOT checked against the paper; treat them as
// plausible starting points, not citations of specific numbers.

export type Formants = [number, number, number];
export type VowelTable = Record<string, Formants>;

export interface VoiceProfile {
  id: string;
  name: string;
  description: string;
  /** A comfortable MIDI note for this voice (centre of its range). Used for auditions. */
  centerNote: number;
  vowels: VowelTable;
  /** Scales consonant place frequencies (loci, burst and frication centres, liquids, nasals). */
  sizeScale: number;
  /** Scales formant bandwidths: lower = narrower, more ringing / buzzy; higher = softer. */
  bandwidthScale: number;
  /** Scales the fixed upper formants F4/F5. */
  upperFormantScale: number;
  /**
   * Glottal pulse: fraction of the cycle spent opening, and the *absolute* time (ms) the folds
   * take to close. A shorter closing is sharper and brighter; real closure is ~0.1-0.5 ms.
   */
  openPhase: number;
  closeMs: number;
  /** Brightness filter coefficient: base + velocity * velocityAmount (higher = brighter). */
  tiltBase: number;
  tiltVelocity: number;
  /** Constant breath noise mixed into voiced sound (0..1). */
  breath: number;
  /** Natural slow pitch drift amount (1 = default, 0 = perfectly steady). */
  drift: number;
  /** Multiplier on consonant noise (frication, bursts, aspiration). */
  consonantLevel: number;
}

const CLASSIC_VOWELS: VowelTable = {
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

const FEMALE_VOWELS: VowelTable = {
  IY: [310, 2790, 3310],
  IH: [430, 2480, 3070],
  EH: [610, 2330, 2990],
  AE: [860, 2050, 2850],
  AA: [850, 1220, 2810],
  AO: [590, 920, 2710],
  UH: [470, 1160, 2680],
  UW: [370, 950, 2670],
  AH: [760, 1400, 2780],
  AX: [600, 1650, 2800],
  ER: [500, 1640, 1960],
};

const CHILD_VOWELS: VowelTable = {
  IY: [370, 3200, 3730],
  IH: [530, 2730, 3600],
  EH: [690, 2610, 3570],
  AE: [1010, 2320, 3320],
  AA: [1030, 1370, 3170],
  AO: [680, 1060, 3180],
  UH: [560, 1410, 3310],
  UW: [430, 1170, 3260],
  AH: [850, 1590, 3360],
  AX: [650, 1900, 3400],
  ER: [560, 1820, 2160],
};

function scaled(table: VowelTable, f: number): VowelTable {
  const out: VowelTable = {};
  for (const [k, v] of Object.entries(table)) out[k] = [v[0] * f, v[1] * f, v[2] * f];
  return out;
}

const BASES: VoiceProfile[] = [
  {
    id: 'classic',
    name: 'Classic',
    description: 'Neutral adult male. The reference voice everything else is tuned against.',
    centerNote: 48,
    vowels: CLASSIC_VOWELS,
    sizeScale: 1,
    bandwidthScale: 1,
    upperFormantScale: 1,
    openPhase: 0.45,
    closeMs: 0.3,
    tiltBase: 0.35,
    tiltVelocity: 0.4,
    breath: 0,
    drift: 1,
    consonantLevel: 1,
  },
  {
    id: 'deep',
    name: 'Deep',
    description: 'Larger vocal tract: darker, bass-like. Sing low (around F2).',
    centerNote: 41,
    vowels: scaled(CLASSIC_VOWELS, 0.88),
    sizeScale: 0.88,
    bandwidthScale: 0.95,
    upperFormantScale: 0.9,
    openPhase: 0.47,
    closeMs: 0.4,
    tiltBase: 0.3,
    tiltVelocity: 0.3,
    breath: 0.02,
    drift: 1,
    consonantLevel: 1,
  },
  {
    id: 'female',
    name: 'Female',
    description: 'Adult female formants, softer glottal pulse. Sing around C4.',
    centerNote: 60,
    vowels: FEMALE_VOWELS,
    sizeScale: 1.17,
    bandwidthScale: 1.1,
    upperFormantScale: 1.1,
    openPhase: 0.5,
    closeMs: 0.45,
    tiltBase: 0.3,
    tiltVelocity: 0.4,
    breath: 0.08,
    drift: 1,
    consonantLevel: 1,
  },
  {
    id: 'child',
    name: 'Child',
    description: 'Small vocal tract: bright and light. Sing around G4.',
    centerNote: 67,
    vowels: CHILD_VOWELS,
    sizeScale: 1.35,
    bandwidthScale: 1.15,
    upperFormantScale: 1.2,
    openPhase: 0.5,
    closeMs: 0.3,
    tiltBase: 0.4,
    tiltVelocity: 0.4,
    breath: 0.05,
    drift: 1.2,
    consonantLevel: 1,
  },
  {
    id: 'robot',
    name: 'Robot',
    description: 'Old-school buzzy synth: narrow ringing formants, razor-sharp pulse, no drift.',
    centerNote: 48,
    vowels: CLASSIC_VOWELS,
    sizeScale: 1,
    bandwidthScale: 0.6,
    upperFormantScale: 1,
    openPhase: 0.3,
    closeMs: 0.15,
    tiltBase: 0.7,
    tiltVelocity: 0.2,
    breath: 0,
    drift: 0,
    consonantLevel: 1.25,
  },
  {
    id: 'breathy',
    name: 'Breathy',
    description: 'Soft, airy tone with wide bandwidths. Good for ballads.',
    centerNote: 55,
    vowels: scaled(CLASSIC_VOWELS, 1.05),
    sizeScale: 1.05,
    bandwidthScale: 1.4,
    upperFormantScale: 1.05,
    openPhase: 0.55,
    closeMs: 0.9,
    tiltBase: 0.25,
    tiltVelocity: 0.3,
    breath: 0.35,
    drift: 1.3,
    consonantLevel: 1,
  },
];

export const VOICES: ReadonlyArray<VoiceProfile> = BASES;

export const CLASSIC: VoiceProfile = VOICES[0];

export function getVoice(id: string): VoiceProfile {
  return VOICES.find((v) => v.id === id) ?? CLASSIC;
}
