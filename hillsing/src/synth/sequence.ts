// Sample-accurate sequence player that drives a KlattVoice inside the AudioWorklet.
// Pure TypeScript (no Web Audio), so it also renders offline in Node tests and, later, WAV export.

import type { KlattVoice } from './klatt';
import type { SeqCommand } from './types';

export class SequenceRunner {
  private cmds: Array<{ sample: number; cmd: SeqCommand }> = [];
  private idx = 0;
  private pos = 0; // samples
  private lengthSamples = 0;
  private playing = false;
  loop = false;
  /** Called (from render) when playback reaches the end and does not loop. */
  onEnded: (() => void) | null = null;

  constructor(
    private readonly voice: KlattVoice,
    private readonly fs: number,
  ) {}

  get isPlaying(): boolean {
    return this.playing;
  }
  get positionSec(): number {
    return this.pos / this.fs;
  }

  /** Replace the sequence. Commands must be sorted by time. */
  load(cmds: SeqCommand[], lengthSec: number, startSec = 0): void {
    this.cmds = cmds.map((cmd) => ({ sample: Math.round(cmd.t * this.fs), cmd }));
    this.lengthSamples = Math.round(lengthSec * this.fs);
    this.seek(startSec);
  }

  seek(sec: number): void {
    this.pos = Math.max(0, Math.min(this.lengthSamples, Math.round(sec * this.fs)));
    let i = 0;
    while (i < this.cmds.length && this.cmds[i].sample < this.pos) i++;
    this.idx = i;
    if (this.playing) this.voice.noteOff();
  }

  play(): void {
    if (this.pos >= this.lengthSamples) this.seek(0);
    this.playing = true;
  }

  pause(): void {
    if (!this.playing) return;
    this.playing = false;
    this.voice.noteOff();
  }

  stop(): void {
    this.playing = false;
    this.voice.noteOff();
    this.seek(0);
  }

  private apply(cmd: SeqCommand): void {
    if (cmd.kind === 'on') this.voice.noteOn(cmd.plan, cmd.f0, cmd.velocity, cmd.legato);
    else this.voice.noteOff();
  }

  render(out: Float32Array): void {
    let off = 0;
    const n = out.length;
    while (off < n) {
      if (!this.playing) {
        this.voice.render(out.subarray(off));
        return;
      }
      // Apply everything due at the current sample.
      while (this.idx < this.cmds.length && this.cmds[this.idx].sample <= this.pos) {
        this.apply(this.cmds[this.idx].cmd);
        this.idx++;
      }
      if (this.pos >= this.lengthSamples) {
        if (this.loop && this.lengthSamples > 0) {
          this.voice.noteOff();
          this.seek(0);
          continue;
        }
        this.playing = false;
        this.voice.noteOff();
        this.onEnded?.();
        continue;
      }
      const nextCmd = this.idx < this.cmds.length ? this.cmds[this.idx].sample : Infinity;
      const boundary = Math.min(nextCmd, this.lengthSamples);
      const chunk = Math.min(n - off, boundary - this.pos);
      this.voice.render(out.subarray(off, off + chunk));
      off += chunk;
      this.pos += chunk;
    }
  }
}
