// The Scope: a scrolling waveform and spectrogram of everything coming out of the audio bus, on one shared time axis,
// so a wobble, click or clip can be lined up with the syllable and key that made it. The waveform is read before the
// output limiter (so a peak past full scale shows in red), the spectrogram after it. Freeze it (or let it freeze
// itself when the signal clips) and hover to read off the moment.

import type { AudioBus } from '../audio/bus';
import { colour } from './spectrogram';
import { REC_MAX_SEC, ScopeRecorder } from './scoperec';

const MAX_HZ = 8000;
const CLIP = 0.999;
const GRID_HZ = [1000, 2000, 4000, 6000];

const LUT = new Uint8Array(256 * 3);
for (let v = 0; v < 256; v++) LUT.set(colour(v), v * 3);

interface Mark {
  slot: number;
  label: string;
  /** Flagged by the user as a sound with an artifact. */
  flag?: boolean;
}

export class Scope {
  private bus: AudioBus | null = null;
  private frozen = false;
  private windowSec = 4;
  private cols = 0;
  private minV = new Float32Array(0);
  private maxV = new Float32Array(0);
  private spec = new Uint8Array(0);
  private slot = 0; // absolute index of the newest column
  private lastT = 0;
  private carry = 0;
  private marks: Mark[] = [];
  private tail = new Float32Array(2048);
  private bins = new Uint8Array(0);
  private usable = 1;
  private peak = 0;
  private hover: number | null = null;
  private readonly freezeBtn: HTMLButtonElement;
  private readonly clipFreeze: HTMLInputElement;
  private readonly readout: HTMLElement;
  private readonly meta: () => string[];
  private readonly badge: HTMLElement | null;
  private readonly rec = new ScopeRecorder();
  private readonly recBtn: HTMLButtonElement;
  private readonly clearBtn: HTMLButtonElement;

  constructor(
    private readonly wave: HTMLCanvasElement,
    private readonly specCanvas: HTMLCanvasElement,
    controls: {
      freeze: HTMLButtonElement;
      clipFreeze: HTMLInputElement;
      window: HTMLSelectElement;
      readout: HTMLElement;
      save: HTMLButtonElement;
      copy: HTMLButtonElement;
      record: HTMLButtonElement;
      clear: HTMLButtonElement;
      /** Lines describing the current voice, tuning and effects, printed at the top of an exported image. */
      meta: () => string[];
      /** The button that folds the scope away, and the badge shown while a recording runs in the folded scope. */
      toggle?: HTMLButtonElement;
      badge?: HTMLElement;
    },
  ) {
    this.freezeBtn = controls.freeze;
    this.clipFreeze = controls.clipFreeze;
    this.readout = controls.readout;
    this.freezeBtn.addEventListener('click', () => {
      this.setFrozen(!this.frozen);
      this.freezeBtn.blur();
    });
    controls.window.addEventListener('change', () => {
      this.windowSec = Number(controls.window.value) || 4;
      this.resetData();
      controls.window.blur();
    });
    this.clipFreeze.addEventListener('change', () => this.clipFreeze.blur());
    this.meta = controls.meta;
    this.badge = controls.badge ?? null;
    this.recBtn = controls.record;
    this.clearBtn = controls.clear;
    this.recBtn.addEventListener('click', () => {
      this.recBtn.blur();
      this.toggleRecord();
    });
    this.clearBtn.addEventListener('click', () => {
      this.clearBtn.blur();
      this.rec.clear();
      this.clearBtn.hidden = true;
      this.readout.textContent = 'Recording cleared: Save PNG and Copy image use the live window again.';
    });
    controls.save.addEventListener('click', () => {
      controls.save.blur();
      void this.save();
    });
    controls.copy.addEventListener('click', () => {
      controls.copy.blur();
      void this.copyImage();
    });
    for (const c of [wave, specCanvas]) {
      c.addEventListener('pointermove', (e) => {
        const r = c.getBoundingClientRect();
        this.hover = Math.max(0, Math.min(this.cols - 1, Math.floor(((e.clientX - r.left) / r.width) * this.cols)));
        if (this.frozen) this.draw();
        this.describe();
      });
      c.addEventListener('pointerleave', () => {
        this.hover = null;
        this.describe();
      });
    }
    new ResizeObserver(() => this.fit()).observe(wave);
    this.fit();
  }

  attach(bus: AudioBus): void {
    this.bus = bus;
    this.bins = new Uint8Array(bus.spectrumBins);
    this.usable = Math.max(1, Math.floor((MAX_HZ / (bus.ctx.sampleRate / 2)) * bus.spectrumBins));
    this.tail = new Float32Array(bus.waveformTailLength);
    this.resetData();
    this.lastT = performance.now();
    requestAnimationFrame(this.frame);
  }

  /** Note something that just started (a syllable, a release) so it is labelled on the time axis. */
  mark(label: string, flag = false): void {
    if (!this.bus) return;
    this.rec.mark(label, flag);
    if (this.frozen) return;
    this.marks.push({ slot: this.slot, label, flag });
    if (this.marks.length > 200) this.marks.splice(0, this.marks.length - 200);
  }

  /** The settings lines printed on exports (also used by the flag list). */
  metaLines(): string[] {
    return this.meta();
  }

  /** Show a message in the readout line (not while recording, which owns it). */
  say(text: string): void {
    if (!this.rec.on) this.readout.textContent = text;
  }

  private setFrozen(on: boolean, why = ''): void {
    this.frozen = on;
    this.freezeBtn.textContent = on ? '▶ Live' : '⏸ Freeze';
    this.freezeBtn.setAttribute('aria-pressed', String(on));
    this.freezeBtn.classList.toggle('active', on);
    if (!on) {
      this.lastT = performance.now();
      this.carry = 0;
    }
    this.readout.textContent = on ? why || 'Frozen: hover to read the moment under the pointer.' : '';
    this.draw();
  }

  private fit(): void {
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(200, Math.floor(this.wave.clientWidth * dpr));
    for (const [c, hCss] of [[this.wave, this.wave.clientHeight], [this.specCanvas, this.specCanvas.clientHeight]] as const) {
      const h = Math.max(60, Math.floor(hCss * dpr));
      if (c.width !== w || c.height !== h) {
        c.width = w;
        c.height = h;
      }
    }
    if (w !== this.cols) {
      this.cols = w;
      this.resetData();
    } else this.draw();
  }

  private resetData(): void {
    this.minV = new Float32Array(this.cols);
    this.maxV = new Float32Array(this.cols);
    this.spec = new Uint8Array(this.cols * this.usable);
    this.marks = [];
    this.peak = 0;
    this.draw();
  }

  private get slotSec(): number {
    return this.windowSec / this.cols;
  }

  private frame = (now: number): void => {
    requestAnimationFrame(this.frame);
    const bus = this.bus;
    if (!bus) return;
    const showing = !this.frozen && this.cols > 0 && this.wave.offsetParent !== null;
    if (!showing) this.lastT = now; // hidden or frozen: do not build up a backlog
    if (!showing && !this.rec.on) return;
    bus.waveformTail(this.tail);
    bus.spectrumInto(this.bins);
    if (this.rec.on) {
      const full = this.rec.push(now, this.tail, this.bins, this.usable);
      this.readout.textContent = `● REC ${this.rec.seconds.toFixed(1)} s · ${this.rec.clips} clipped column${this.rec.clips === 1 ? '' : 's'}`;
      if (full) {
        this.toggleRecord(`Stopped at the ${REC_MAX_SEC / 60}-minute limit. `);
      }
    }
    if (!showing) return;
    const sr = bus.ctx.sampleRate;
    this.carry += (now - this.lastT) / 1000 / this.slotSec;
    this.lastT = now;
    const sps = Math.max(1, Math.round(sr * this.slotSec));
    const slots = Math.min(Math.floor(this.carry), Math.floor(this.tail.length / sps), this.cols);
    if (this.carry >= 1) this.carry -= Math.floor(this.carry);
    if (slots < 1) return;
    let clipped = false;
    for (let k = 0; k < slots; k++) {
      // The newest `slots * sps` samples, oldest slot first.
      const from = this.tail.length - (slots - k) * sps;
      let lo = 0;
      let hi = 0;
      for (let i = from; i < from + sps; i++) {
        const v = this.tail[i];
        if (v < lo) lo = v;
        if (v > hi) hi = v;
      }
      this.slot++;
      const c = this.slot % this.cols;
      this.minV[c] = lo;
      this.maxV[c] = hi;
      this.spec.set(this.bins.subarray(0, this.usable), c * this.usable);
      this.peak = Math.max(this.peak * 0.995, Math.max(hi, -lo));
      if (hi >= CLIP || lo <= -CLIP) clipped = true;
    }
    this.draw();
    if (clipped && this.clipFreeze.checked) this.setFrozen(true, 'Froze because the signal reached full scale (red). Hover to read the moment.');
  };

  /** Start recording, or stop and save the recording as a PNG. */
  private toggleRecord(prefix = ''): void {
    if (!this.bus) {
      this.readout.textContent = 'Start audio first, then record.';
      return;
    }
    if (!this.rec.on) {
      this.rec.start(performance.now(), this.bus.ctx.sampleRate);
      this.recBtn.textContent = '⏹ Stop recording';
      this.recBtn.classList.add('recording');
      if (this.badge) this.badge.hidden = false;
      this.recBtn.setAttribute('aria-pressed', 'true');
      this.clearBtn.hidden = true;
      this.readout.textContent = '● REC 0.0 s';
      return;
    }
    this.rec.stop();
    this.recBtn.textContent = '⏺ Record';
    this.recBtn.classList.remove('recording');
    if (this.badge) this.badge.hidden = true;
    this.recBtn.setAttribute('aria-pressed', 'false');
    if (!this.rec.hasData) {
      this.readout.textContent = `${prefix}Nothing was recorded.`;
      return;
    }
    this.clearBtn.hidden = false;
    void this.save(`${prefix}Recorded ${this.rec.seconds.toFixed(1)} s. `);
  }

  private columnAt(x: number): number {
    // x is a canvas column 0..cols-1 (oldest..newest); return its ring index and absolute slot.
    return this.slot - (this.cols - 1 - x);
  }

  private draw(): void {
    const g = this.wave.getContext('2d');
    const s = this.specCanvas.getContext('2d');
    if (!g || !s || this.cols === 0) return;
    const w = this.wave.width;
    const h = this.wave.height;
    // ---- waveform ----
    g.fillStyle = '#0b0f16';
    g.fillRect(0, 0, w, h);
    const mid = h / 2;
    const scale = (h / 2) / 1.3; // a little headroom above full scale
    g.strokeStyle = 'rgba(255,255,255,0.12)';
    g.lineWidth = 1;
    g.beginPath();
    g.moveTo(0, mid + 0.5);
    g.lineTo(w, mid + 0.5);
    g.stroke();
    g.strokeStyle = 'rgba(255,74,61,0.55)';
    g.setLineDash([4, 4]);
    for (const sign of [-1, 1]) {
      const y = Math.round(mid - sign * scale) + 0.5;
      g.beginPath();
      g.moveTo(0, y);
      g.lineTo(w, y);
      g.stroke();
    }
    g.setLineDash([]);
    for (let x = 0; x < w; x++) {
      const abs = this.columnAt(x);
      if (abs <= 0) continue;
      const c = ((abs % this.cols) + this.cols) % this.cols;
      const lo = this.minV[c];
      const hi = this.maxV[c];
      const clipped = hi >= CLIP || lo <= -CLIP;
      g.fillStyle = clipped ? '#ff4a3d' : '#6cb6ff';
      const y0 = mid - hi * scale;
      const y1 = mid - lo * scale;
      g.fillRect(x, y0, 1, Math.max(1, y1 - y0));
    }
    // ---- marks (shared by both canvases) ----
    const dpr = window.devicePixelRatio || 1;
    g.font = `${Math.round(10 * dpr)}px system-ui, sans-serif`;
    g.textBaseline = 'top';
    const live = this.marks.filter((m) => this.slot - m.slot < this.cols);
    for (const m of live) {
      const x = this.cols - 1 - (this.slot - m.slot);
      g.fillStyle = m.flag ? '#ff4a3d' : 'rgba(240,180,41,0.9)';
      g.fillRect(x, 0, m.flag ? 2 : 1, h);
      g.fillText(m.flag ? `⚠ ${m.label}` : m.label, x + 3, m.flag ? 14 : 2);
    }
    this.drawHover(g, h);
    // ---- spectrogram ----
    const sw = this.specCanvas.width;
    const sh = this.specCanvas.height;
    const img = s.createImageData(sw, sh);
    const gridRows = new Set(GRID_HZ.map((hz) => Math.min(sh - 1, Math.round(sh - 1 - (hz / MAX_HZ) * sh))));
    const rowBin = new Int32Array(sh);
    for (let y = 0; y < sh; y++) rowBin[y] = Math.min(this.usable - 1, Math.floor(((sh - 1 - y) / sh) * this.usable));
    for (let x = 0; x < sw; x++) {
      const abs = this.columnAt(x);
      const c = ((abs % this.cols) + this.cols) % this.cols;
      const have = abs > 0;
      const base = c * this.usable;
      for (let y = 0; y < sh; y++) {
        let r = 0;
        let gr = 0;
        let b = 40;
        if (have) {
          const li = this.spec[base + rowBin[y]] * 3;
          r = LUT[li];
          gr = LUT[li + 1];
          b = LUT[li + 2];
        }
        if (gridRows.has(y)) {
          r = Math.min(255, r + 40);
          gr = Math.min(255, gr + 40);
          b = Math.min(255, b + 40);
        }
        const i = (y * sw + x) * 4;
        img.data[i] = r;
        img.data[i + 1] = gr;
        img.data[i + 2] = b;
        img.data[i + 3] = 255;
      }
    }
    s.putImageData(img, 0, 0);
    for (const m of live) {
      const x = this.cols - 1 - (this.slot - m.slot);
      s.fillStyle = m.flag ? '#ff4a3d' : 'rgba(240,180,41,0.8)';
      s.fillRect(x, 0, m.flag ? 2 : 1, sh);
    }
    this.drawHover(s, sh);
  }

  /** The waveform and spectrogram as one picture, with a header saying what made the sound and a time axis under it. */
  private render(): HTMLCanvasElement {
    const savedHover = this.hover;
    this.hover = null;
    this.draw();
    const dpr = window.devicePixelRatio || 1;
    const pad = Math.round(10 * dpr);
    const line = Math.round(15 * dpr);
    const raw = [
      `HillSing scope · ${new Date().toLocaleString()} · window ${this.windowSec} s · ${this.frozen ? 'frozen' : 'live'}${this.bus ? ` · ${this.bus.ctx.sampleRate} Hz` : ''}`,
      ...this.meta(),
      'Waveform is before the output limiter (red = full scale). Spectrogram is 0-8 kHz after it. Gold lines mark syllables (↑ = key released), red lines (⚠) the ones you flagged.',
    ];
    // Wrap long lines to the picture's width so nothing is cut off.
    const measure = document.createElement('canvas').getContext('2d')!;
    measure.font = `${Math.round(11 * dpr)}px system-ui, sans-serif`;
    const maxW = this.wave.width;
    const lines: string[] = [];
    for (const text of raw) {
      let cur = '';
      for (const word of text.split(' ')) {
        const next = cur ? `${cur} ${word}` : word;
        if (cur && measure.measureText(next).width > maxW) {
          lines.push(cur);
          cur = word;
        } else cur = next;
      }
      lines.push(cur);
    }
    const head = pad + lines.length * line;
    const axis = Math.round(18 * dpr);
    const w = this.wave.width;
    const out = document.createElement('canvas');
    out.width = w + pad * 2;
    out.height = head + this.wave.height + this.specCanvas.height + axis + pad;
    const g = out.getContext('2d')!;
    g.fillStyle = '#14171c';
    g.fillRect(0, 0, out.width, out.height);
    g.fillStyle = '#e6e9ee';
    g.textBaseline = 'top';
    g.font = `${Math.round(11 * dpr)}px system-ui, sans-serif`;
    lines.forEach((l, i) => g.fillText(l, pad, pad + i * line));
    g.drawImage(this.wave, pad, head);
    g.drawImage(this.specCanvas, pad, head + this.wave.height);
    g.fillStyle = 'rgba(255,255,255,0.8)';
    g.font = `${Math.round(10 * dpr)}px system-ui, sans-serif`;
    const sy = head + this.wave.height;
    for (const [hz, label] of [[1000, '1k'], [2000, '2k'], [4000, '4k'], [6000, '6k'], [8000, '8k']] as const) {
      g.fillText(label, pad + 3, sy + (1 - hz / MAX_HZ) * this.specCanvas.height - (hz === MAX_HZ ? 0 : 5 * dpr));
    }
    // time axis: seconds before "now" at the right edge
    const ay = head + this.wave.height + this.specCanvas.height + Math.round(3 * dpr);
    for (let t = 0; t <= this.windowSec; t += this.windowSec <= 2 ? 0.5 : 1) {
      const x = pad + w - (t / this.windowSec) * (w - 1);
      g.fillRect(x, ay - 3 * dpr, 1, 3 * dpr);
      g.fillText(t === 0 ? 'now' : `-${t}s`, Math.min(x + 2, pad + w - 24 * dpr), ay);
    }
    this.hover = savedHover;
    this.draw();
    return out;
  }

  /** What Save and Copy export: the last recording if there is one, otherwise the live window. */
  private picture(): HTMLCanvasElement {
    return this.rec.hasData && !this.rec.on ? this.rec.render(this.meta()) : this.render();
  }

  private async save(note = ''): Promise<void> {
    if (!this.bus) {
      this.readout.textContent = 'Nothing to export yet: start audio and play something first.';
      return;
    }
    const canvas = this.picture();
    const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/png'));
    if (!blob) {
      this.readout.textContent = 'Could not make the image.';
      return;
    }
    const d = new Date();
    const pad2 = (n: number): string => String(n).padStart(2, '0');
    const name = `hillsing-scope-${d.getFullYear()}${pad2(d.getMonth() + 1)}${pad2(d.getDate())}-${pad2(d.getHours())}${pad2(d.getMinutes())}${pad2(d.getSeconds())}.png`;
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = name;
    document.body.append(a);
    a.click();
    a.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 10000);
    this.readout.textContent = `${note}Saved ${name} (if nothing downloaded here, use Copy).`;
  }

  private async copyImage(): Promise<void> {
    if (!this.bus) {
      this.readout.textContent = 'Nothing to export yet: start audio and play something first.';
      return;
    }
    const canvas = this.picture();
    try {
      const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/png'));
      if (!blob) throw new Error('no image');
      await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]);
      this.readout.textContent = 'Image copied: paste it into a message or document.';
    } catch {
      this.readout.textContent = 'This browser would not copy the image. Use Save PNG instead.';
    }
  }

  private drawHover(g: CanvasRenderingContext2D, h: number): void {
    if (this.hover === null || !this.frozen) return;
    g.fillStyle = 'rgba(255,255,255,0.7)';
    g.fillRect(this.hover, 0, 1, h);
  }

  private describe(): void {
    if (this.hover === null) {
      if (!this.frozen) this.readout.textContent = '';
      return;
    }
    const abs = this.columnAt(this.hover);
    if (abs <= 0) return;
    const c = ((abs % this.cols) + this.cols) % this.cols;
    const peak = Math.max(this.maxV[c], -this.minV[c]);
    const db = peak > 0 ? 20 * Math.log10(peak) : -Infinity;
    const ago = (this.cols - 1 - this.hover) * this.slotSec;
    const near = this.marks.filter((m) => Math.abs(m.slot - abs) <= 3);
    // loudest spectral bin in this column
    let best = 0;
    let bestV = 0;
    for (let i = 0; i < this.usable; i++) {
      const v = this.spec[c * this.usable + i];
      if (v > bestV) {
        bestV = v;
        best = i;
      }
    }
    const hz = this.bus ? Math.round((best / this.bus.spectrumBins) * (this.bus.ctx.sampleRate / 2)) : 0;
    this.readout.textContent =
      `${ago.toFixed(2)} s ago · peak ${Number.isFinite(db) ? db.toFixed(1) : '-∞'} dBFS${peak >= CLIP ? ' (CLIPPED)' : ''} · loudest band ${hz} Hz` +
      (near.length ? ` · ${near.map((m) => m.label).join(', ')}` : '');
  }
}
