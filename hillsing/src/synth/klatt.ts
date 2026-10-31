// Real-time formant synthesizer in the style of Klatt (1980), "Software for a
// cascade/parallel formant synthesizer", J. Acoust. Soc. Am. 67(3), 971-995.
//
// Simplified: glottal pulse train (+ aspiration noise) -> optional nasal notch ->
// cascade of five formant resonators, plus a parallel frication noise branch.
// Pure TypeScript with no Web Audio dependency, so it runs inside an AudioWorklet and in
// plain Node for tests/offline rendering.
//
// Control model: note-driven. noteOn() queues onset consonants then a held vowel; the vowel
// sustains until noteOff(), which queues the coda consonants and a fade-out. Legato
// noteOn() skips the onset and glides into the new vowel.

import { buildSyllable, TAIL_SEGMENT } from './phonemes';
import {
  DEFAULT_TARGETS,
  TARGET_KEYS,
  type Segment,
  type SyllablePlan,
  type Targets,
} from './types';

/** Parameters are updated every BLOCK samples (control rate = fs / BLOCK). */
const BLOCK = 16;

const VOICE_GAIN = 0.08;
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

export type VoiceParamName =
  | 'formantShift'
  | 'consonantScale'
  | 'portamento'
  | 'vibratoDepth'
  | 'vibratoRate'
  | 'volume';

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
  private seed = 0x1234abcd;
  private readonly r1 = new Resonator();
  private readonly r2 = new Resonator();
  private readonly r3 = new Resonator();
  private readonly r4 = new Resonator();
  private readonly r5 = new Resonator();
  private readonly rf = new Resonator();
  private readonly notch = new Notch();

  constructor(sampleRate: number) {
    this.fs = sampleRate;
    this.cur.av = this.cur.ah = this.cur.af = this.cur.nz = 0;
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
    const segs = buildSyllable(plan, this.consonantScale, isLegato);
    this.velocity = Math.min(1, Math.max(0, velocity));
    this.tiltA = 0.35 + 0.4 * this.velocity; // louder = brighter
    this.pendingCoda = segs.coda;
    this.codaQueued = false;
    this.noteHeld = true;
    this.releaseRequested = false;
    this.setF0(f0);

    const first = [...segs.onset, ...segs.nucleus];
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
    for (const r of [this.r1, this.r2, this.r3, this.r4, this.r5, this.rf]) r.reset();
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
    this.drift = this.drift * 0.995 + this.rand() * 0.0004; // zero-mean: rand() is in [-1, 1)
    this.f0 = Math.exp(this.logF0) * Math.pow(2, cents / 1200) * (1 + this.drift);

    // Filter coefficients from the current parameters.
    const c = this.cur;
    const shift = this.formantShift;
    this.r1.set(c.f1 * shift, c.b1, this.fs);
    this.r2.set(c.f2 * shift, c.b2, this.fs);
    this.r3.set(c.f3 * shift, c.b3, this.fs);
    this.r4.set(3300 * shift, 250, this.fs);
    this.r5.set(3750 * shift, 200, this.fs);
    this.rf.set(c.ff, c.bf, this.fs);
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
      const norm = (this.fs / this.f0) * 0.29;
      const gain = (0.4 + 0.6 * this.velocity) * this.volume;
      const useNasal = c.nz > 0.001;
      for (let j = 0; j < m; j++) {
        // Rosenberg-style glottal flow, differentiated (lip radiation), then tilt filter.
        this.phase += inc;
        if (this.phase >= 1) this.phase -= 1;
        const p = this.phase;
        const g =
          p < 0.45
            ? 0.5 * (1 - Math.cos((Math.PI * p) / 0.45))
            : p < 0.61
              ? Math.cos((Math.PI / 2) * ((p - 0.45) / 0.16))
              : 0;
        const dg = (g - this.gPrev) * norm;
        this.gPrev = g;
        this.tiltY += this.tiltA * (dg - this.tiltY);

        const noise = this.rand();
        let src = c.av * this.tiltY + c.ah * noise * 0.35;
        if (useNasal) src = src * (1 - c.nz) + this.notch.process(src) * c.nz;
        let y = this.r1.process(src);
        y = this.r2.process(y);
        y = this.r3.process(y);
        y = this.r4.process(y);
        y = this.r5.process(y);
        const fr = this.rf.process(noise * c.af);
        out[b + j] = Math.tanh((y * VOICE_GAIN + fr * FRIC_GAIN) * gain);
      }
    }
  }
}
