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

import { buildSyllable, TAIL_SEGMENT } from './phonemes';
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
const MAX_VOWEL_GAIN = 8;
const FRIC_GAIN = 0.5;

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
  | 'brightness';

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
  private readonly notch = new Notch();

  /** True for the internal loudness-probe instance (which must not probe itself). */
  private readonly isProbe: boolean;
  private probe: KlattVoice | null = null;
  private readonly gainCache = new Map<string, number>();

  constructor(sampleRate: number, isProbe = false) {
    this.fs = sampleRate;
    this.isProbe = isProbe;
    this.shelfA = 1 - Math.exp((-2 * Math.PI * 2000) / sampleRate);
    this.cur.av = this.cur.ah = this.cur.af = this.cur.nz = 0;
    this.setProfile(CLASSIC);
  }

  get voice(): VoiceProfile {
    return this.profile;
  }

  /**
   * Gain that brings vowel `key` to TARGET_RMS at pitch `f0` in the current voice. Measured by
   * briefly rendering the vowel in a private probe voice, because loudness depends on where
   * the harmonics land relative to the formants (up to +-8 dB across the singing range), which
   * no fixed per-vowel table can capture. Cached per (voice, vowel, pitch, shift, ...).
   */
  private vowelGain(key: string, f0: number): number {
    if (this.isProbe) return 1;
    const quarterSemis = Math.round(48 * Math.log2(f0 / 440));
    const id = [
      this.profile.id, key, quarterSemis,
      this.formantShift.toFixed(2), this.brightness.toFixed(1), this.breathiness.toFixed(1),
    ].join('|');
    const hit = this.gainCache.get(id);
    if (hit !== undefined) return hit;

    const probe = (this.probe ??= new KlattVoice(this.fs, true));
    probe.setProfile(this.profile);
    probe.formantShift = this.formantShift;
    probe.brightness = this.brightness;
    probe.breathiness = this.breathiness;
    probe.volume = PROBE_VOLUME;
    probe.stop();
    probe.noteOn({ onset: [], nucleus: key + '1', coda: [] }, f0, PROBE_VELOCITY, false);
    const warm = Math.round(0.07 * this.fs);
    const measure = Math.max(Math.round(0.03 * this.fs), Math.round((4 * this.fs) / f0));
    const buf = new Float32Array(measure);
    probe.render(new Float32Array(warm)); // let the formants ring up past the vowel transition
    probe.render(buf);
    let sum = 0;
    for (let i = 0; i < measure; i++) sum += buf[i] * buf[i];
    const rms = Math.sqrt(sum / measure);
    const gain = rms > 1e-6 ? Math.min(MAX_VOWEL_GAIN, TARGET_RMS / rms) : 1;
    if (this.gainCache.size > 4000) this.gainCache.clear();
    this.gainCache.set(id, gain);
    return gain;
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
    const segs = buildSyllable(plan, this.consonantScale, isLegato, this.profile, (k) => this.vowelGain(k, f0));
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

  /** Immediately silence everything (panic). */
  stop(): void {
    this.active = false;
    this.noteHeld = false;
    this.releaseRequested = false;
    this.cur.av = this.cur.ah = this.cur.af = this.cur.nz = 0;
    for (const r of [this.r1, this.r2, this.r3, this.r4, this.r5, this.rf, ...this.ru]) r.reset();
    this.notch.reset();
  }

  private startSegment(i: number): void {
    const s = this.queue[i];
    this.qi = i;
    this.elapsed = 0;
    this.from = { ...this.cur };
    this.to = { ...this.cur, ...s.t };
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
    for (const k of TARGET_KEYS) this.cur[k] = this.from[k] + (this.to[k] - this.from[k]) * a;

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
    // Equalise noise power across bandwidths (equivalent noise bandwidth of a 2nd-order
    // band-pass is about (pi/2) * bandwidth), so `af` means the same loudness for any shape.
    this.fricNorm = Math.min(6, Math.sqrt(this.fs / 2 / (1.57 * Math.max(200, c.bf))));
    if (c.nz > 0.001) this.notch.set(c.fz * shift, 120, this.fs);
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
        const fr = this.rf.process(noise * afNow) * this.fricNorm;
        let mix = y * VOICE_GAIN + fr * FRIC_GAIN;
        this.shelfLp += this.shelfA * (mix - this.shelfLp);
        mix += this.shelfGain * (mix - this.shelfLp);
        out[b + j] = Math.tanh(mix * gain);
      }
    }
  }
}
