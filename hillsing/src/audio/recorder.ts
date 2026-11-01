// Records what HillSing plays (after the effects and the output limiter, so it is what you hear), as 16-bit stereo, for
// as long as you like up to a limit. It uses a ScriptProcessorNode rather than an AudioWorklet so it also works where
// worklets are blocked (a viewer's sandbox). Export is in wav.ts and mp3.ts.

import { floatToInt16 } from './wav';

/** Stop by themselves after this long: 16-bit stereo at 44.1 kHz is about 10 MB a minute. */
export const MAX_RECORD_SEC = 20 * 60;

export interface Recording {
  left: Int16Array[];
  right: Int16Array[];
  sampleRate: number;
  frames: number;
}

export class Recorder {
  private node: ScriptProcessorNode | null = null;
  private mute: GainNode | null = null;
  private inputs: AudioNode[] = [];
  private left: Int16Array[] = [];
  private right: Int16Array[] = [];
  private frames = 0;
  private sampleRate = 44100;
  /** Called (on the audio thread's schedule) when the length limit is reached. */
  onLimit: (() => void) | null = null;

  get recording(): boolean {
    return this.node !== null;
  }

  get seconds(): number {
    return this.frames / this.sampleRate;
  }

  /** Start capturing `sources` (they are summed). */
  start(ctx: BaseAudioContext, sources: AudioNode[]): void {
    this.stop();
    this.left = [];
    this.right = [];
    this.frames = 0;
    this.sampleRate = ctx.sampleRate;
    const node = ctx.createScriptProcessor(4096, 2, 2);
    const mute = ctx.createGain();
    mute.gain.value = 0; // the processor only runs while connected onward; this keeps it silent
    node.onaudioprocess = (e) => {
      const l = e.inputBuffer.getChannelData(0);
      const r = e.inputBuffer.numberOfChannels > 1 ? e.inputBuffer.getChannelData(1) : l;
      this.left.push(floatToInt16(l));
      this.right.push(floatToInt16(r));
      this.frames += l.length;
      if (this.seconds >= MAX_RECORD_SEC) {
        const cb = this.onLimit;
        queueMicrotask(() => cb?.());
      }
    };
    for (const s of sources) s.connect(node);
    node.connect(mute).connect(ctx.destination);
    this.node = node;
    this.mute = mute;
    this.inputs = sources;
  }

  /** Stop and hand back what was captured (empty if nothing was running). */
  stop(): Recording | null {
    const node = this.node;
    if (!node) return null;
    node.onaudioprocess = null;
    for (const s of this.inputs) {
      try {
        s.disconnect(node);
      } catch {
        /* already disconnected */
      }
    }
    node.disconnect();
    this.mute?.disconnect();
    this.node = null;
    this.mute = null;
    this.inputs = [];
    return { left: this.left, right: this.right, sampleRate: this.sampleRate, frames: this.frames };
  }
}
