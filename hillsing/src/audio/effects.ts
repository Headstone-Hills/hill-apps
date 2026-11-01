// The effects rack: bitcrush -> distortion -> EQ -> compressor -> delay -> reverb, in series. Every stage has
// a dry path and a wet path so an effect that is off costs nothing audible and an effect with a Mix control
// blends in the way you expect. The rack only follows an FxState; the panel edits the state (see fxparams.ts).

import { dbToGain, effectSpec, impulseResponse, softClipCurve, type EffectId, type FxState } from './fxparams';
import crushUrl from './bitcrush.worklet.ts?worker&url';

interface Stage {
  input: GainNode;
  output: GainNode;
  dry: GainNode;
  wet: GainNode;
}

const SMOOTH_SEC = 0.015;

function makeStage(ctx: BaseAudioContext): Stage {
  const input = ctx.createGain();
  const output = ctx.createGain();
  const dry = ctx.createGain();
  const wet = ctx.createGain();
  input.connect(dry).connect(output);
  wet.connect(output);
  return { input, output, dry, wet };
}

export class EffectsRack {
  readonly input: GainNode;
  readonly output: GainNode;
  /** False when the bitcrush worklet could not be loaded (the effect then passes sound through). */
  readonly bitcrushAvailable: boolean;

  private readonly stages = new Map<EffectId, Stage>();
  private readonly crush: AudioWorkletNode | null;
  private readonly pre: GainNode;
  private readonly shaper: WaveShaperNode;
  private readonly distTone: BiquadFilterNode;
  private readonly post: GainNode;
  private readonly low: BiquadFilterNode;
  private readonly mid: BiquadFilterNode;
  private readonly high: BiquadFilterNode;
  private readonly comp: DynamicsCompressorNode;
  private readonly makeup: GainNode;
  private readonly delay: DelayNode;
  private readonly feedback: GainNode;
  private readonly echoTone: BiquadFilterNode;
  private readonly convolver: ConvolverNode;
  private irKey = '';
  private irTimer = 0;

  private constructor(private readonly ctx: BaseAudioContext, crush: AudioWorkletNode | null) {
    this.crush = crush;
    this.bitcrushAvailable = crush !== null;

    // bitcrush
    const bc = makeStage(ctx);
    if (crush) bc.input.connect(crush).connect(bc.wet);
    this.stages.set('bitcrush', bc);

    // distortion: gain into a soft clipper, tone filter, output level
    const ds = makeStage(ctx);
    this.pre = ctx.createGain();
    this.shaper = ctx.createWaveShaper();
    this.shaper.curve = softClipCurve();
    this.shaper.oversample = '4x';
    this.distTone = ctx.createBiquadFilter();
    this.distTone.type = 'lowpass';
    this.post = ctx.createGain();
    ds.input.connect(this.pre).connect(this.shaper).connect(this.distTone).connect(this.post).connect(ds.wet);
    this.stages.set('distortion', ds);

    // EQ
    const eq = makeStage(ctx);
    this.low = ctx.createBiquadFilter();
    this.low.type = 'lowshelf';
    this.mid = ctx.createBiquadFilter();
    this.mid.type = 'peaking';
    this.high = ctx.createBiquadFilter();
    this.high.type = 'highshelf';
    eq.input.connect(this.low).connect(this.mid).connect(this.high).connect(eq.wet);
    this.stages.set('eq', eq);

    // compressor
    const cp = makeStage(ctx);
    this.comp = ctx.createDynamicsCompressor();
    this.makeup = ctx.createGain();
    cp.input.connect(this.comp).connect(this.makeup).connect(cp.wet);
    this.stages.set('compressor', cp);

    // delay: the echo loop runs through a tone filter, so each repeat is a little darker
    const dl = makeStage(ctx);
    this.delay = ctx.createDelay(2);
    this.echoTone = ctx.createBiquadFilter();
    this.echoTone.type = 'lowpass';
    this.feedback = ctx.createGain();
    dl.input.connect(this.delay);
    this.delay.connect(this.echoTone);
    this.echoTone.connect(dl.wet);
    this.echoTone.connect(this.feedback).connect(this.delay);
    this.stages.set('delay', dl);

    // reverb
    const rv = makeStage(ctx);
    this.convolver = ctx.createConvolver();
    rv.input.connect(this.convolver).connect(rv.wet);
    this.stages.set('reverb', rv);

    // in series
    const order: EffectId[] = ['bitcrush', 'distortion', 'eq', 'compressor', 'delay', 'reverb'];
    this.input = this.stages.get(order[0])!.input;
    this.output = this.stages.get(order[order.length - 1])!.output;
    for (let i = 0; i + 1 < order.length; i++) this.stages.get(order[i])!.output.connect(this.stages.get(order[i + 1])!.input);
    this.apply({ enabled: Object.fromEntries(order.map((id) => [id, false])) as FxState['enabled'], values: Object.fromEntries(order.map((id) => [id, Object.fromEntries(effectSpec(id).params.map((q) => [q.id, q.value]))])) as FxState['values'] });
  }

  static async create(ctx: BaseAudioContext): Promise<EffectsRack> {
    let node: AudioWorkletNode | null = null;
    try {
      await ctx.audioWorklet.addModule(crushUrl);
      node = new AudioWorkletNode(ctx, 'hs-bitcrush', { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [2] });
    } catch {
      node = null; // e.g. worklets blocked: everything but bitcrush still works
    }
    return new EffectsRack(ctx, node);
  }

  private set(param: AudioParam, value: number): void {
    param.cancelScheduledValues(this.ctx.currentTime);
    param.setTargetAtTime(value, this.ctx.currentTime, SMOOTH_SEC);
  }

  /** Make the rack follow `state`. Cheap to call on every slider move. */
  apply(state: FxState): void {
    const v = state.values;
    const on = state.enabled;
    const blend = (id: EffectId, mix: number | null): void => {
      const s = this.stages.get(id)!;
      const wet = on[id] ? (mix ?? 1) : 0;
      this.set(s.wet.gain, wet);
      // effects with a Mix control keep the dry signal under the wet one; the rest replace it
      this.set(s.dry.gain, on[id] ? (mix === null ? 0 : 1 - mix) : 1);
    };

    const b = v.bitcrush;
    if (this.crush) {
      this.crush.parameters.get('bits')?.setValueAtTime(b.bits, this.ctx.currentTime);
      this.crush.parameters.get('rate')?.setValueAtTime(b.rate, this.ctx.currentTime);
    }
    blend('bitcrush', this.crush ? b.mix : 0);
    if (!this.crush) this.set(this.stages.get('bitcrush')!.dry.gain, 1);

    const d = v.distortion;
    this.set(this.pre.gain, dbToGain(d.drive));
    this.set(this.distTone.frequency, d.tone);
    this.set(this.post.gain, dbToGain(d.level));
    blend('distortion', d.mix);

    const e = v.eq;
    this.set(this.low.frequency, e.lowFreq);
    this.set(this.low.gain, e.lowGain);
    this.set(this.mid.frequency, e.midFreq);
    this.set(this.mid.Q, e.midQ);
    this.set(this.mid.gain, e.midGain);
    this.set(this.high.frequency, e.highFreq);
    this.set(this.high.gain, e.highGain);
    blend('eq', null);

    const c = v.compressor;
    this.set(this.comp.threshold, c.threshold);
    this.set(this.comp.ratio, c.ratio);
    this.set(this.comp.attack, c.attack / 1000);
    this.set(this.comp.release, c.release / 1000);
    this.set(this.comp.knee, c.knee);
    this.set(this.makeup.gain, dbToGain(c.makeup));
    blend('compressor', null);

    const y = v.delay;
    this.set(this.delay.delayTime, y.time / 1000);
    this.set(this.feedback.gain, y.feedback);
    this.set(this.echoTone.frequency, y.tone);
    blend('delay', y.mix);

    const r = v.reverb;
    this.scheduleImpulse(r.size, r.damping, r.preDelay);
    blend('reverb', r.mix);
  }

  /** The reverb's impulse response is rebuilt (after a short pause, so dragging a slider is not choppy) when its shape changes. */
  private scheduleImpulse(size: number, damping: number, preDelay: number): void {
    const key = `${size}|${damping}|${preDelay}`;
    if (key === this.irKey) return;
    const build = (): void => {
      this.irKey = key;
      const [l, r] = impulseResponse(this.ctx.sampleRate, size, damping, preDelay);
      const buf = this.ctx.createBuffer(2, l.length, this.ctx.sampleRate);
      buf.getChannelData(0).set(l);
      buf.getChannelData(1).set(r);
      this.convolver.buffer = buf;
    };
    clearTimeout(this.irTimer);
    if (!this.convolver.buffer) build();
    else this.irTimer = window.setTimeout(build, 120);
  }
}
