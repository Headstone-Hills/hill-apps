// AudioWorklet entry point: wraps KlattVoice. Bundled by Vite via `?worker&url`.
import { KlattVoice, type VoiceParamName } from './klatt';
import type { SyllablePlan } from './types';

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
  | { type: 'stop' };

class KlattProcessor extends AudioWorkletProcessor {
  private readonly voice = new KlattVoice(sampleRate);

  constructor() {
    super();
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
        case 'stop':
          this.voice.stop();
          break;
      }
    };
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const out = outputs[0];
    if (!out || out.length === 0) return true;
    this.voice.render(out[0]);
    for (let c = 1; c < out.length; c++) out[c].set(out[0]);
    return true;
  }
}

registerProcessor('klatt-voice', KlattProcessor as unknown as new () => AudioWorkletProcessor);
