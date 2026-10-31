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
  /**
   * The classic formant-TTS voice this preset is patterned on (DECtalk descends from the same
   * Klatt 1980 synthesizer this engine implements). These are approximations by character, not
   * DECtalk's actual parameter sets.
   */
  model: string;
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
    model: 'DECtalk Perfect Paul (1984)',
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
    model: 'DECtalk Huge Harry (1984)',
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
    model: 'DECtalk Beautiful Betty (1984)',
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
    model: 'DECtalk Kit the Kid (1984)',
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
    model: 'Generic buzzy 80s speech chip',
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
    model: 'DECtalk Whispering Wendy (1984)',
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

/** A preset derived from one of the bases above, overriding only what makes it different. */
function variant(
  base: VoiceProfile,
  id: string,
  name: string,
  model: string,
  description: string,
  over: Partial<VoiceProfile> = {},
  vowelScale = 1,
): VoiceProfile {
  return {
    ...base,
    id,
    name,
    model,
    description,
    ...over,
    vowels: over.vowels ?? (vowelScale === 1 ? base.vowels : scaled(base.vowels, vowelScale)),
  };
}

const [CLASSIC_BASE, , FEMALE_BASE, CHILD_BASE, ROBOT_BASE] = BASES;

// Period voices, 1961-1990s. Each is a parameter set patterned on the *character* of the original
// (pitch range, bandwidth, buzziness, steadiness), not a reconstruction of its actual data.
const PERIOD: VoiceProfile[] = [
  variant(ROBOT_BASE, 'daisy', 'Daisy', 'IBM 704 "Daisy Bell" (1961)',
    'The first singing computer: dark, steady and slightly muffled, with soft consonants.',
    { centerNote: 50, tiltBase: 0.3, tiltVelocity: 0.15, bandwidthScale: 0.9, openPhase: 0.4, closeMs: 0.5, consonantLevel: 0.8 }, 0.96),
  variant(CLASSIC_BASE, 'kurzweil', 'Reader', 'Kurzweil Reading Machine (1976)',
    'Flat, even, dutiful male reader. Little drift, moderate brightness.',
    { bandwidthScale: 0.9, tiltBase: 0.45, drift: 0.2, closeMs: 0.25 }, 1.02),
  variant(CHILD_BASE, 'speakspell', 'Speak & Spell', 'Texas Instruments Speak & Spell (1978)',
    'Bright, thin and buzzy, like the toy: narrow formants, razor-edged pulse, steady pitch.',
    { bandwidthScale: 0.7, tiltBase: 0.65, tiltVelocity: 0.2, openPhase: 0.3, closeMs: 0.15, breath: 0, drift: 0, consonantLevel: 1.2 }, 0.97),
  variant(CLASSIC_BASE, 'mitalk', 'MITalk', 'MITalk (1979)',
    'Early research formant TTS: smooth, rule-driven, a little stiff.',
    { bandwidthScale: 0.95, drift: 0.4, closeMs: 0.35, tiltBase: 0.4 }, 1.01),
  variant(ROBOT_BASE, 'votrax', 'Votrax', 'Votrax SC-01 (1980)',
    'Arcade-cabinet speech chip: very narrow, ringing and metallic, hard consonants.',
    { bandwidthScale: 0.55, tiltBase: 0.75, openPhase: 0.28, closeMs: 0.12, consonantLevel: 1.4 }, 1.04),
  variant(ROBOT_BASE, 'intellivoice', 'Intellivoice', 'Mattel Intellivoice (1982)',
    'Gritty console voice: buzzy, mid-bright, hard-edged.',
    { bandwidthScale: 0.65, tiltBase: 0.6, openPhase: 0.33, closeMs: 0.18, consonantLevel: 1.3 }, 1),
  variant(ROBOT_BASE, 'sam', 'SAM', 'S.A.M. Software Automatic Mouth (1982)',
    'Commodore 64 speech: nasal, buzzy and a bit mushy, with exaggerated consonants.',
    { centerNote: 52, bandwidthScale: 0.7, tiltBase: 0.55, openPhase: 0.35, closeMs: 0.2, consonantLevel: 1.35, drift: 0 }, 0.95),
  variant(CLASSIC_BASE, 'macintalk', 'MacinTalk', 'Apple MacinTalk "Fred" (1984)',
    'The original Macintosh voice: raspy, low and a touch gravelly.',
    { centerNote: 45, bandwidthScale: 0.8, tiltBase: 0.5, closeMs: 0.15, breath: 0.03, drift: 0.3, consonantLevel: 1.15 }, 0.93),
  variant(CLASSIC_BASE, 'dennis', 'Dennis', 'DECtalk Doctor Dennis (1984)',
    'Clear, slightly brighter male, a more animated sibling of Paul.',
    { tiltBase: 0.4, bandwidthScale: 0.95, closeMs: 0.28 }, 0.98),
  variant(FEMALE_BASE, 'ursula', 'Ursula', 'DECtalk Uppity Ursula (1984)',
    'Bright, prim adult female with a sharp edge.',
    { bandwidthScale: 0.95, tiltBase: 0.4, closeMs: 0.35, breath: 0.04 }, 1.02),
  variant(FEMALE_BASE, 'rita', 'Rita', 'DECtalk Rough Rita (1984)',
    'Gravelly adult female: sharper glottal pulse, tight bandwidths.',
    { bandwidthScale: 0.85, tiltBase: 0.45, closeMs: 0.15, breath: 0.02, drift: 1.2 }, 0.98),
  variant(CLASSIC_BASE, 'frank', 'Frank', 'DECtalk Frail Frank (1984)',
    'Thin, elderly male: soft, wavering, and breathy.',
    { centerNote: 50, bandwidthScale: 1.2, breath: 0.15, drift: 1.8, closeMs: 0.6, tiltBase: 0.3 }, 1.03),
  variant(ROBOT_BASE, 'amiga', 'Amiga', 'Amiga Narrator (1985)',
    'The Workbench voice: robotic male, nasal, with a dry, even buzz.',
    { centerNote: 48, bandwidthScale: 0.7, tiltBase: 0.55, openPhase: 0.35, closeMs: 0.2, consonantLevel: 1.2, drift: 0.1 }, 0.97),
];

export const VOICES: ReadonlyArray<VoiceProfile> = [...BASES, ...PERIOD];

export const CLASSIC: VoiceProfile = VOICES[0];

export function getVoice(id: string): VoiceProfile {
  return VOICES.find((v) => v.id === id) ?? CLASSIC;
}
