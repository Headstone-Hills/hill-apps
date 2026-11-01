// The one audio path for every voice: sources (formant synth, live samples, sample melodies, Voice lab) plug into
// `input`; the effects rack processes the mix; a limiter and soft clip protect the speakers; an analyser (the meter and
// spectrogram read it) comes last.
// One AudioContext for the whole page, created on the first click.

import { EffectsRack } from './effects';
import { FxModel, limiterCurve } from './fxparams';

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
    private readonly preMeter: AnalyserNode,
  ) {}

  private outputId = '';
  private monitor: { dest: MediaStreamAudioDestinationNode; el: HTMLAudioElement } | null = null;

  /** True when the browser can send the sound to a chosen output device (Chromium and Electron can). */
  get canChooseOutput(): boolean {
    return typeof (this.ctx as AudioContext & { setSinkId?: unknown }).setSinkId === 'function';
  }

  /** The audio outputs the browser can see. Names are blank unless the app is allowed to read device labels. */
  async outputDevices(): Promise<Array<{ id: string; label: string }>> {
    if (!navigator.mediaDevices?.enumerateDevices) return [];
    const all = await navigator.mediaDevices.enumerateDevices();
    return all.filter((d) => d.kind === 'audiooutput').map((d, i) => ({ id: d.deviceId, label: d.label || `Output ${i + 1} (name not available)` }));
  }

  /** Play everything to this output device ('' = the system default). */
  async setOutput(id: string): Promise<void> {
    const ctx = this.ctx as AudioContext & { setSinkId?: (id: string) => Promise<void> };
    if (!ctx.setSinkId) throw new Error('This browser cannot choose an output device');
    await ctx.setSinkId(id);
    this.outputId = id;
  }

  private mic: { stream: MediaStream; source: MediaStreamAudioSourceNode; gain: GainNode } | null = null;
  private micLevel = 1;

  /** The audio inputs (microphones) the browser can see, without the virtual cables (a cable that carries our own output would loop). */
  async inputDevices(): Promise<Array<{ id: string; label: string }>> {
    if (!navigator.mediaDevices?.enumerateDevices) return [];
    const all = await navigator.mediaDevices.enumerateDevices();
    return all
      .filter((d) => d.kind === 'audioinput' && !/blackhole|loopback|soundflower|aggregate/i.test(d.label))
      .map((d, i) => ({ id: d.deviceId, label: d.label || `Microphone ${i + 1} (name not available)` }));
  }

  /**
   * Mix a microphone into the output (for talking over the music on a call). It goes straight to the output device, past
   * the effects, the limiter and the second-device monitor, so you do not hear yourself back. No echo cancellation,
   * noise suppression or auto-gain, which would mangle it. Pass null to turn it off.
   */
  async setMic(id: string | null): Promise<void> {
    if (this.mic) {
      this.mic.source.disconnect();
      this.mic.gain.disconnect();
      for (const t of this.mic.stream.getTracks()) t.stop();
      this.mic = null;
    }
    if (id === null) return;
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { deviceId: id ? { exact: id } : undefined, echoCancellation: false, noiseSuppression: false, autoGainControl: false, channelCount: 1 },
    });
    const source = this.ctx.createMediaStreamSource(stream);
    const gain = this.ctx.createGain();
    gain.gain.value = this.micLevel;
    source.connect(gain).connect(this.ctx.destination);
    this.mic = { stream, source, gain };
  }

  setMicLevel(level: number): void {
    this.micLevel = Math.max(0, Math.min(2, level));
    if (this.mic) this.mic.gain.gain.setTargetAtTime(this.micLevel, this.ctx.currentTime, 0.02);
  }

  /** The microphone's output node while a microphone is mixed in (so a recording can include it), else null. */
  get micNode(): AudioNode | null {
    return this.mic ? this.mic.gain : null;
  }

  get currentOutput(): string {
    return this.outputId;
  }

  /**
   * Also play the sound on a second device (to hear yourself while the main output goes to a virtual cable). It rides
   * on an audio element, so it arrives a little later than the main output. Pass null to stop.
   */
  async setMonitor(id: string | null): Promise<void> {
    if (this.monitor) {
      this.analyser.disconnect(this.monitor.dest);
      this.monitor.el.pause();
      this.monitor.el.srcObject = null;
      this.monitor.el.remove();
      this.monitor = null;
    }
    if (id === null) return;
    const dest = this.ctx.createMediaStreamDestination();
    this.analyser.connect(dest);
    const el = document.createElement('audio') as HTMLAudioElement & { setSinkId?: (id: string) => Promise<void> };
    el.srcObject = dest.stream;
    el.style.display = 'none';
    document.body.append(el);
    if (el.setSinkId) await el.setSinkId(id);
    await el.play();
    this.monitor = { dest, el };
  }

  /** The analyser that watches the signal before the output limiter (for measuring). */
  get preAnalyser(): AnalyserNode {
    return this.preMeter;
  }

  /** Fill `into` (up to 2048 samples) with the newest output samples as they were before the limiter, so clipping shows. */
  waveformTail(into: Float32Array<ArrayBuffer>): void {
    this.preMeter.getFloatTimeDomainData(into);
  }

  get waveformTailLength(): number {
    return this.preMeter.fftSize;
  }

  /** Current output spectrum (after the limiter), 0-255 on the analyser's dB scale. */
  spectrumInto(into: Uint8Array<ArrayBuffer>): void {
    this.analyser.getByteFrequencyData(into);
  }

  get spectrumBins(): number {
    return this.analyser.frequencyBinCount;
  }

  /** Highest sample (absolute) of the last ~20 ms before the output limiter: 1 or more means it would have clipped. */
  overPeak(): number {
    const buf = new Float32Array(this.preMeter.fftSize);
    this.preMeter.getFloatTimeDomainData(buf);
    let peak = 0;
    for (const x of buf) peak = Math.max(peak, Math.abs(x));
    return peak;
  }

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
    // Output safety: whatever the effects add (make-up gain, EQ boosts, a reverb tail on top of a loud vowel), a fast
    // limiter catches the peaks and a soft knee rounds off any that get through, so nothing hard-clips at the speakers.
    const preMeter = ctx.createAnalyser();
    preMeter.fftSize = 2048;
    const limiter = ctx.createDynamicsCompressor();
    limiter.threshold.value = -3;
    limiter.knee.value = 0;
    limiter.ratio.value = 20;
    limiter.attack.value = 0.001;
    limiter.release.value = 0.08;
    const half = ctx.createGain();
    half.gain.value = 0.5;
    const soft = ctx.createWaveShaper();
    soft.curve = limiterCurve();
    fx.output.connect(preMeter);
    fx.output.connect(limiter);
    limiter.connect(half);
    half.connect(soft);
    soft.connect(analyser);
    analyser.connect(ctx.destination);
    fx.apply(fxModel.state);
    fxModel.onChange(() => fx.apply(fxModel.state));
    if (ctx.state !== 'running') await ctx.resume();
    return new AudioBus(ctx, input, analyser, fx, preMeter);
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
