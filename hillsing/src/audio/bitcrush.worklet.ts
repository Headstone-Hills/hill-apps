// AudioWorklet for the bitcrush effect: bit-depth and sample-rate reduction (the maths is crushBlock in fxparams.ts,
// shared with the tests). Bundled by Vite via `?worker&url`.
import { crushBlock } from './fxparams';

declare class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor();
}
declare function registerProcessor(name: string, ctor: (new () => AudioWorkletProcessor) & { parameterDescriptors?: unknown }): void;

class BitcrushProcessor extends AudioWorkletProcessor {
  static get parameterDescriptors() {
    return [
      { name: 'bits', defaultValue: 8, minValue: 1, maxValue: 16, automationRate: 'k-rate' },
      { name: 'rate', defaultValue: 4, minValue: 1, maxValue: 64, automationRate: 'k-rate' },
    ];
  }

  private readonly state: Array<{ phase: number; held: number }> = [];

  process(inputs: Float32Array[][], outputs: Float32Array[][], parameters: Record<string, Float32Array>): boolean {
    const input = inputs[0];
    const output = outputs[0];
    if (!input || input.length === 0) return true;
    const bits = parameters.bits[0];
    const rate = parameters.rate[0];
    for (let c = 0; c < output.length; c++) {
      output[c].set(input[Math.min(c, input.length - 1)]);
      const st = (this.state[c] ??= { phase: 0, held: 0 });
      crushBlock(output[c], bits, rate, st);
    }
    return true;
  }
}

registerProcessor('hs-bitcrush', BitcrushProcessor as unknown as (new () => AudioWorkletProcessor) & { parameterDescriptors?: unknown });
