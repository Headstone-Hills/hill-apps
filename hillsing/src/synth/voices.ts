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

// Voices from the 1960s to the 2010s that are not DECtalk descendants: research systems, hobbyist boards, chip and PC
// voices of the 1990s, and the commercial text-to-speech of the 2000s and 2010s (up to the neural voices). Same approach
// as the voices above: a parameter set patterned on the *character* of each (era, pitch range, how smooth or buzzy it
// sounds, how steady), not a reconstruction. The later ones were mostly built from recordings of a real speaker, which a
// formant engine cannot play, so these capture their general timbre (clean, smooth, warm) and nothing more. Model names
// and years are written from memory and are approximate.
const ERAS: VoiceProfile[] = [
  // 1960s - 1970s: research and hobbyist formant synthesis
  variant(CLASSIC_BASE, 'holmes', 'Holmes', 'J. N. Holmes parallel-formant synthesizer (1964)',
    'Early British research synthesizer: smooth, steady and slightly thin, with gentle consonants.',
    { centerNote: 48, bandwidthScale: 1.05, tiltBase: 0.35, tiltVelocity: 0.15, openPhase: 0.42, closeMs: 0.5, drift: 0.1, consonantLevel: 0.85 }, 0.97),
  variant(CLASSIC_BASE, 'umeda', 'Umeda', 'Umeda ETL English text-to-speech (1968)',
    'The first complete English text-to-speech: flat, mechanical and cautious.',
    { centerNote: 47, bandwidthScale: 0.95, tiltBase: 0.3, tiltVelocity: 0.1, closeMs: 0.4, drift: 0, consonantLevel: 0.7 }, 0.99),
  variant(CLASSIC_BASE, 'computalker', 'Computalker', 'Computalker CT-1 S-100 board (1976)',
    'Hobbyist computer speech: buzzy but smoother than the toy chips, with a steady, mid-bright tone.',
    { centerNote: 46, bandwidthScale: 0.75, tiltBase: 0.55, openPhase: 0.33, closeMs: 0.25, drift: 0.3, consonantLevel: 1.1 }, 0.97),
  // 1980s
  variant(CLASSIC_BASE, 'ti99', 'TI-99', 'Texas Instruments TI-99/4A Speech Synthesizer (1981)',
    'Thin, nasal home-computer speech: narrow formants and a bright, steady buzz.',
    { centerNote: 50, bandwidthScale: 0.7, tiltBase: 0.6, openPhase: 0.32, closeMs: 0.15, drift: 0, consonantLevel: 1.15 }, 0.98),
  // 1990s: PC speech
  variant(CLASSIC_BASE, 'sbaitso', 'Dr. Sbaitso', 'Creative Labs Dr. Sbaitso (1991)',
    'Sound Blaster era: flat, slightly lo-fi male with hard edges.',
    { centerNote: 48, bandwidthScale: 0.8, tiltBase: 0.5, closeMs: 0.2, drift: 0.1, consonantLevel: 1.2 }, 0.98),
  variant(CLASSIC_BASE, 'festival', 'Festival', 'Festival diphone voice (1996)',
    'Open-source research voice: smooth, even and a little nasal, with steady pitch.',
    { centerNote: 52, bandwidthScale: 1.16, tiltBase: 0.35, closeMs: 0.3, breath: 0.01, drift: 0.5 }, 1.07),
  variant(CLASSIC_BASE, 'mssam', 'MS Sam', 'Microsoft Sam (Windows 2000, 1999)',
    'The famous default Windows voice: buzzy, nasal and flat.',
    { centerNote: 47, bandwidthScale: 0.8, tiltBase: 0.5, openPhase: 0.38, closeMs: 0.2, drift: 0.15, consonantLevel: 1.2 }, 0.97),
  variant(FEMALE_BASE, 'msmary', 'MS Mary', 'Microsoft Mary (Windows, 1998)',
    'Synthetic Windows female: smooth but buzzy, with a prim, even delivery.',
    { centerNote: 59, bandwidthScale: 0.85, tiltBase: 0.5, closeMs: 0.25, drift: 0.3, consonantLevel: 1.1 }, 1),
  variant(CLASSIC_BASE, 'msmike', 'MS Mike', 'Microsoft Mike (Windows, 1998)',
    'Plain, clear Windows male: a bit nasal, mildly buzzy, a little higher than Sam.',
    { centerNote: 50, bandwidthScale: 1, tiltBase: 0.5, closeMs: 0.28, drift: 0.3, consonantLevel: 1.1 }, 1.03),
  variant(CHILD_BASE, 'peedy', 'Peedy', 'Microsoft Agent Peedy (1998)',
    'The on-screen parrot assistant: high, squawky and bright.',
    { centerNote: 58, bandwidthScale: 0.8, tiltBase: 0.55, closeMs: 0.2, drift: 0.6, consonantLevel: 1.15 }, 0.9),
  // 2000s: commercial concatenative voices, and eSpeak
  variant(CLASSIC_BASE, 'espeak', 'eSpeak', 'eSpeak formant synthesizer (2005)',
    'Compact open-source formant speech: clear, robotic and very steady.',
    { centerNote: 48, bandwidthScale: 0.7, tiltBase: 0.55, openPhase: 0.35, closeMs: 0.2, drift: 0, consonantLevel: 1.15 }, 0.99),
  variant(CLASSIC_BASE, 'attmike', 'Natural Mike', 'AT&T Natural Voices "Mike" (2001)',
    'Early "natural" male: smooth and clean with little buzz, easy on the ear.',
    { centerNote: 47, bandwidthScale: 1.12, tiltBase: 0.38, tiltVelocity: 0.45, breath: 0.02, drift: 0.9, closeMs: 0.35, consonantLevel: 0.95 }, 1),
  variant(FEMALE_BASE, 'crystal', 'Crystal', 'AT&T Natural Voices "Crystal" (2001)',
    'Smooth, even "natural" female: round and slightly darker, with a soft pulse and a light breath.',
    { centerNote: 58, bandwidthScale: 1.2, tiltBase: 0.36, openPhase: 0.52, breath: 0.08, drift: 0.6, closeMs: 0.5, sizeScale: 1.1, consonantLevel: 0.85 }, 0.99),
  variant(CLASSIC_BASE, 'cepstral', 'Cepstral David', 'Cepstral David (2003)',
    'Warm, rounded male: wide formants and a gentle, steady tone.',
    { centerNote: 47, bandwidthScale: 1.1, tiltBase: 0.36, breath: 0.03, drift: 1.1, closeMs: 0.5, consonantLevel: 0.95 }, 1),
  variant(FEMALE_BASE, 'anna', 'Anna', 'Microsoft Anna (Windows Vista, 2006)',
    'Clean, bright Vista-era female: even, thin and a little crisp.',
    { centerNote: 62, bandwidthScale: 1, tiltBase: 0.44, breath: 0.04, drift: 0.5, closeMs: 0.32 }, 1.02),
  variant(CLASSIC_BASE, 'alex', 'Alex', 'Apple Alex (Mac OS X, 2007)',
    'Natural-sounding male with a warm, breathy tone and a relaxed drift.',
    { centerNote: 49, bandwidthScale: 1.25, tiltBase: 0.27, openPhase: 0.5, breath: 0.12, drift: 1.2, closeMs: 0.55 }, 1.03),
  variant(FEMALE_BASE, 'kate', 'Kate', 'NeoSpeech VoiceText Kate (2008)',
    'Very bright, crisp female: sharp consonants over a light, clear voice.',
    { centerNote: 64, bandwidthScale: 0.95, tiltBase: 0.55, breath: 0.03, closeMs: 0.22, consonantLevel: 1.2 }, 1.03),
  // 2010s: the last voices before neural speech
  variant(FEMALE_BASE, 'samantha', 'Samantha', 'Apple Samantha / Siri (2011)',
    'Clear, friendly assistant female: smooth with an even, polished tone.',
    { centerNote: 60, bandwidthScale: 1.05, tiltBase: 0.4, breath: 0.05, drift: 0.9, closeMs: 0.35 }, 1.01),
  variant(CLASSIC_BASE, 'msdavid', 'MS David', 'Microsoft David (Windows 8, 2012)',
    'Clean, neutral male: smooth, unfussy and slightly bright.',
    { centerNote: 49, bandwidthScale: 1.1, tiltBase: 0.44, breath: 0.02, drift: 0.7, closeMs: 0.3, upperFormantScale: 1.03 }, 1.02),
  variant(FEMALE_BASE, 'zira', 'MS Zira', 'Microsoft Zira (Windows 8, 2012)',
    'Warm, composed lower female: a gentle pulse and a soft airiness.',
    { centerNote: 57, bandwidthScale: 1.15, tiltBase: 0.32, breath: 0.09, drift: 1, closeMs: 0.5, sizeScale: 1.12 }, 0.98),
  variant(CLASSIC_BASE, 'brian', 'Brian', 'IVONA Brian (2012)',
    'Rich, deep British-style male: dark, round and slightly breathy.',
    { centerNote: 43, bandwidthScale: 1.25, tiltBase: 0.28, breath: 0.07, drift: 1, closeMs: 0.6, sizeScale: 0.9, upperFormantScale: 0.92 }, 0.9),
  variant(FEMALE_BASE, 'pico', 'Pico', 'SVOX Pico (Android, 2011)',
    'Compact phone voice: small, light and a little bright.',
    { centerNote: 62, bandwidthScale: 1, tiltBase: 0.5, closeMs: 0.25, drift: 0.6, consonantLevel: 1.1, sizeScale: 1.2 }, 1.03),
  variant(FEMALE_BASE, 'gtts', 'Translate', 'Google Translate voice (2010s)',
    'Flat, slightly dull web-page reader: even and steady, with very little drift.',
    { centerNote: 59, bandwidthScale: 1.05, tiltBase: 0.38, drift: 0.1, closeMs: 0.35, consonantLevel: 1.05, sizeScale: 1.15 }, 0.99),
];

// Character voices from later synthesis scenes. Same approach as above: a parameter set patterned on
// the *character* of the voice, not its data. A UTAU voicebank is a set of recorded samples, which
// this formant engine cannot play, so these never contain or imitate any specific recording.
const CHARACTER: VoiceProfile[] = [
  variant(FEMALE_BASE, 'teto', 'Teto', 'UTAU Kasane Teto (character-style)',
    'Bright, youthful and light, in the spirit of the UTAU character. A formant approximation, not her voicebank. Sing around E4.',
    { centerNote: 64, bandwidthScale: 1.0, upperFormantScale: 1.12, sizeScale: 1.2, openPhase: 0.5, closeMs: 0.3, tiltBase: 0.38, tiltVelocity: 0.4, breath: 0.06, drift: 1.1 }, 1.04),
];

export const VOICES: ReadonlyArray<VoiceProfile> = [...BASES, ...PERIOD, ...ERAS, ...CHARACTER];

/** The year in a voice's model name ("... (1984)"), or null for the character voices, which have none. */
export function voiceYear(v: VoiceProfile): number | null {
  const m = /\((?:[^)]*?,\s*)?(\d{4})\)/.exec(v.model);
  if (m) return Number(m[1]);
  const spelled = /\b(\d{4})s\b/.exec(v.model); // "(2010s)"
  if (spelled) return Number(spelled[1]);
  const short = /\b(\d)0s\b/.exec(v.model); // "80s speech chip"
  return short ? 1900 + Number(short[1]) * 10 : null;
}

/** The voices grouped by decade, oldest first, each group in chronological order; undated voices come last. */
export function voicesByEra(): Array<{ label: string; voices: VoiceProfile[] }> {
  const groups = new Map<string, VoiceProfile[]>();
  for (const v of VOICES) {
    const y = voiceYear(v);
    const label = y === null ? 'Characters' : `${Math.floor(y / 10) * 10}s`;
    groups.set(label, [...(groups.get(label) ?? []), v]);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => (a === 'Characters' ? 1 : b === 'Characters' ? -1 : a.localeCompare(b)))
    .map(([label, voices]) => ({ label, voices: [...voices].sort((p, q) => (voiceYear(p) ?? 9999) - (voiceYear(q) ?? 9999)) }));
}

export const CLASSIC: VoiceProfile = VOICES[0];

export function getVoice(id: string): VoiceProfile {
  return VOICES.find((v) => v.id === id) ?? CLASSIC;
}
