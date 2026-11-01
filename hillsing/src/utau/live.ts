// Live key-playing with a UTAU voicebank. Implements the same VoiceSink the formant engine does, so the
// performance layer (keys, MIDI, lyric pointer, legato) drives it unchanged.
//
// Press: the attack (consonants + start of the vowel) plays once, then a seamless loop of the steady vowel
// takes over and repeats until release. Release: the loop fades out under the syllable's ending samples
// ("E d", or "E -" for a plain vowel). Pitch bend / a held-key glide re-tunes the playing sources.

import type { VoiceSink } from '../engine/performance';
import type { SyllablePlan } from '../synth/types';
import { alignOffset, buildAttack, buildRelease } from './livenote';
import { xsampaVowel } from './planner';
import { centsToRatio, genderToFormant, type Tuning } from './tuning';
import type { Voicebank } from './voicebank';

interface Playing {
  gain: GainNode;
  sources: AudioBufferSourceNode[];
  plan: SyllablePlan;
  f0: number;
  /** Loudness of the sustain, for matching the release to it. */
  loopRms: number;
  /** Vibrato oscillator, stopped with the note. */
  lfo?: OscillatorNode;
  /** The sustain being looped, where it started (context time) and from which offset, to line the release up with it. */
  sustain?: { data: Float32Array; sampleRate: number; startAt: number; offsetSec: number };
  /** The earliest time the note may start to end: the consonant and the start of the vowel always play out. */
  minReleaseAt: number;
}

const START_DELAY_SEC = 0.0015;
/** The bank's samples are loud: at 0.7 a held vowel peaked at 0.97, too close to clipping. */
const BASE_LEVEL = 0.4;
/** Live notes play consonants this much faster than recorded so the vowel arrives sooner; the tuning's speed scales it. */
const LIVE_BASE_SPEED = 1.5;
/** Shortest sustained vowel after the attack, so even a quick tap is heard as a vowel. */
const MIN_SUSTAIN_SEC = 0.1;

export class UtauLiveVoice implements VoiceSink {
  private readonly master: GainNode;
  private playing: Playing | null = null;
  /** The note whose release is scheduled but has not finished, and its tail sound (cut short if a new note starts). */
  private releasing: Playing | null = null;
  private tail: Playing | null = null;
  private gateOpen = false;
  /** Bumped by every key press and by stop(), so a note that is still being built can tell it was superseded. */
  private token = 0;

  constructor(
    private readonly ctx: AudioContext,
    out: AudioNode,
    private readonly bank: () => Voicebank | null,
    private readonly tuning: () => Tuning,
  ) {
    this.master = ctx.createGain();
    this.master.gain.value = BASE_LEVEL * this.tuning().volume;
    this.master.connect(out);
  }

  /** The tuning's volume changed: apply it to what is playing now too. */
  refreshVolume(): void {
    this.master.gain.setTargetAtTime(BASE_LEVEL * this.tuning().volume, this.ctx.currentTime, 0.02);
  }

  noteOn(plan: SyllablePlan, f0: number, velocity: number, legato: boolean): void {
    void this.start(plan, f0, velocity, legato);
  }

  noteOff(): void {
    this.gateOpen = false;
    void this.release();
  }

  setF0(f0: number): void {
    const p = this.playing;
    if (!p) return;
    const t = this.tuning();
    const cents = 1200 * Math.log2((f0 * centsToRatio(t.tune)) / p.f0);
    const tc = Math.max(0.003, t.glide / 3000); // a time constant: the slide is about 95 % done after `glide` ms
    for (const s of p.sources) s.detune.setTargetAtTime(cents, this.ctx.currentTime, tc);
  }

  /** Silence everything now (voice switched, panic). */
  stop(): void {
    this.token++;
    this.gateOpen = false;
    for (const p of [this.playing, this.releasing, this.tail]) if (p) this.fadeOut(p, 0.03);
    this.playing = this.releasing = this.tail = null;
  }

  private async start(plan: SyllablePlan, f0: number, velocity: number, legato: boolean): Promise<void> {
    const bank = this.bank();
    if (!bank) return;
    const mine = ++this.token;
    this.gateOpen = true;
    const prev = this.playing;
    const prevVowel = legato && prev && prev.plan.coda.length === 0 ? xsampaVowel(prev.plan.nucleus) : null;
    const t = this.tuning();
    f0 *= centsToRatio(t.tune);
    const shaping = { formant: genderToFormant(t.gender), fixedGain: t.consonant, fixedSoftness: t.tone };
    const attack = await buildAttack(bank, plan, f0, prevVowel, { consonantSpeed: Math.max(1, LIVE_BASE_SPEED * t.velocity), shaping });
    if (!attack || mine !== this.token) return; // nothing to sing, or a newer key press took over
    if (this.ctx.state !== 'running') await this.ctx.resume();
    if (mine !== this.token) return;

    if (prev) this.fadeOut(prev, legato ? 0.06 : 0.03);
    // A note still ending (a held-over release or its tail) gives way to the new one.
    for (const p of [this.releasing, this.tail]) if (p) this.fadeOut(p, 0.03);
    this.releasing = this.tail = null;
    const t0 = this.ctx.currentTime + START_DELAY_SEC;
    const note = this.ctx.createGain();
    note.gain.value = 0.4 + 0.6 * Math.min(1, Math.max(0, velocity));
    note.connect(this.master);
    const sources: AudioBufferSourceNode[] = [];

    const attackBuf = this.buffer(attack.samples, attack.sampleRate);
    const attackGain = this.ctx.createGain();
    attackGain.connect(note);
    const a = this.ctx.createBufferSource();
    a.buffer = attackBuf;
    a.connect(attackGain);
    a.start(t0);
    sources.push(a);

    if (attack.loop) {
      const loopAt = t0 + attackBuf.duration - attack.crossfade;
      const loopGain = this.ctx.createGain();
      loopGain.gain.setValueAtTime(0, t0);
      loopGain.gain.setValueAtTime(0, loopAt);
      loopGain.gain.linearRampToValueAtTime(1, loopAt + attack.crossfade);
      loopGain.connect(note);
      attackGain.gain.setValueAtTime(1, loopAt);
      attackGain.gain.linearRampToValueAtTime(0, loopAt + attack.crossfade);
      const l = this.ctx.createBufferSource();
      l.buffer = this.buffer(attack.loop, attack.sampleRate);
      l.loop = true;
      l.connect(loopGain);
      l.start(loopAt, attack.loopOffset);
      sources.push(l);
    }
    // A tap still sings a syllable: the consonant, the start of the vowel and a short stretch of sustained vowel.
    const minReleaseAt = t0 + Math.max(0, attackBuf.duration - attack.crossfade) + (attack.hasLoop ? MIN_SUSTAIN_SEC : 0);
    let sustain: Playing['sustain'];
    if (attack.loop) sustain = { data: attack.loop, sampleRate: attack.sampleRate, startAt: t0 + attackBuf.duration - attack.crossfade, offsetSec: attack.loopOffset };
    let lfo: OscillatorNode | undefined;
    if (t.vibratoDepth > 0) {
      // Vibrato: a slow sine added to every source's pitch (in cents), fading in after the note has started.
      lfo = this.ctx.createOscillator();
      lfo.frequency.value = t.vibratoRate;
      const depth = this.ctx.createGain();
      depth.gain.setValueAtTime(0, t0);
      depth.gain.setValueAtTime(0, t0 + 0.15);
      depth.gain.linearRampToValueAtTime(t.vibratoDepth, t0 + 0.45);
      lfo.connect(depth);
      for (const s of sources) depth.connect(s.detune);
      lfo.start(t0);
    }
    this.playing = { gain: note, sources, plan, f0, loopRms: attack.loopRms, minReleaseAt, lfo, sustain };
    if (!this.gateOpen) void this.release(); // the key came up while the note was being built
  }

  private async release(): Promise<void> {
    const p = this.playing;
    const bank = this.bank();
    if (!p || !bank) return;
    const mine = this.token;
    const tu = this.tuning();
    const tail = await buildRelease(bank, p.plan, p.f0, p.loopRms, { formant: genderToFormant(tu.gender), fixedGain: tu.consonant, fixedSoftness: tu.tone });
    if (mine !== this.token || this.playing !== p) return; // a new note (or stop) already took over
    // Never end before the attack has played out: a quick tap still sings the consonant and the start of the vowel.
    const t = Math.max(this.ctx.currentTime + START_DELAY_SEC, p.minReleaseAt);
    this.playing = null;
    this.releasing = p;
    if (tail) {
      // The ending sample fades in while the sustain fades out, so the level carries straight through.
      const noteLevel = p.gain.gain.value;
      const out = this.ctx.createGain();
      out.gain.setValueAtTime(0, t);
      out.gain.linearRampToValueAtTime(noteLevel, t + tail.overlap);
      out.connect(this.master);
      const s = this.ctx.createBufferSource();
      s.buffer = this.buffer(tail.samples, tail.sampleRate);
      s.connect(out);
      s.start(t, this.releaseStart(p, tail.samples, tail.sampleRate, tail.overlap, t));
      this.tail = { gain: out, sources: [s], plan: p.plan, f0: p.f0, loopRms: p.loopRms, minReleaseAt: t };
      this.fadeOut(p, tail.overlap, t);
    } else {
      this.fadeOut(p, 0.08, t);
    }
  }

  /**
   * How far into the ending sample to start so its waveform lines up with the sustain it takes over from at time `t`.
   * Two crossfading waveforms that are out of step cancel, and the level dips and comes back (heard as a stutter).
   */
  private releaseStart(p: Playing, tail: Float32Array, tailRate: number, overlap: number, t: number): number {
    const su = p.sustain;
    if (!su || su.sampleRate !== tailRate || t < su.startAt + 0.02) return 0;
    const len = su.data.length;
    const at = Math.round(((su.offsetSec + (t - su.startAt)) * su.sampleRate) % len);
    const n = Math.min(Math.round(Math.min(overlap, 0.03) * tailRate), tail.length >> 1);
    const win = new Float32Array(n);
    for (let i = 0; i < n; i++) win[i] = su.data[(at + i) % len];
    return alignOffset(win, tail, Math.ceil((1.25 * tailRate) / p.f0)) / tailRate;
  }

  private fadeOut(p: Playing, seconds: number, at = this.ctx.currentTime): void {
    p.gain.gain.cancelScheduledValues(at);
    p.gain.gain.setValueAtTime(p.gain.gain.value, at);
    p.gain.gain.linearRampToValueAtTime(0, at + seconds);
    for (const s of p.sources) {
      try {
        s.stop(at + seconds + 0.02);
      } catch {
        /* already stopped */
      }
    }
    try {
      p.lfo?.stop(at + seconds + 0.02);
    } catch {
      /* already stopped */
    }
  }

  private buffer(samples: Float32Array, sampleRate: number): AudioBuffer {
    const b = this.ctx.createBuffer(1, samples.length, sampleRate);
    b.getChannelData(0).set(samples);
    return b;
  }
}
