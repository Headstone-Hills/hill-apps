// The effects section: what each effect is called, its parameters and defaults, the settings model that the
// panel edits (with persistence), and the pure maths behind the effects (waveshaper curve, reverb impulse
// response, bit/rate reduction). No Web Audio here, so it is all testable in Node.

export type EffectId = 'bitcrush' | 'distortion' | 'eq' | 'compressor' | 'chorus' | 'delay' | 'reverb';

export interface ParamSpec {
  id: string;
  label: string;
  min: number;
  max: number;
  step: number;
  value: number;
  unit: string;
  /** Logarithmic slider (frequencies). */
  log?: boolean;
}

export interface EffectSpec {
  id: EffectId;
  name: string;
  blurb: string;
  params: ParamSpec[];
}

const p = (id: string, label: string, min: number, max: number, step: number, value: number, unit = '', log = false): ParamSpec => ({
  id, label, min, max, step, value, unit, log,
});

/** In signal-chain order: the sound passes through them top to bottom. */
export const EFFECTS: ReadonlyArray<EffectSpec> = [
  {
    id: 'bitcrush',
    name: 'Bitcrush',
    blurb: 'Fewer bits and a lower sample rate: gritty, lo-fi, old-console.',
    params: [
      p('bits', 'Bit depth', 1, 16, 1, 8, ' bits'),
      p('rate', 'Rate reduction', 1, 40, 1, 4, '×'),
      p('mix', 'Mix', 0, 1, 0.01, 1),
    ],
  },
  {
    id: 'distortion',
    name: 'Distortion',
    blurb: 'Soft-clipping overdrive that adds harmonics.',
    params: [
      p('drive', 'Drive', 0, 40, 0.5, 12, ' dB'),
      p('tone', 'Tone', 500, 12000, 1, 6000, ' Hz', true),
      p('level', 'Output level', -24, 0, 0.5, -6, ' dB'),
      p('mix', 'Mix', 0, 1, 0.01, 1),
    ],
  },
  {
    id: 'eq',
    name: 'EQ',
    blurb: 'Low shelf, one adjustable mid band, high shelf.',
    params: [
      p('lowGain', 'Low', -15, 15, 0.5, 0, ' dB'),
      p('lowFreq', 'Low corner', 40, 500, 1, 150, ' Hz', true),
      p('midGain', 'Mid', -15, 15, 0.5, 0, ' dB'),
      p('midFreq', 'Mid frequency', 150, 6000, 1, 1200, ' Hz', true),
      p('midQ', 'Mid width (Q)', 0.2, 8, 0.1, 1),
      p('highGain', 'High', -15, 15, 0.5, 0, ' dB'),
      p('highFreq', 'High corner', 1500, 14000, 1, 6000, ' Hz', true),
    ],
  },
  {
    id: 'compressor',
    name: 'Compressor',
    blurb: 'Evens out loud and quiet parts.',
    params: [
      p('threshold', 'Threshold', -60, 0, 0.5, -24, ' dB'),
      p('ratio', 'Ratio', 1, 20, 0.1, 4, ':1'),
      p('attack', 'Attack', 1, 200, 1, 10, ' ms'),
      p('release', 'Release', 20, 1000, 1, 200, ' ms'),
      p('knee', 'Knee', 0, 40, 1, 12, ' dB'),
      p('makeup', 'Make-up gain', 0, 24, 0.5, 0, ' dB'),
    ],
  },
  {
    id: 'chorus',
    name: 'Chorus',
    blurb: 'Two slightly detuned copies drifting left and right: a wider, thicker voice.',
    params: [
      p('rate', 'Rate', 0.05, 8, 0.05, 0.8, ' Hz', true),
      p('depth', 'Depth', 0, 15, 0.1, 4, ' ms'),
      p('delay', 'Delay', 5, 40, 0.5, 18, ' ms'),
      p('mix', 'Mix', 0, 1, 0.01, 0.5),
    ],
  },
  {
    id: 'delay',
    name: 'Delay',
    blurb: 'Echoes that fade, each a little darker.',
    params: [
      p('time', 'Time', 20, 1200, 1, 350, ' ms'),
      p('feedback', 'Feedback', 0, 0.9, 0.01, 0.35),
      p('tone', 'Echo tone', 800, 10000, 1, 4000, ' Hz', true),
      p('mix', 'Mix', 0, 1, 0.01, 0.3),
    ],
  },
  {
    id: 'reverb',
    name: 'Reverb',
    blurb: 'A room around the voice, from a booth to a cathedral.',
    params: [
      p('size', 'Size', 0.2, 6, 0.1, 2.2, ' s'),
      p('preDelay', 'Pre-delay', 0, 150, 1, 20, ' ms'),
      p('damping', 'Damping', 1000, 16000, 1, 6000, ' Hz', true),
      p('mix', 'Mix', 0, 1, 0.01, 0.25),
    ],
  },
];

export const effectSpec = (id: EffectId): EffectSpec => EFFECTS.find((e) => e.id === id)!;

export interface FxState {
  enabled: Record<EffectId, boolean>;
  values: Record<EffectId, Record<string, number>>;
}

export function defaultFxState(): FxState {
  const enabled = {} as Record<EffectId, boolean>;
  const values = {} as Record<EffectId, Record<string, number>>;
  for (const e of EFFECTS) {
    enabled[e.id] = false; // everything starts off, so the sound is unchanged until you turn something on
    values[e.id] = Object.fromEntries(e.params.map((q) => [q.id, q.value]));
  }
  return { enabled, values };
}

const clamp = (x: number, lo: number, hi: number): number => Math.min(hi, Math.max(lo, x));

/** Merge saved/preset data into a complete state, ignoring unknown names and clamping every value to its range. */
export function mergeFxState(base: FxState, saved: unknown): FxState {
  const out: FxState = { enabled: { ...base.enabled }, values: Object.fromEntries(Object.entries(base.values).map(([k, v]) => [k, { ...v }])) as FxState['values'] };
  if (!saved || typeof saved !== 'object') return out;
  const s = saved as { enabled?: Record<string, unknown>; values?: Record<string, Record<string, unknown>> };
  for (const e of EFFECTS) {
    if (typeof s.enabled?.[e.id] === 'boolean') out.enabled[e.id] = s.enabled[e.id] as boolean;
    const vals = s.values?.[e.id];
    if (!vals || typeof vals !== 'object') continue;
    for (const q of e.params) {
      const v = vals[q.id];
      if (typeof v === 'number' && Number.isFinite(v)) out.values[e.id][q.id] = clamp(v, q.min, q.max);
    }
  }
  return out;
}

export const FX_PRESETS: ReadonlyArray<{ id: string; name: string; state: Partial<{ enabled: Partial<Record<EffectId, boolean>>; values: Partial<Record<EffectId, Record<string, number>>> }> }> = [
  { id: 'clean', name: 'Clean (all off)', state: {} },
  {
    id: 'room',
    name: 'Small room',
    state: { enabled: { reverb: true, compressor: true }, values: { reverb: { size: 0.9, preDelay: 8, damping: 7000, mix: 0.18 }, compressor: { threshold: -22, ratio: 3, makeup: 3 } } },
  },
  {
    id: 'hall',
    name: 'Big hall',
    state: { enabled: { reverb: true, delay: true }, values: { reverb: { size: 3.8, preDelay: 35, damping: 5000, mix: 0.4 }, delay: { time: 420, feedback: 0.3, tone: 3200, mix: 0.18 } } },
  },
  {
    id: 'wide',
    name: 'Wide chorus',
    state: { enabled: { chorus: true, reverb: true }, values: { chorus: { rate: 0.6, depth: 5, delay: 20, mix: 0.55 }, reverb: { size: 1.6, preDelay: 15, damping: 7000, mix: 0.18 } } },
  },
  {
    id: 'lofi',
    name: 'Lo-fi tape',
    state: { enabled: { bitcrush: true, eq: true, distortion: true }, values: { bitcrush: { bits: 10, rate: 3, mix: 0.7 }, distortion: { drive: 6, tone: 4500, level: -4, mix: 0.5 }, eq: { lowGain: 3, highGain: -9, highFreq: 4500, midGain: 0 } } },
  },
  {
    id: 'radio',
    name: 'Old radio',
    state: { enabled: { eq: true, distortion: true, compressor: true }, values: { eq: { lowGain: -12, lowFreq: 300, midGain: 7, midFreq: 1600, midQ: 1.4, highGain: -10, highFreq: 4000 }, distortion: { drive: 14, tone: 3500, level: -8, mix: 0.6 }, compressor: { threshold: -30, ratio: 8, attack: 5, release: 120, makeup: 6 } } },
  },
  {
    id: 'robot',
    name: 'Robot choir',
    state: { enabled: { bitcrush: true, delay: true, reverb: true }, values: { bitcrush: { bits: 5, rate: 6, mix: 0.6 }, delay: { time: 120, feedback: 0.55, tone: 5000, mix: 0.4 }, reverb: { size: 2.6, preDelay: 12, damping: 8000, mix: 0.3 } } },
  },
  {
    id: 'vocal',
    name: 'Polished vocal',
    state: { enabled: { compressor: true, eq: true, reverb: true }, values: { compressor: { threshold: -26, ratio: 3.5, attack: 12, release: 180, knee: 14, makeup: 4 }, eq: { lowGain: -3, lowFreq: 120, midGain: 2, midFreq: 3000, midQ: 0.9, highGain: 4, highFreq: 9000 }, reverb: { size: 1.4, preDelay: 18, damping: 9000, mix: 0.16 } } },
  },
  {
    id: 'warm',
    name: 'Warm and close',
    state: { enabled: { eq: true, compressor: true, distortion: true }, values: { eq: { lowGain: 4, lowFreq: 200, midGain: -2, midFreq: 900, midQ: 0.8, highGain: -4, highFreq: 5000 }, compressor: { threshold: -22, ratio: 3, attack: 15, release: 220, knee: 16, makeup: 3 }, distortion: { drive: 4, tone: 3500, level: -3, mix: 0.25 } } },
  },
  {
    id: 'air',
    name: 'Bright and airy',
    state: { enabled: { eq: true, chorus: true, reverb: true }, values: { eq: { lowGain: -5, lowFreq: 150, midGain: 1, midFreq: 2500, midQ: 1, highGain: 8, highFreq: 8000 }, chorus: { rate: 0.4, depth: 3, delay: 16, mix: 0.3 }, reverb: { size: 2.4, preDelay: 25, damping: 14000, mix: 0.3 } } },
  },
  {
    id: 'cathedral',
    name: 'Cathedral',
    state: { enabled: { reverb: true, eq: true }, values: { reverb: { size: 6, preDelay: 60, damping: 3500, mix: 0.6 }, eq: { lowGain: -4, lowFreq: 160, midGain: 0, midFreq: 1200, midQ: 1, highGain: -3, highFreq: 6000 } } },
  },
  {
    id: 'slapback',
    name: 'Slapback echo',
    state: { enabled: { delay: true, compressor: true }, values: { delay: { time: 110, feedback: 0.12, tone: 5000, mix: 0.35 }, compressor: { threshold: -24, ratio: 3, attack: 10, release: 150, makeup: 2 } } },
  },
  {
    id: 'dub',
    name: 'Dub echo',
    state: { enabled: { delay: true, reverb: true, eq: true }, values: { delay: { time: 560, feedback: 0.65, tone: 2400, mix: 0.45 }, reverb: { size: 2.8, preDelay: 30, damping: 3500, mix: 0.3 }, eq: { lowGain: 5, lowFreq: 120, midGain: -3, midFreq: 800, midQ: 1, highGain: -6, highFreq: 4000 } } },
  },
  {
    id: 'dream',
    name: 'Dreamy',
    state: { enabled: { chorus: true, delay: true, reverb: true }, values: { chorus: { rate: 0.25, depth: 7, delay: 24, mix: 0.6 }, delay: { time: 480, feedback: 0.5, tone: 3000, mix: 0.3 }, reverb: { size: 5, preDelay: 40, damping: 5500, mix: 0.5 } } },
  },
  {
    id: 'underwater',
    name: 'Underwater',
    state: { enabled: { eq: true, chorus: true, reverb: true }, values: { eq: { lowGain: 6, lowFreq: 180, midGain: -4, midFreq: 1000, midQ: 0.7, highGain: -15, highFreq: 1800 }, chorus: { rate: 0.35, depth: 10, delay: 28, mix: 0.7 }, reverb: { size: 2, preDelay: 10, damping: 1800, mix: 0.35 } } },
  },
  {
    id: 'telephone',
    name: 'Telephone',
    state: { enabled: { eq: true, distortion: true, compressor: true }, values: { eq: { lowGain: -15, lowFreq: 400, midGain: 9, midFreq: 1800, midQ: 2, highGain: -15, highFreq: 3400 }, distortion: { drive: 8, tone: 3000, level: -6, mix: 0.4 }, compressor: { threshold: -28, ratio: 10, attack: 3, release: 100, makeup: 8 } } },
  },
  {
    id: 'megaphone',
    name: 'Megaphone',
    state: { enabled: { eq: true, distortion: true, compressor: true }, values: { eq: { lowGain: -12, lowFreq: 350, midGain: 8, midFreq: 2200, midQ: 1.2, highGain: -6, highFreq: 5000 }, distortion: { drive: 22, tone: 4200, level: -10, mix: 0.75 }, compressor: { threshold: -30, ratio: 12, attack: 2, release: 90, makeup: 9 } } },
  },
  {
    id: 'console',
    name: '8-bit console',
    state: { enabled: { bitcrush: true, eq: true }, values: { bitcrush: { bits: 4, rate: 9, mix: 1 }, eq: { lowGain: -4, lowFreq: 200, midGain: 3, midFreq: 2000, midQ: 1, highGain: -8, highFreq: 6000 } } },
  },
  {
    id: 'broken',
    name: 'Broken speaker',
    state: { enabled: { bitcrush: true, distortion: true, eq: true }, values: { bitcrush: { bits: 7, rate: 5, mix: 0.5 }, distortion: { drive: 30, tone: 2600, level: -12, mix: 0.85 }, eq: { lowGain: -10, lowFreq: 300, midGain: 6, midFreq: 1400, midQ: 2.5, highGain: -12, highFreq: 3000 } } },
  },
  {
    id: 'stadium',
    name: 'Stadium',
    state: { enabled: { delay: true, reverb: true, compressor: true }, values: { delay: { time: 700, feedback: 0.35, tone: 3500, mix: 0.25 }, reverb: { size: 4.5, preDelay: 70, damping: 4500, mix: 0.45 }, compressor: { threshold: -24, ratio: 5, attack: 8, release: 160, makeup: 5 } } },
  },
  {
    id: 'shimmer',
    name: 'Shimmer chorus',
    state: { enabled: { chorus: true, eq: true, reverb: true }, values: { chorus: { rate: 1.6, depth: 4, delay: 14, mix: 0.7 }, eq: { lowGain: -6, lowFreq: 200, midGain: 0, midFreq: 1500, midQ: 1, highGain: 9, highFreq: 7000 }, reverb: { size: 3, preDelay: 20, damping: 12000, mix: 0.35 } } },
  },
];

type Store = Pick<Storage, 'getItem' | 'setItem'>;
const KEY = 'hillsing.fx.v1';

/** The settings the panel edits and the audio rack follows. Saves itself on every change. */
export class FxModel {
  state: FxState = defaultFxState();
  private readonly listeners: Array<(id: EffectId | null) => void> = [];

  constructor(private readonly store: Store | null = null) {
    try {
      const raw = store?.getItem(KEY);
      if (raw) this.state = mergeFxState(defaultFxState(), JSON.parse(raw));
    } catch {
      /* storage unavailable or damaged: start from the defaults */
    }
  }

  onChange(listener: (id: EffectId | null) => void): void {
    this.listeners.push(listener);
  }

  setEnabled(id: EffectId, on: boolean): void {
    this.state.enabled[id] = on;
    this.changed(id);
  }

  setParam(id: EffectId, name: string, value: number): void {
    const spec = effectSpec(id).params.find((q) => q.id === name);
    if (!spec) return;
    this.state.values[id][name] = clamp(value, spec.min, spec.max);
    this.changed(id);
  }

  /** Replace everything (a preset or a reset). */
  load(saved: unknown): void {
    this.state = mergeFxState(defaultFxState(), saved);
    this.changed(null);
  }

  applyPreset(presetId: string): void {
    const pr = FX_PRESETS.find((x) => x.id === presetId);
    if (pr) this.load(pr.state);
  }

  private changed(id: EffectId | null): void {
    try {
      this.store?.setItem(KEY, JSON.stringify(this.state));
    } catch {
      /* fine */
    }
    for (const l of this.listeners) l(id);
  }
}

// ---- the maths --------------------------------------------------------------------------------------------

export const dbToGain = (db: number): number => Math.pow(10, db / 20);

/** Waveshaper table for tanh soft clipping, normalised so full scale in is full scale out. */
export function softClipCurve(n = 4096): Float32Array<ArrayBuffer> {
  const out = new Float32Array(n);
  const k = 2.5; // fixed shape; the drive control is a gain ahead of the shaper
  const norm = Math.tanh(k);
  for (let i = 0; i < n; i++) {
    const x = (i / (n - 1)) * 2 - 1;
    out[i] = Math.tanh(k * x) / norm;
  }
  return out;
}

/** `base` with each named number taken from `saved` (clamped to its spec), ignoring anything else. */
export function mergeNumbers<T extends object>(base: T, saved: unknown, specs: ReadonlyArray<ParamSpec>): T {
  const out = { ...base } as Record<string, number>;
  if (saved && typeof saved === 'object') {
    for (const sp of specs) {
      const v = (saved as Record<string, unknown>)[sp.id];
      if (typeof v === 'number' && Number.isFinite(v)) out[sp.id] = clamp(v, sp.min, sp.max);
    }
  }
  return out as T;
}

/** Map a 0..1 slider position to a value on a log (frequency) or linear scale, and back. */
export function sliderToValue(spec: ParamSpec, t: number): number {
  const u = clamp(t, 0, 1);
  const v = spec.log ? spec.min * Math.pow(spec.max / spec.min, u) : spec.min + (spec.max - spec.min) * u;
  const snapped = Math.round(v / spec.step) * spec.step;
  return clamp(Number(snapped.toFixed(4)), spec.min, spec.max);
}
export function valueToSlider(spec: ParamSpec, v: number): number {
  const x = clamp(v, spec.min, spec.max);
  return spec.log ? Math.log(x / spec.min) / Math.log(spec.max / spec.min) : (x - spec.min) / (spec.max - spec.min);
}

export function formatParam(spec: ParamSpec, v: number): string {
  const digits = spec.step >= 1 ? 0 : spec.step >= 0.1 ? 1 : 2;
  if (spec.id === 'mix' || spec.id === 'feedback') return `${Math.round(v * 100)}%`;
  if (spec.unit === ' Hz' && v >= 1000) return `${(v / 1000).toFixed(1)} kHz`;
  return `${v.toFixed(digits)}${spec.unit}`;
}

/**
 * Stereo reverb impulse response: decaying noise (about 60 dB down at `seconds`), darkening over time through a
 * one-pole low-pass that sweeps from `dampHz` toward a quarter of it, after `preDelayMs` of silence.
 * `seed` makes it repeatable for tests.
 */
export function impulseResponse(sampleRate: number, seconds: number, dampHz: number, preDelayMs: number, seed = 1): [Float32Array, Float32Array] {
  const pre = Math.round((preDelayMs / 1000) * sampleRate);
  const body = Math.max(1, Math.round(seconds * sampleRate));
  const n = pre + body;
  const out: [Float32Array, Float32Array] = [new Float32Array(n), new Float32Array(n)];
  for (let ch = 0; ch < 2; ch++) {
    let s = (seed * 2654435761 + ch * 40503) >>> 0 || 1;
    const rand = (): number => {
      s ^= s << 13;
      s >>>= 0;
      s ^= s >>> 17;
      s ^= s << 5;
      s >>>= 0;
      return s / 2147483648 - 1;
    };
    let lp = 0;
    for (let i = 0; i < body; i++) {
      const t = i / body;
      const env = Math.pow(10, (-60 * t) / 20); // -60 dB at the end
      const fc = dampHz * (1 - 0.75 * t);
      const a = 1 - Math.exp((-2 * Math.PI * fc) / sampleRate);
      lp += a * (rand() - lp);
      out[ch][pre + i] = lp * env;
    }
  }
  return out;
}

/**
 * Bit and sample-rate reduction of one channel's block, in place. `hold` and `held` carry the sample-and-hold state
 * between blocks: `rate` is how many input samples share one output value (fractions allowed).
 */
export function crushBlock(x: Float32Array, bits: number, rate: number, state: { phase: number; held: number }): void {
  const levels = Math.pow(2, Math.max(1, Math.min(16, bits)) - 1);
  const step = 1 / Math.max(1, rate);
  for (let i = 0; i < x.length; i++) {
    state.phase += step;
    if (state.phase >= 1) {
      state.phase -= 1;
      state.held = Math.round(x[i] * levels) / levels;
    }
    x[i] = state.held;
  }
}
