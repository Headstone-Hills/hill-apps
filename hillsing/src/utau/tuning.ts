// "Tuning" for the sample-based voice, like a UTAU editor's parameters: how fast consonants go by, how loud and
// bright they are, the voice's gender (formant) colour, vibrato, how a held key glides to the next pitch, a fine
// tune and the volume. One shared set of values that live playing and rendered melodies both follow. No Web Audio
// here, so it is testable in Node.

import { mergeNumbers, type ParamSpec } from '../audio/fxparams';

export interface Tuning {
  velocity: number;
  consonant: number;
  tone: number;
  gender: number;
  vibratoDepth: number;
  vibratoRate: number;
  glide: number;
  tune: number;
  volume: number;
}

type TuneSpec = ParamSpec & { id: keyof Tuning };
const q = (id: keyof Tuning, label: string, min: number, max: number, step: number, value: number, unit = '', log = false): TuneSpec => ({
  id, label, min, max, step, value, unit, log,
});

export const TUNING_PARAMS: ReadonlyArray<ParamSpec & { id: keyof Tuning; hint: string; live?: boolean }> = [
  { ...q('velocity', 'Consonant speed', 0.5, 2.5, 0.05, 1, '×'), hint: 'How fast consonants go by (UTAU velocity). Higher brings the vowel in sooner. Live keys already play consonants 1.5× faster; this scales that. In melodies, below 1× changes nothing.' },
  { ...q('consonant', 'Consonant level', 0, 2, 0.05, 1), hint: 'How loud the consonants are against the vowel.' },
  { ...q('tone', 'Consonant tone', 2000, 16000, 1, 16000, ' Hz', true), hint: 'Rounds off harsh bursts and hiss in consonants. All the way up is off. Lower it if K, T or S clicks or sizzles.' },
  { ...q('gender', 'Gender', -1, 1, 0.01, 0), hint: 'Moves the voice colour (formants) without changing pitch: up is brighter and younger, down is darker and bigger.' },
  { ...q('vibratoDepth', 'Vibrato depth', 0, 100, 1, 0, ' cents'), hint: 'How far held notes wave in pitch. It fades in after the start of the note.' },
  { ...q('vibratoRate', 'Vibrato rate', 3, 9, 0.1, 5.5, ' Hz'), hint: 'How fast the vibrato waves.' },
  { ...q('glide', 'Glide', 0, 400, 1, 30, ' ms'), live: true, hint: 'How long a live key takes to slide into the next pitch when you play legato (portamento).' },
  { ...q('tune', 'Fine tune', -100, 100, 1, 0, ' cents'), hint: 'Shifts the whole voice sharp or flat.' },
  { ...q('volume', 'Volume', 0, 1.5, 0.01, 1), hint: 'Loudness of the sample voice.' },
];

export function defaultTuning(): Tuning {
  const t = {} as Tuning;
  for (const p of TUNING_PARAMS) t[p.id] = p.value;
  return t;
}

/** Spectrum ratio for the sampler from the Gender value (-1..1): about ±19 % in formant position at the extremes. */
export const genderToFormant = (g: number): number => Math.pow(2, g * 0.25);

/** Pitch multiplier for vibrato `seconds` after a note starts (0 delay before 0.15 s, full depth after 0.45 s). */
export function vibratoFactor(depthCents: number, rateHz: number, seconds: number): number {
  if (depthCents <= 0 || seconds <= 0.15) return 1;
  const env = Math.min(1, (seconds - 0.15) / 0.3);
  return Math.pow(2, (depthCents * env * Math.sin(2 * Math.PI * rateHz * seconds)) / 1200);
}

export const centsToRatio = (c: number): number => Math.pow(2, c / 1200);

export function formatTuning(id: keyof Tuning, v: number): string {
  switch (id) {
    case 'velocity': return `${v.toFixed(2)}×`;
    case 'consonant':
    case 'volume': return `${Math.round(v * 100)}%`;
    case 'tone': return v >= 16000 ? 'off' : `${(v / 1000).toFixed(1)} kHz`;
    case 'gender': return `${v > 0 ? '+' : ''}${Math.round(v * 100)}`;
    case 'vibratoRate': return `${v.toFixed(1)} Hz`;
    case 'glide': return `${Math.round(v)} ms`;
    default: return `${Math.round(v)} cents`;
  }
}

type Store = Pick<Storage, 'getItem' | 'setItem'>;
const KEY = 'hillsing.utau.tuning.v1';

export class TuningModel {
  values: Tuning = defaultTuning();
  private readonly listeners: Array<(id: keyof Tuning | null) => void> = [];

  constructor(private readonly store: Store | null = null) {
    try {
      const raw = store?.getItem(KEY);
      if (raw) this.values = mergeNumbers(defaultTuning(), JSON.parse(raw), TUNING_PARAMS);
    } catch {
      /* unreadable saved settings: use the defaults */
    }
  }

  set(id: keyof Tuning, value: number): void {
    const spec = TUNING_PARAMS.find((p) => p.id === id);
    if (!spec) return;
    this.values[id] = Math.min(spec.max, Math.max(spec.min, value));
    this.save();
    for (const l of this.listeners) l(id);
  }

  reset(): void {
    this.values = defaultTuning();
    this.save();
    for (const l of this.listeners) l(null);
  }

  onChange(listener: (id: keyof Tuning | null) => void): void {
    this.listeners.push(listener);
  }

  private save(): void {
    try {
      this.store?.setItem(KEY, JSON.stringify(this.values));
    } catch {
      /* storage unavailable: fine */
    }
  }
}
