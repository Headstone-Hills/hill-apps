// Live key-playing with a UTAU voicebank. Implements the same VoiceSink the formant engine does, so the
// performance layer (keys, MIDI, lyric pointer, legato) drives it unchanged.
//
// Press: the attack (consonants + start of the vowel) plays once, then a seamless loop of the steady vowel
// takes over and repeats until release. Release: the loop fades out under the syllable's ending samples
// ("E d", or "E -" for a plain vowel). Pitch bend / a held-key glide re-tunes the playing sources.

import type { VoiceSink } from '../engine/performance';
import type { SyllablePlan } from '../synth/types';
import { buildAttack, buildRelease } from './livenote';
import { xsampaVowel } from './planner';
import type { Voicebank } from './voicebank';

interface Playing {
  gain: GainNode;
  sources: AudioBufferSourceNode[];
  plan: SyllablePlan;
  f0: number;
}

const START_DELAY_SEC = 0.004;

export class UtauLiveVoice implements VoiceSink {
  private readonly master: GainNode;
  private playing: Playing | null = null;
  private gateOpen = false;
  /** Bumped by every key press and by stop(), so a note that is still being built can tell it was superseded. */
  private token = 0;

  constructor(
    private readonly ctx: AudioContext,
    out: AudioNode,
    private readonly bank: () => Voicebank | null,
  ) {
    this.master = ctx.createGain();
    this.master.gain.value = 0.4; // the bank's samples are loud: at 0.7 a held vowel peaked at 0.97, too close to clipping
    this.master.connect(out);
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
    const cents = 1200 * Math.log2(f0 / p.f0);
    for (const s of p.sources) s.detune.setTargetAtTime(cents, this.ctx.currentTime, 0.01);
  }

  /** Silence everything now (voice switched, panic). */
  stop(): void {
    this.token++;
    this.gateOpen = false;
    if (this.playing) this.fadeOut(this.playing, 0.03);
    this.playing = null;
  }

  private async start(plan: SyllablePlan, f0: number, velocity: number, legato: boolean): Promise<void> {
    const bank = this.bank();
    if (!bank) return;
    const mine = ++this.token;
    this.gateOpen = true;
    const prev = this.playing;
    const prevVowel = legato && prev && prev.plan.coda.length === 0 ? xsampaVowel(prev.plan.nucleus) : null;
    const attack = await buildAttack(bank, plan, f0, prevVowel);
    if (!attack || mine !== this.token) return; // nothing to sing, or a newer key press took over
    if (this.ctx.state !== 'running') await this.ctx.resume();
    if (mine !== this.token) return;

    if (prev) this.fadeOut(prev, legato ? 0.06 : 0.03);
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
      l.start(loopAt);
      sources.push(l);
    }
    this.playing = { gain: note, sources, plan, f0 };
    if (!this.gateOpen) void this.release(); // the key came up while the note was being built
  }

  private async release(): Promise<void> {
    const p = this.playing;
    const bank = this.bank();
    if (!p || !bank) return;
    const mine = this.token;
    const tail = await buildRelease(bank, p.plan, p.f0);
    if (mine !== this.token || this.playing !== p) return; // a new note (or stop) already took over
    const t = this.ctx.currentTime + START_DELAY_SEC;
    if (tail) {
      const out = this.ctx.createGain();
      out.gain.value = p.gain.gain.value;
      out.connect(this.master);
      const s = this.ctx.createBufferSource();
      s.buffer = this.buffer(tail.samples, tail.sampleRate);
      s.connect(out);
      s.start(t);
      this.fadeOut(p, tail.overlap, t);
    } else {
      this.fadeOut(p, 0.08, t);
    }
    this.playing = null;
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
  }

  private buffer(samples: Float32Array, sampleRate: number): AudioBuffer {
    const b = this.ctx.createBuffer(1, samples.length, sampleRate);
    b.getChannelData(0).set(samples);
    return b;
  }
}
