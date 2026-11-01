// Plays a rendered UTAU buffer with transport controls (play from a position, pause, seek, loop). A UTAU render is
// a finished buffer, so a buffer source is all it needs; it plays through the page's audio bus (effects included).

import { getBus } from '../audio/bus';

export class UtauPlayer {
  private ctx: AudioContext | null = null;
  private source: AudioBufferSourceNode | null = null;
  private buffer: AudioBuffer | null = null;
  private audio: { samples: Float32Array; sampleRate: number } | null = null;
  private startedAtCtx = 0;
  private startedAtSec = 0;
  private raf = 0;
  private active = false;
  loop = false;
  /** Volume from the tuning panel, applied when playback starts. */
  level = 1;

  constructor(
    /** Called about every frame while playing, with the position in buffer seconds. */
    private readonly onPosition: (sec: number) => void,
    private readonly onEnded: () => void,
  ) {}

  get isPlaying(): boolean {
    return this.active;
  }
  get durationSec(): number {
    return this.audio ? this.audio.samples.length / this.audio.sampleRate : 0;
  }

  /** Replace the audio (stops playback). */
  setAudio(samples: Float32Array, sampleRate: number): void {
    this.stop();
    this.audio = { samples, sampleRate };
    this.buffer = null; // built on the next play, on the bus's context
  }

  async play(fromSec: number): Promise<void> {
    if (!this.audio) return;
    const bus = await getBus();
    const ctx = bus.ctx;
    this.ctx = ctx;
    if (!this.buffer || this.buffer.length !== this.audio.samples.length) {
      this.buffer = ctx.createBuffer(1, this.audio.samples.length, this.audio.sampleRate);
      this.buffer.getChannelData(0).set(this.audio.samples);
    }
    const buf = this.buffer;
    if (ctx.state !== 'running') await ctx.resume();
    this.halt();
    const src = ctx.createBufferSource();
    const gain = ctx.createGain();
    gain.gain.value = 0.7 * this.level;
    src.buffer = buf;
    src.loop = this.loop;
    src.connect(gain).connect(bus.input);
    const from = Math.max(0, Math.min(buf.duration - 0.01, fromSec));
    src.onended = () => {
      if (this.source === src && this.active) {
        this.active = false;
        cancelAnimationFrame(this.raf);
        this.onEnded();
      }
    };
    src.start(0, from);
    this.source = src;
    this.startedAtCtx = ctx.currentTime;
    this.startedAtSec = from;
    this.active = true;
    const tick = (): void => {
      if (!this.active) return;
      this.onPosition(this.position());
      this.raf = requestAnimationFrame(tick);
    };
    this.raf = requestAnimationFrame(tick);
  }

  /** Current position in buffer seconds. */
  position(): number {
    if (!this.ctx || !this.buffer) return 0;
    if (!this.active) return this.startedAtSec;
    const t = this.startedAtSec + (this.ctx.currentTime - this.startedAtCtx);
    return this.loop ? t % this.buffer.duration : Math.min(t, this.buffer.duration);
  }

  /** Stop sounding and remember where we were (for resuming). */
  pause(): number {
    const at = this.position();
    this.halt();
    this.startedAtSec = at;
    return at;
  }

  stop(): void {
    this.halt();
    this.startedAtSec = 0;
  }

  private halt(): void {
    this.active = false;
    cancelAnimationFrame(this.raf);
    if (this.source) {
      this.source.onended = null;
      try {
        this.source.stop();
      } catch {
        /* already stopped */
      }
      this.source = null;
    }
  }
}
