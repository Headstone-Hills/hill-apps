// The one audio path for every voice: sources (formant synth, live samples, sample melodies, Voice lab) plug into
// `input`; the effects rack processes the mix; an analyser (the meter and spectrogram read it) feeds the speakers.
// One AudioContext for the whole page, created on the first click.

import { EffectsRack } from './effects';
import { FxModel } from './fxparams';

function pageStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** The effect settings the panel edits. Loaded from, and saved to, the browser's storage. */
export const fxModel = new FxModel(pageStorage());

export class AudioBus {
  private constructor(
    readonly ctx: AudioContext,
    /** Where every source connects. */
    readonly input: GainNode,
    readonly analyser: AnalyserNode,
    readonly fx: EffectsRack,
  ) {}

  /** Must be called from a user gesture so the context may start. */
  static async create(): Promise<AudioBus> {
    const ctx = new AudioContext({ latencyHint: 'interactive' });
    const fx = await EffectsRack.create(ctx);
    const input = ctx.createGain();
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 2048;
    analyser.smoothingTimeConstant = 0.15; // keep bursts visible in the spectrogram
    analyser.minDecibels = -100;
    analyser.maxDecibels = -20;
    input.connect(fx.input);
    fx.output.connect(analyser);
    analyser.connect(ctx.destination);
    fx.apply(fxModel.state);
    fxModel.onChange(() => fx.apply(fxModel.state));
    if (ctx.state !== 'running') await ctx.resume();
    return new AudioBus(ctx, input, analyser, fx);
  }
}

let shared: Promise<AudioBus> | null = null;

/** The page's single audio bus (created on first use; a failed creation is retried next time). */
export function getBus(): Promise<AudioBus> {
  shared ??= AudioBus.create().catch((err) => {
    shared = null;
    throw err;
  });
  return shared;
}
