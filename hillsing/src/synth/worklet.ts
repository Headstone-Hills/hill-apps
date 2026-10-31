// AudioWorklet entry point: wraps KlattVoice. Bundled by Vite via `?worker&url`.
import { KlattVoice, type VoiceParamName } from './klatt';
import { SequenceRunner } from './sequence';
import { getVoice } from './voices';
import type { SeqCommand, SyllablePlan } from './types';

// Minimal typings for the AudioWorkletGlobalScope (not part of lib.dom).
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor();
}
declare function registerProcessor(name: string, ctor: new () => AudioWorkletProcessor): void;
declare const sampleRate: number;

export type WorkletMessage =
  | { type: 'noteOn'; plan: SyllablePlan; f0: number; velocity: number; legato: boolean }
  | { type: 'noteOff' }
  | { type: 'f0'; f0: number }
  | { type: 'param'; name: VoiceParamName; value: number }
  | { type: 'voice'; id: string }
  | { type: 'stop' }
  | { type: 'seqLoad'; cmds: SeqCommand[]; lengthSec: number; startSec: number }
  | { type: 'seqPlay' }
  | { type: 'seqPause' }
  | { type: 'seqStop' }
  | { type: 'seqSeek'; sec: number }
  | { type: 'seqLoop'; loop: boolean };

/** Messages the worklet posts back to the main thread. */
export type WorkletEvent =
  | { type: 'seqPos'; sec: number; playing: boolean }
  | { type: 'seqEnded' };

/** Report the sequence position about every 2048 samples (~46 ms at 44.1 kHz). */
const POS_REPORT_SAMPLES = 2048;

class KlattProcessor extends AudioWorkletProcessor {
  private readonly voice = new KlattVoice(sampleRate);
  private readonly seq = new SequenceRunner(this.voice, sampleRate);
  private sinceReport = 0;

  constructor() {
    super();
    this.seq.onEnded = () => this.port.postMessage({ type: 'seqEnded' } satisfies WorkletEvent);
    this.port.onmessage = (e: MessageEvent<WorkletMessage>) => {
      const m = e.data;
      switch (m.type) {
        case 'noteOn':
          this.voice.noteOn(m.plan, m.f0, m.velocity, m.legato);
          break;
        case 'noteOff':
          this.voice.noteOff();
          break;
        case 'f0':
          this.voice.setF0(m.f0);
          break;
        case 'param':
          this.voice.setParam(m.name, m.value);
          break;
        case 'voice':
          this.voice.setProfile(getVoice(m.id));
          break;
        case 'stop':
          this.voice.stop();
          break;
        case 'seqLoad':
          this.seq.load(m.cmds, m.lengthSec, m.startSec);
          this.report();
          break;
        case 'seqPlay':
          this.seq.play();
          break;
        case 'seqPause':
          this.seq.pause();
          this.report();
          break;
        case 'seqStop':
          this.seq.stop();
          this.report();
          break;
        case 'seqSeek':
          this.seq.seek(m.sec);
          this.report();
          break;
        case 'seqLoop':
          this.seq.loop = m.loop;
          break;
      }
    };
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const out = outputs[0];
    if (!out || out.length === 0) return true;
    this.seq.render(out[0]);
    for (let c = 1; c < out.length; c++) out[c].set(out[0]);
    this.sinceReport += out[0].length;
    if (this.seq.isPlaying && this.sinceReport >= POS_REPORT_SAMPLES) this.report();
    return true;
  }

  private report(): void {
    this.sinceReport = 0;
    this.port.postMessage({ type: 'seqPos', sec: this.seq.positionSec, playing: this.seq.isPlaying } satisfies WorkletEvent);
  }
}

registerProcessor('klatt-voice', KlattProcessor as unknown as new () => AudioWorkletProcessor);
