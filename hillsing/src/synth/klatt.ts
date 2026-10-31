// Real-time formant synthesizer in the style of Klatt (1980), "Software for a
// cascade/parallel formant synthesizer", J. Acoust. Soc. Am. 67(3), 971-995.
//
// Simplified: glottal pulse train (+ aspiration noise) -> optional nasal notch ->
// cascade of five formant resonators, plus a parallel frication noise branch.
// Pure TypeScript with no Web Audio dependency, so it runs inside an AudioWorklet and in
// plain Node for tests/offline rendering.
//
// Control model: note-driven. noteOn() queues onset consonants then a held vowel; the vowel
// sustains until noteOff(), which queues the coda consonants and a fade-out. A legato noteOn()
// (a note starting while another still sounds) does not release: it plays the previous
// syllable's coda and the new syllable's onset *between* the two vowels, so consonants are
// never lost, then glides into the new vowel at the new pitch.

import { buildSyllable, TAIL_SEGMENT, vowelFormants } from './phonemes';
import { CLASSIC, type VoiceProfile } from './voices';
import {
  DEFAULT_TARGETS,
  TARGET_KEYS,
  type Segment,
  type SyllablePlan,
  type Targets,
} from './types';

/** Parameters are updated every BLOCK samples (control rate = fs / BLOCK). */
const BLOCK = 16;

/**
 * Fixed formants above F5 as [frequency Hz, bandwidth Hz]. A cascade of only five formants
 * makes a synthesized vowel's spectrum collapse above ~4 kHz (measured: about -70 dB at 5 kHz
 * and -105 dB above 6 kHz), which sounds muffled and robs consonants of clarity. Real tracts
 * keep resonating at 5-7 kHz, and formant synthesizers conventionally add upper poles to
 * compensate (Klatt 1980 "higher-pole correction").
 */
export const UPPER_POLES: Array<[number, number]> = [
  [4900, 700],
  [6300, 900],
];

const VOICE_GAIN = 0.036;

/**
 * Loudness every sustained vowel is brought to (RMS at velocity 0.8, volume 0.8). Loud enough
 * to be clear, low enough that peaks stay well under the output limiter.
 */
export const TARGET_RMS = 0.19;
const PROBE_VELOCITY = 0.8;
const PROBE_VOLUME = 0.8;
const MAX_VOWEL_GAIN = 40;
/** Samples per period when computing the source's harmonics, and shared cos/sin tables for that. */
const PULSE_POINTS = 1024;
const COS_T = new Float64Array(PULSE_POINTS);
const SIN_T = new Float64Array(PULSE_POINTS);
for (let i = 0; i < PULSE_POINTS; i++) {
  COS_T[i] = Math.cos((2 * Math.PI * i) / PULSE_POINTS);
  SIN_T[i] = Math.sin((2 * Math.PI * i) / PULSE_POINTS);
}
const FRIC_GAIN = 0.5;
/**
 * Default corner of the consonant noise low-pass (two cascaded one-pole filters, 12 dB/octave).
 * Vowels carry almost nothing above 5 kHz, so unfiltered /s/ and burst noise reached 8 kHz and
 * beyond as sudden spikes at every syllable edge, heard as a scratch. Adjustable (see `noiseRolloff`).
 */
const DEFAULT_NOISE_ROLLOFF_HZ = 6500;

/**
 * Smooth loudness limiter ahead of the output soft-clipper. Diphthong glides, voiced consonants
 * and some vowel/pitch combinations surge 3-4 dB above a steady vowel (equal sustained RMS does not
 * mean equal level at every moment), heard as a pop at syllable edges. It watches short-term RMS,
 * not peaks: steady vowels sit at TARGET_RMS and are untouched, while surges are pulled back. Gain
 * moves smoothly (milliseconds), so the limiter itself adds no harmonics.
 */
const LIMIT_RMS = 0.215;
const LIMIT_WINDOW_SEC = 0.008;
const LIMIT_ATTACK_SEC = 0.003;
const LIMIT_RELEASE_SEC = 0.08;

class Resonator {
  private y1 = 0;
  private y2 = 0;
  private a = 1;
  private b = 0;
  private c = 0;

  set(freq: number, bw: number, fs: number): void {
    const f = Math.min(Math.max(freq, 20), fs * 0.45);
    const c = -Math.exp((-2 * Math.PI * bw) / fs);
    const b = 2 * Math.exp((-Math.PI * bw) / fs) * Math.cos((2 * Math.PI * f) / fs);
    this.a = 1 - b - c;
    this.b = b;
    this.c = c;
  }

  process(x: number): number {
    const y = this.a * x + this.b * this.y1 + this.c * this.y2;
    this.y2 = this.y1;
    this.y1 = y;
    return y;
  }

  reset(): void {
    this.y1 = this.y2 = 0;
  }
}

/**
 * Second-order band-pass (RBJ cookbook, constant 0 dB peak gain) for frication noise. A
 * DC-normalised resonator is the wrong tool here: it passes everything below its centre at
 * full gain, which made /s/ a broadband hiss instead of high-frequency noise.
 */
class Bandpass {
  private x1 = 0;
  private x2 = 0;
  private y1 = 0;
  private y2 = 0;
  private b0 = 0;
  private a1 = 0;
  private a2 = 0;

  set(freq: number, bw: number, fs: number): void {
    const f = Math.min(Math.max(freq, 100), fs * 0.45);
    const q = Math.max(0.3, f / Math.max(50, bw));
    const w = (2 * Math.PI * f) / fs;
    const alpha = Math.sin(w) / (2 * q);
    const a0 = 1 + alpha;
    this.b0 = alpha / a0;
    this.a1 = (-2 * Math.cos(w)) / a0;
    this.a2 = (1 - alpha) / a0;
  }

  process(x: number): number {
    const y = this.b0 * (x - this.x2) - this.a1 * this.y1 - this.a2 * this.y2;
    this.x2 = this.x1;
    this.x1 = x;
    this.y2 = this.y1;
    this.y1 = y;
    return y;
  }

  reset(): void {
    this.x1 = this.x2 = this.y1 = this.y2 = 0;
  }
}

/** Second-order notch (stand-in for Klatt's nasal anti-resonance), unity gain at DC. */
class Notch {
  private x1 = 0;
  private x2 = 0;
  private y1 = 0;
  private y2 = 0;
  private b1 = 0;
  private a1 = 0;
  private a2 = 0;
  private g = 1;

  set(freq: number, bw: number, fs: number): void {
    const f = Math.min(Math.max(freq, 50), fs * 0.45);
    const r = Math.exp((-Math.PI * bw) / fs);
    const cw = Math.cos((2 * Math.PI * f) / fs);
    this.b1 = -2 * cw;
    this.a1 = -2 * r * cw;
    this.a2 = r * r;
    this.g = (1 + this.a1 + this.a2) / (2 + this.b1);
  }

  process(x: number): number {
    const y = this.g * (x + this.b1 * this.x1 + this.x2) - this.a1 * this.y1 - this.a2 * this.y2;
    this.x2 = this.x1;
    this.x1 = x;
    this.y2 = this.y1;
    this.y1 = y;
    return y;
  }

  reset(): void {
    this.x1 = this.x2 = this.y1 = this.y2 = 0;
  }
}

/** Rosenberg-style glottal flow: rises over `tp`, falls over `tn`, closed for the rest. */
function glottalFlow(p: number, tp: number, tn: number): number {
  if (p < tp) return 0.5 * (1 - Math.cos((Math.PI * p) / tp));
  if (p < tp + tn) return Math.cos((Math.PI / 2) * ((p - tp) / tn));
  return 0;
}

/**
 * Source scale. The flow pulse has amplitude 1 whatever its shape, so the low harmonics (which
 * dominate loudness) do not depend on how fast the pulse closes; only the high harmonics do.
 * So all voices share one scale (per-vowel loudness is equalised per note, see vowelGain).
 */
const PULSE_SCALE = 0.29;

export type VoiceParamName =
  | 'formantShift'
  | 'consonantScale'
  | 'portamento'
  | 'vibratoDepth'
  | 'vibratoRate'
  | 'volume'
  | 'consonantLevel'
  | 'breathiness'
  | 'brightness'
  | 'noiseRolloff'
  | 'burstLevel'
  | 'fricLevel'
  | 'aspLevel';

export class KlattVoice {
  readonly fs: number;

  /** Multiplies all vowel formants (voice "size"/gender character). 1 = unchanged. */
  formantShift = 1;
  /** Consonant duration multiplier. Lower = vowel lands sooner after the key press. */
  consonantScale = 0.7;
  /** Time (ms) for pitch to glide between legato notes. */
  portamento = 45;
  /** 0..1, wired to the mod wheel. 1 = +/-80 cents at vibratoRate. */
  vibratoDepth = 0;
  vibratoRate = 5.5;
  volume = 0.8;
  /** Extra multiplier on consonant noise, on top of the voice's own level. */
  consonantLevel = 1;
  /** Extra breath noise (0..1) added to the voice's own breathiness. */
  breathiness = 0;
  /** -1 (darker) .. +1 (brighter): a high shelf over the whole output. 0 = off. */
  brightness = 0;
  /** Corner (Hz) of the low-pass on consonant noise: lower = softer, darker hiss and bursts. */
  noiseRolloff = DEFAULT_NOISE_ROLLOFF_HZ;
  /** Level of each consonant noise class (applied when a syllable starts): stop bursts, fricatives, aspiration. */
  burstLevel = 1;
  fricLevel = 1;
  aspLevel = 1;

  private profile: VoiceProfile = CLASSIC;
  private tp = 0.45;
  private tn = 0.04;
  private pulseScale = PULSE_SCALE;

  private active = false;
  private cur: Targets = { ...DEFAULT_TARGETS };
  private from: Targets = { ...DEFAULT_TARGETS };
  private to: Targets = { ...DEFAULT_TARGETS };
  private queue: Segment[] = [];
  private qi = 0;
  private elapsed = 0; // samples in the current segment
  private transSamples = 1;
  private durSamples = 0;
  private minHoldSamples = 0;

  // Loudness compensation during formant movement (see applyAgc).
  private avFrom = 0;
  private avTo = 0;
  private agcCount = 0;
  private agcRf = 1;
  private agcRt = 1;

  private noteHeld = false;
  private releaseRequested = false;
  private pendingCoda: Segment[] = [];
  private codaQueued = false;
  private velocity = 1;

  // Pitch state (log domain for natural glides).
  private logF0Target = Math.log(130);
  private logF0 = Math.log(130);
  private f0 = 130;
  private vibPhase = 0;
  private vibEnv = 0;
  private drift = 0;

  // Source / filter state.
  private phase = 0;
  private gPrev = 0;
  private tiltY = 0;
  private tiltA = 0.6;
  private shelfGain = 0;
  private shelfLp = 0;
  private readonly shelfA: number;
  private seed = 0x1234abcd;
  private readonly r1 = new Resonator();
  private readonly r2 = new Resonator();
  private readonly r3 = new Resonator();
  private readonly r4 = new Resonator();
  private readonly r5 = new Resonator();
  /** Fixed upper poles standing in for the real tract's higher resonances (see UPPER_POLES). */
  private readonly ru = UPPER_POLES.map(() => new Resonator());
  private readonly rf = new Bandpass();
  private fricNorm = 1;
  private limGain = 1;
  private limMs = 0;
  private readonly limWin: number;
  private readonly limAtk: number;
  private readonly limRel: number;
  private nl1 = 0;
  private nl2 = 0;
  private noiseA = 0;
  private readonly notch = new Notch();

  private readonly ampCache = new Map<string, Float64Array>();

  constructor(sampleRate: number) {
    this.fs = sampleRate;
    this.shelfA = 1 - Math.exp((-2 * Math.PI * 2000) / sampleRate);
    this.limWin = 1 - Math.exp(-1 / (LIMIT_WINDOW_SEC * sampleRate));
    this.limAtk = 1 - Math.exp(-1 / (LIMIT_ATTACK_SEC * sampleRate));
    this.limRel = 1 - Math.exp(-1 / (LIMIT_RELEASE_SEC * sampleRate));
    this.cur.av = this.cur.ah = this.cur.af = this.cur.nz = 0;
    this.setProfile(CLASSIC);
  }

  get voice(): VoiceProfile {
    return this.profile;
  }

  /**
   * Gain that brings vowel `key` to TARGET_RMS at pitch `f0` in the current voice. Loudness depends
   * on where the harmonics land relative to the formants (up to +-8 dB across the singing range),
   * which no fixed per-vowel table can capture, so it is calculated from the source spectrum and
   * the filters' responses (see analyticGain).
   */
  private vowelGain(key: string, f0: number): number {
    return this.analyticGain({ ...vowelFormants(key, this.profile), nz: 0 }, f0);
  }

  /** Like vowelGain, for voicing through arbitrary formants (a voiced consonant's locus, a liquid, a nasal with its notch). */
  private formantGain(t: Partial<Targets>, f0: number): number {
    return this.analyticGain(t, f0);
  }

  /**
   * Harmonic amplitudes (k = 1..K) of the glottal source at `f0`, as the render loop produces it:
   * the flow pulse differentiated (lip radiation) and scaled by PULSE_SCALE. Cached per (voice, f0).
   */
  private harmonicAmps(f0: number): Float64Array {
    const key = `${this.profile.id}|${f0.toFixed(1)}`;
    const hit = this.ampCache.get(key);
    if (hit) return hit;
    const tp = this.profile.openPhase;
    const tn = this.closeFraction(this.profile.closeMs, f0);
    const N = PULSE_POINTS;
    const gp = new Float64Array(N);
    for (let n = 0; n < N; n++) {
      const p = (n + 0.5) / N;
      gp[n] =
        p < tp ? 0.5 * (Math.PI / tp) * Math.sin((Math.PI * p) / tp)
        : p < tp + tn ? -(Math.PI / 2 / tn) * Math.sin(((Math.PI / 2) * (p - tp)) / tn)
        : 0;
    }
    const K = Math.max(1, Math.min(Math.floor(Math.min(10000, this.fs * 0.45) / f0), N / 2 - 1));
    const amps = new Float64Array(K);
    for (let k = 1; k <= K; k++) {
      let re = 0, im = 0;
      for (let n = 0; n < N; n++) {
        const idx = (k * n) % N;
        re += gp[n] * COS_T[idx];
        im -= gp[n] * SIN_T[idx];
      }
      const w = (2 * Math.PI * k * f0) / this.fs;
      const diff = (2 * Math.sin(w / 2)) / w; // the render takes a one-sample difference, not a true derivative
      amps[k - 1] = 2 * Math.hypot(re, im) / N * this.pulseScale * diff;
    }
    if (this.ampCache.size > 256) this.ampCache.clear();
    this.ampCache.set(key, amps);
    return amps;
  }

  /**
   * Gain that brings unit voicing through formants `t` (plus nasal notch, breath noise and the
   * brightness shelf) to TARGET_RMS at pitch `f0`: calculated from the source
   * spectrum and the filters' magnitude responses. (An earlier version rendered test audio at every
   * note-on, which took 1-9 ms on the audio thread, longer than its 2.9 ms block, and stuttered.)
   */
  private analyticGain(t: Partial<Targets>, f0: number): number {
    const fs = this.fs;
    const pr = this.profile;
    const shift = this.formantShift;
    const bwS = pr.bandwidthScale;
    const up = pr.upperFormantScale * shift;
    const list: Array<[number, number]> = [
      [(t.f1 ?? 500) * shift, (t.b1 ?? 60) * bwS],
      [(t.f2 ?? 1500) * shift, (t.b2 ?? 90) * bwS],
      [(t.f3 ?? 2500) * shift, (t.b3 ?? 150) * bwS],
      [3300 * up, 250 * bwS],
      [3750 * up, 200 * bwS],
      ...UPPER_POLES.map(([f, b]): [number, number] => [f * up, b * bwS]),
    ];
    const res = list.map(([f, bw]) => {
      const fc = Math.min(Math.max(f, 20), fs * 0.45);
      const c = -Math.exp((-2 * Math.PI * bw) / fs);
      const b = 2 * Math.exp((-Math.PI * bw) / fs) * Math.cos((2 * Math.PI * fc) / fs);
      return [1 - b - c, b, c];
    });
    const nz = t.nz ?? 0;
    const nasal = nz > 0.001;
    let nb1 = 0, na1 = 0, na2 = 0, ng = 1;
    if (nasal) {
      const fz = Math.min(Math.max((t.fz ?? 1000) * shift, 50), fs * 0.45);
      const r = Math.exp((-Math.PI * 120) / fs);
      const cz = Math.cos((2 * Math.PI * fz) / fs);
      nb1 = -2 * cz;
      na1 = -2 * r * cz;
      na2 = r * r;
      ng = (1 + na1 + na2) / (2 + nb1);
    }
    const tiltA = Math.min(0.95, Math.max(0.08, pr.tiltBase + pr.tiltVelocity * PROBE_VELOCITY));
    const sg = this.brightness >= 0 ? 1.5 * this.brightness : 0.6 * this.brightness;
    const response = (w: number): number => {
      const cw = Math.cos(w), sw = Math.sin(w), c2 = Math.cos(2 * w), s2 = Math.sin(2 * w);
      let m = 1;
      for (const [a, b, c] of res) m *= a / Math.hypot(1 - b * cw - c * c2, b * sw + c * s2);
      if (nasal) {
        const num = ng * Math.abs(2 * cw + nb1);
        const den = Math.hypot(1 + na1 * cw + na2 * c2, na1 * sw + na2 * s2);
        m *= 1 - nz + nz * (num / den);
      }
      return m;
    };
    const shelf = (w: number): number => {
      if (sg === 0) return 1;
      // 1 + sg * (1 - LP(w)), LP = one-pole low-pass at 2 kHz
      const d = 1 - (1 - this.shelfA) * Math.cos(w), e = (1 - this.shelfA) * Math.sin(w);
      const den = d * d + e * e;
      const lr = (this.shelfA * d) / den, li = (this.shelfA * e) / den;
      return Math.hypot(1 + sg * (1 - lr), sg * li);
    };
    const amps = this.harmonicAmps(f0);
    let v2 = 0;
    for (let k = 1; k <= amps.length; k++) {
      const w = (2 * Math.PI * k * f0) / fs;
      const tilt = tiltA / Math.hypot(1 - (1 - tiltA) * Math.cos(w), (1 - tiltA) * Math.sin(w));
      const a = amps[k - 1] * tilt * response(w) * shelf(w);
      v2 += (a * a) / 2;
    }
    let n2 = 0;
    const breath = pr.breath + this.breathiness;
    if (breath > 0) {
      let m2 = 0;
      const M = 96;
      for (let i = 0; i < M; i++) {
        const w = (Math.PI * (i + 0.5)) / M;
        const h = response(w);
        m2 += h * h;
      }
      n2 = ((breath * 0.35) ** 2 / 3) * (m2 / M);
    }
    const rmsPre = Math.sqrt(v2 + n2);
    const G = (0.4 + 0.6 * PROBE_VELOCITY) * PROBE_VOLUME;
    const rms = rmsPre * VOICE_GAIN * G;
    return rms > 1e-9 ? Math.min(MAX_VOWEL_GAIN, TARGET_RMS / rms) : 1;
  }

  /** Closing time as a fraction of the period (absolute time, so sharper at low pitch). */
  private closeFraction(closeMs: number, f0: number): number {
    return Math.min(0.3, Math.max(0.01, (closeMs / 1000) * f0));
  }

  /** Switch voice. Takes effect from the next note (vowel targets are chosen at note-on). */
  setProfile(profile: VoiceProfile): void {
    this.profile = profile;
    this.tp = profile.openPhase;
    this.tn = this.closeFraction(profile.closeMs, this.f0);
  }

  get isActive(): boolean {
    return this.active;
  }

  setParam(name: VoiceParamName, value: number): void {
    this[name] = value;
  }

  setF0(f0: number): void {
    this.logF0Target = Math.log(Math.max(20, f0));
  }

  noteOn(plan: SyllablePlan, f0: number, velocity: number, legato: boolean): void {
    const isLegato = legato && this.active;
    const segs = buildSyllable(plan, this.consonantScale, isLegato, this.profile, (k) => this.vowelGain(k, f0), {
      burst: this.burstLevel,
      fric: this.fricLevel,
      asp: this.aspLevel,
    }, (t) => this.formantGain(t, f0));
    this.velocity = Math.min(1, Math.max(0, velocity));
    // Legato: the previous syllable's final consonants have not been sung yet (they normally
    // follow note-off), so sing them now, then this syllable's onset, then glide into the vowel.
    const connecting = isLegato && this.noteHeld && !this.codaQueued ? this.pendingCoda : [];
    this.pendingCoda = segs.coda;
    this.codaQueued = false;
    this.noteHeld = true;
    this.releaseRequested = false;
    this.setF0(f0);

    const first = [...connecting, ...segs.onset, ...segs.nucleus];
    if (!isLegato) {
      this.logF0 = this.logF0Target;
      this.vibEnv = 0;
      this.vibPhase = 0;
    }
    // Never start a segment instantly: a few ms of ramp avoids clicks on restarts.
    first[0] = { ...first[0], trans: Math.max(first[0].trans, 4) };
    this.queue = first;
    this.active = true;
    this.startSegment(0);
  }

  noteOff(): void {
    if (!this.active || !this.noteHeld) return;
    this.noteHeld = false;
    this.releaseRequested = true;
    if (!this.codaQueued) {
      this.queue.push(...this.pendingCoda, TAIL_SEGMENT);
      this.codaQueued = true;
    }
  }

  /**
   * Run a syllable through the whole pipeline once, silently, so the JavaScript engine has compiled
   * the hot paths before the first real note (a cold first note-on takes ~2 ms, close to the audio
   * block budget). Leaves the voice idle.
   */
  warmUp(): void {
    const scratch = new Float32Array(128);
    for (const f0 of [130, 220]) {
      this.noteOn({ onset: ['S', 'T', 'R'], nucleus: 'AH1', coda: ['M', 'P', 'T'] }, f0, 0.8, false);
      for (let i = 0; i < 40; i++) this.render(scratch);
      this.noteOff();
      for (let i = 0; i < 40; i++) this.render(scratch);
    }
    this.stop();
  }

  /** Immediately silence everything (panic). */
  stop(): void {
    this.active = false;
    this.noteHeld = false;
    this.releaseRequested = false;
    this.cur.av = this.cur.ah = this.cur.af = this.cur.nz = 0;
    for (const r of [this.r1, this.r2, this.r3, this.r4, this.r5, this.rf, ...this.ru]) r.reset();
    this.nl1 = this.nl2 = 0;
    this.limGain = 1;
    this.limMs = 0;
    this.notch.reset();
  }

  private startSegment(i: number): void {
    const s = this.queue[i];
    this.qi = i;
    this.elapsed = 0;
    this.from = { ...this.cur };
    this.to = { ...this.cur, ...s.t };
    // The noise filter must not sweep while noise fades in (the sweep is audible as a scratch):
    // place it at its new centre while it is still quiet.
    if (this.to.af > this.from.af) {
      this.from.ff = this.to.ff;
      this.from.bf = this.to.bf;
    }
    this.avFrom = this.from.av;
    this.avTo = this.to.av;
    this.agcCount = 0;
    this.transSamples = Math.max(1, (s.trans * this.fs) / 1000);
    this.durSamples = (s.dur * this.fs) / 1000;
    this.minHoldSamples = ((s.minHold ?? 0) * this.fs) / 1000;
  }

  private advance(): void {
    const next = this.qi + 1;
    if (next < this.queue.length) {
      this.startSegment(next);
    } else {
      this.active = false;
      this.cur.av = this.cur.ah = this.cur.af = this.cur.nz = 0;
    }
  }

  /**
   * One control-rate update covering the next `n` samples: parameter interpolation, segment
   * sequencing, pitch, filters. `n` is the real chunk length (<= BLOCK), so timing stays exact
   * when a render call is split at sequencer command boundaries.
   */
  private tick(n: number): void {
    this.elapsed += n;
    const seg = this.queue[this.qi];
    const a = Math.min(1, this.elapsed / this.transSamples);
    // Amplitudes (voicing, aspiration, frication) fade along a raised cosine: a straight ramp has
    // hard corners at both ends, which are audible as ticks at every consonant edge. Formants and
    // the other parameters stay linear.
    const ae = 0.5 - 0.5 * Math.cos(Math.PI * a);
    for (const k of TARGET_KEYS) {
      const w = k === 'av' || k === 'ah' || k === 'af' ? ae : a;
      this.cur[k] = this.from[k] + (this.to[k] - this.from[k]) * w;
    }
    if (a < 1) this.applyAgc(ae);

    if (seg.hold) {
      if (this.releaseRequested && this.elapsed >= this.minHoldSamples) this.advance();
    } else if (this.elapsed >= this.durSamples) {
      this.advance();
    }

    // Pitch: glide, vibrato, tiny random drift for naturalness.
    const dt = n / this.fs;
    const glideTau = Math.max(0.001, this.portamento / 1000 / 3);
    this.logF0 += (this.logF0Target - this.logF0) * (1 - Math.exp(-dt / glideTau));
    this.vibPhase += this.vibratoRate * dt;
    if (this.vibPhase > 1) this.vibPhase -= 1;
    this.vibEnv = Math.min(1, this.vibEnv + dt / 0.25);
    const cents = this.vibratoDepth * 80 * this.vibEnv * Math.sin(2 * Math.PI * this.vibPhase);
    this.drift = this.drift * 0.995 + this.rand() * 0.0004 * this.profile.drift; // zero-mean: rand() is in [-1, 1)
    this.f0 = Math.exp(this.logF0) * Math.pow(2, cents / 1200) * (1 + this.drift);
    this.tn = this.closeFraction(this.profile.closeMs, this.f0);

    // Filter coefficients from the current parameters.
    const c = this.cur;
    const shift = this.formantShift;
    const bw = this.profile.bandwidthScale;
    const up = this.profile.upperFormantScale * shift;
    this.r1.set(c.f1 * shift, c.b1 * bw, this.fs);
    this.r2.set(c.f2 * shift, c.b2 * bw, this.fs);
    this.r3.set(c.f3 * shift, c.b3 * bw, this.fs);
    this.r4.set(3300 * up, 250 * bw, this.fs);
    this.r5.set(3750 * up, 200 * bw, this.fs);
    for (let i = 0; i < this.ru.length; i++) this.ru[i].set(UPPER_POLES[i][0] * up, UPPER_POLES[i][1] * bw, this.fs);
    // Source brightness: voice default + velocity (louder = brighter).
    const pr = this.profile;
    this.tiltA = Math.min(0.95, Math.max(0.08, pr.tiltBase + pr.tiltVelocity * this.velocity));
    // User brightness: a high shelf above ~2 kHz. -1 -> about -8 dB, +1 -> about +8 dB; exactly
    // a no-op at 0, so voices are unchanged until the control is moved.
    this.shelfGain = this.brightness >= 0 ? 1.5 * this.brightness : 0.6 * this.brightness;
    this.rf.set(c.ff, c.bf, this.fs);
    this.noiseA = 1 - Math.exp((-2 * Math.PI * Math.min(this.noiseRolloff, this.fs * 0.45)) / this.fs);
    // Equalise noise power across bandwidths (equivalent noise bandwidth of a 2nd-order
    // band-pass is about (pi/2) * bandwidth), so `af` means the same loudness for any shape.
    this.fricNorm = Math.min(6, Math.sqrt(this.fs / 2 / (1.57 * Math.max(200, c.bf))));
    if (c.nz > 0.001) this.notch.set(c.fz * shift, 120, this.fs);
  }

  /**
   * Approximate steady-state loudness of unit voicing through a formant set at the current pitch:
   * the RMS sum of the source harmonics (about -6 dB/octave, softened above 3 kHz) weighted by the
   * magnitude response of F1-F3 and the nasal notch. Only ratios of this are used.
   */
  private response(t: Targets): number {
    const fs = this.fs;
    const shift = this.formantShift;
    const bw = this.profile.bandwidthScale;
    const res: Array<[number, number, number]> = []; // a, b, c
    for (const [f, b] of [[t.f1, t.b1], [t.f2, t.b2], [t.f3, t.b3]] as const) {
      const fc = Math.min(Math.max(f * shift, 20), fs * 0.45);
      const bwHz = b * bw;
      const c = -Math.exp((-2 * Math.PI * bwHz) / fs);
      const bb = 2 * Math.exp((-Math.PI * bwHz) / fs) * Math.cos((2 * Math.PI * fc) / fs);
      res.push([1 - bb - c, bb, c]);
    }
    const nasal = t.nz > 0.001;
    let nb1 = 0, na1 = 0, na2 = 0, ng = 1, ncos = 0;
    if (nasal) {
      const fz = Math.min(Math.max(t.fz * shift, 50), fs * 0.45);
      const r = Math.exp((-Math.PI * 120) / fs);
      ncos = Math.cos((2 * Math.PI * fz) / fs);
      nb1 = -2 * ncos;
      na1 = -2 * r * ncos;
      na2 = r * r;
      ng = (1 + na1 + na2) / (2 + nb1);
    }
    const f0 = Math.max(40, this.f0);
    const top = Math.min(4500, fs * 0.45);
    let sum = 0;
    for (let k = 1, f = f0; f < top; k++, f = k * f0) {
      const w = (2 * Math.PI * f) / fs;
      const cw = Math.cos(w), sw = Math.sin(w), c2 = Math.cos(2 * w), s2 = Math.sin(2 * w);
      let m = 1 / k / Math.sqrt(1 + (f / 3000) * (f / 3000));
      for (const [a, b, c] of res) m *= a / Math.hypot(1 - b * cw - c * c2, b * sw + c * s2);
      if (nasal) {
        const num = ng * Math.abs(2 * cw + nb1);
        const den = Math.hypot(1 + na1 * cw + na2 * c2, na1 * sw + na2 * s2);
        m *= 1 - t.nz + t.nz * (num / den);
      }
      sum += m * m;
    }
    return Math.sqrt(sum);
  }

  /**
   * Keep loudness (not just the voicing amplitude) moving smoothly between two segments. Vowel and
   * consonant gains are matched at their settled formants, but while the formants sweep, a
   * harmonic crossing a narrow resonance swells the output 4-6 dB for a few milliseconds (a pop
   * on the vowel after /dʒ/, /b/, /l/ ... at some pitches). The target loudness is interpolated
   * between the two ends' loudness, and voicing is divided by the response of the formants as they
   * are right now. Settled segments are unaffected.
   */
  private applyAgc(w: number): void {
    if (this.avFrom <= 0 && this.avTo <= 0) return;
    if ((this.agcCount++ & 3) === 0) {
      this.agcRf = this.response(this.from);
      this.agcRt = this.response(this.to);
    }
    const rc = this.response(this.cur);
    const lin = this.cur.av;
    const want = ((1 - w) * this.avFrom * this.agcRf + w * this.avTo * this.agcRt) / Math.max(rc, 1e-9);
    this.cur.av = Math.min(lin * 3, Math.max(lin * 0.3, want));
  }

  private rand(): number {
    // xorshift32 -> [-1, 1)
    let x = this.seed;
    x ^= x << 13;
    x ^= x >>> 17;
    x ^= x << 5;
    this.seed = x;
    return (x >>> 0) / 2147483648 - 1;
  }

  /** Render into `out` (mono). Output is zero while idle. */
  render(out: Float32Array): void {
    const n = out.length;
    for (let b = 0; b < n; b += BLOCK) {
      const m = Math.min(BLOCK, n - b);
      if (!this.active) {
        out.fill(0, b);
        return;
      }
      this.tick(m);
      if (!this.active) {
        out.fill(0, b);
        return;
      }
      const c = this.cur;
      const inc = this.f0 / this.fs;
      const norm = (this.fs / this.f0) * this.pulseScale;
      const tp = this.tp;
      const tn = this.tn;
      const breath = this.profile.breath + this.breathiness;
      const cl = this.profile.consonantLevel * this.consonantLevel;
      const afNow = c.af * cl;
      const ahNow = c.ah * cl + breath * Math.min(1, c.av);
      const gain = (0.4 + 0.6 * this.velocity) * this.volume;
      const useNasal = c.nz > 0.001;
      for (let j = 0; j < m; j++) {
        // Rosenberg-style glottal flow, differentiated (lip radiation), then tilt filter.
        this.phase += inc;
        if (this.phase >= 1) this.phase -= 1;
        const p = this.phase;
        const g = glottalFlow(p, tp, tn);
        const dg = (g - this.gPrev) * norm;
        this.gPrev = g;
        this.tiltY += this.tiltA * (dg - this.tiltY);

        const noise = this.rand();
        let src = c.av * this.tiltY + ahNow * noise * 0.35;
        if (useNasal) src = src * (1 - c.nz) + this.notch.process(src) * c.nz;
        let y = this.r1.process(src);
        y = this.r2.process(y);
        y = this.r3.process(y);
        y = this.r4.process(y);
        y = this.r5.process(y);
        for (let i = 0; i < this.ru.length; i++) y = this.ru[i].process(y);
        this.nl1 += this.noiseA * (this.rf.process(noise * afNow) * this.fricNorm - this.nl1);
        this.nl2 += this.noiseA * (this.nl1 - this.nl2);
        const fr = this.nl2;
        let mix = y * VOICE_GAIN + fr * FRIC_GAIN;
        this.shelfLp += this.shelfA * (mix - this.shelfLp);
        mix += this.shelfGain * (mix - this.shelfLp);
        const s = mix * gain;
        this.limMs += (s * s - this.limMs) * this.limWin;
        const lr = Math.sqrt(this.limMs);
        const want = lr > LIMIT_RMS ? LIMIT_RMS / lr : 1;
        this.limGain += (want - this.limGain) * (want < this.limGain ? this.limAtk : this.limRel);
        out[b + j] = Math.tanh(s * this.limGain);
      }
    }
  }
}
