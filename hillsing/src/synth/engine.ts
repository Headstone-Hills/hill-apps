import type { VoiceSink } from '../engine/performance';
import type { VoiceParamName } from './klatt';
import type { SeqCommand, SyllablePlan } from './types';
import type { WorkletEvent, WorkletMessage } from './worklet';
import workletUrl from './worklet.ts?worker&url';

/** Main-thread handle to the synth running in an AudioWorklet. */
export class SingerEngine implements VoiceSink {
  /** Sequence position updates from the worklet (engine seconds). */
  onSeqPosition: ((sec: number, playing: boolean) => void) | null = null;
  onSeqEnded: (() => void) | null = null;

  private constructor(
    readonly context: AudioContext,
    private readonly node: AudioWorkletNode,
    private readonly analyser: AnalyserNode,
  ) {
    node.port.onmessage = (e: MessageEvent<WorkletEvent>) => {
      const m = e.data;
      if (m.type === 'seqPos') this.onSeqPosition?.(m.sec, m.playing);
      else if (m.type === 'seqEnded') this.onSeqEnded?.();
    };
  }

  /** Must be called from a user gesture so the AudioContext may start. */
  static async create(): Promise<SingerEngine> {
    const context = new AudioContext({ latencyHint: 'interactive' });
    await context.audioWorklet.addModule(workletUrl);
    const node = new AudioWorkletNode(context, 'klatt-voice', {
      numberOfInputs: 0,
      outputChannelCount: [1],
    });
    const analyser = context.createAnalyser();
    analyser.fftSize = 1024;
    node.connect(analyser);
    analyser.connect(context.destination);
    if (context.state !== 'running') await context.resume();
    return new SingerEngine(context, node, analyser);
  }

  noteOn(plan: SyllablePlan, f0: number, velocity: number, legato: boolean): void {
    this.post({ type: 'noteOn', plan, f0, velocity, legato });
  }
  noteOff(): void {
    this.post({ type: 'noteOff' });
  }
  setF0(f0: number): void {
    this.post({ type: 'f0', f0 });
  }
  setParam(name: VoiceParamName, value: number): void {
    this.post({ type: 'param', name, value });
  }
  stop(): void {
    this.post({ type: 'stop' });
  }

  // ---- Sequence (MIDI file) playback, scheduled sample-accurately in the worklet ----
  seqLoad(cmds: SeqCommand[], lengthSec: number, startSec: number): void {
    this.post({ type: 'seqLoad', cmds, lengthSec, startSec });
  }
  seqPlay(): void {
    this.post({ type: 'seqPlay' });
  }
  seqPause(): void {
    this.post({ type: 'seqPause' });
  }
  seqStop(): void {
    this.post({ type: 'seqStop' });
  }
  seqSeek(sec: number): void {
    this.post({ type: 'seqSeek', sec });
  }
  seqLoop(loop: boolean): void {
    this.post({ type: 'seqLoop', loop });
  }

  /** Peak absolute sample of the most recent ~1024 output samples (0..1). */
  peakLevel(): number {
    const buf = new Float32Array(this.analyser.fftSize);
    this.analyser.getFloatTimeDomainData(buf);
    let peak = 0;
    for (const x of buf) peak = Math.max(peak, Math.abs(x));
    return peak;
  }

  /** Output latency estimate in ms (base + hardware output latency). */
  latencyMs(): number {
    const ctx = this.context;
    return (ctx.baseLatency + (ctx.outputLatency || 0)) * 1000;
  }

  private post(m: WorkletMessage): void {
    this.node.port.postMessage(m);
  }
}
