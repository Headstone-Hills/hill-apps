// Plays a rendered UTAU buffer with transport controls (play from a position, pause, seek, loop). Separate
// from the formant engine's AudioWorklet: a UTAU render is a finished buffer, so a buffer source is all it needs.

export class UtauPlayer {
  private ctx: AudioContext | null = null;
  private source: AudioBufferSourceNode | null = null;
  private buffer: AudioBuffer | null = null;
  private startedAtCtx = 0;
  private startedAtSec = 0;
  private raf = 0;
  private active = false;
  loop = false;

  constructor(
    /** Called about every frame while playing, with the position in buffer seconds. */
    private readonly onPosition: (sec: number) => void,
    private readonly onEnded: () => void,
  ) {}

  get isPlaying(): boolean {
    return this.active;
  }
  get durationSec(): number {
    return this.buffer?.duration ?? 0;
  }

  /** Replace the audio (stops playback). */
  setAudio(samples: Float32Array, sampleRate: number): void {
    this.stop();
    this.ctx ??= new AudioContext();
    const buf = this.ctx.createBuffer(1, samples.length, sampleRate);
    buf.getChannelData(0).set(samples);
    this.buffer = buf;
  }

  async play(fromSec: number): Promise<void> {
    const ctx = this.ctx;
    const buf = this.buffer;
    if (!ctx || !buf) return;
    if (ctx.state !== 'running') await ctx.resume();
    this.halt();
    const src = ctx.createBufferSource();
    const gain = ctx.createGain();
    gain.gain.value = 0.7;
    src.buffer = buf;
    src.loop = this.loop;
    src.connect(gain).connect(ctx.destination);
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
