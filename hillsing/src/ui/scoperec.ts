// Records the Scope for as long as you like (up to three minutes) and turns it into one tall PNG: the waveform and
// spectrogram in rows of about eleven seconds each, stacked, with the syllable marks and a time axis on every row.
// It keeps its own fixed-rate columns, so the recording does not depend on the live window's size or zoom.

import { colour } from './spectrogram';

export const REC_RATE = 125; // columns per second (8 ms each)
export const REC_MAX_SEC = 180;
const ROW_SLOTS = 1400; // columns per row of the picture (11.2 s)
const WAVE_H = 56;
const SPEC_H = 86;
const LABEL_H = 16;
const ROW_GAP = 8;
const MAX_HZ = 8000;
const CLIP = 0.999;

const LUT = new Uint8Array(256 * 3);
for (let v = 0; v < 256; v++) LUT.set(colour(v), v * 3);

interface Mark {
  slot: number;
  label: string;
}

export class ScopeRecorder {
  on = false;
  private minV: number[] = [];
  private maxV: number[] = [];
  private spec: Uint8Array[] = [];
  private marks: Mark[] = [];
  private carry = 0;
  private last = 0;
  clips = 0;
  sampleRate = 44100;
  startedAt = new Date();

  get seconds(): number {
    return this.minV.length / REC_RATE;
  }

  get hasData(): boolean {
    return this.minV.length > 0;
  }

  start(now: number, sampleRate: number): void {
    this.minV = [];
    this.maxV = [];
    this.spec = [];
    this.marks = [];
    this.clips = 0;
    this.carry = 0;
    this.last = now;
    this.on = true;
    this.sampleRate = sampleRate;
    this.startedAt = new Date();
  }

  stop(): void {
    this.on = false;
  }

  clear(): void {
    this.on = false;
    this.minV = [];
    this.maxV = [];
    this.spec = [];
    this.marks = [];
  }

  mark(label: string): void {
    if (this.on) this.marks.push({ slot: this.minV.length, label });
  }

  /** Add the columns that have gone by since the last call. Returns true when the length limit was reached. */
  push(now: number, tail: Float32Array, bins: Uint8Array, usable: number): boolean {
    this.carry += ((now - this.last) / 1000) * REC_RATE;
    this.last = now;
    const total = Math.floor(this.carry);
    if (total < 1) return false;
    this.carry -= total;
    const sps = Math.max(1, Math.round(this.sampleRate / REC_RATE));
    const real = Math.min(total, Math.floor(tail.length / sps));
    // Columns the page was too busy to sample (a slow frame, a background tab) are kept as silence so times stay true.
    for (let k = 0; k < total - real; k++) {
      this.minV.push(0);
      this.maxV.push(0);
      this.spec.push(new Uint8Array(usable));
    }
    for (let k = 0; k < real; k++) {
      const from = tail.length - (real - k) * sps;
      let lo = 0;
      let hi = 0;
      for (let i = from; i < from + sps; i++) {
        const v = tail[i];
        if (v < lo) lo = v;
        if (v > hi) hi = v;
      }
      this.minV.push(lo);
      this.maxV.push(hi);
      this.spec.push(bins.slice(0, usable));
      if (hi >= CLIP || lo <= -CLIP) this.clips++;
    }
    return this.minV.length >= REC_MAX_SEC * REC_RATE;
  }

  /** The whole recording as a picture. `meta` lines are printed at the top. */
  render(meta: string[]): HTMLCanvasElement {
    const n = this.minV.length;
    const usable = this.spec[0]?.length ?? 1;
    const rows = Math.max(1, Math.ceil(n / ROW_SLOTS));
    const pad = 10;
    const lineH = 15;
    const raw = [
      `HillSing scope recording · ${this.startedAt.toLocaleString()} · ${this.seconds.toFixed(1)} s · ${this.sampleRate} Hz · ${this.clips} clipped column${this.clips === 1 ? '' : 's'}`,
      ...meta,
      'Waveform is before the output limiter (red = full scale). Spectrogram is 0-8 kHz after it. Gold lines mark syllables (↑ = key released). Each row is 11.2 s.',
    ];
    const measure = document.createElement('canvas').getContext('2d')!;
    measure.font = '11px system-ui, sans-serif';
    const lines: string[] = [];
    for (const text of raw) {
      let cur = '';
      for (const word of text.split(' ')) {
        const next = cur ? `${cur} ${word}` : word;
        if (cur && measure.measureText(next).width > ROW_SLOTS) {
          lines.push(cur);
          cur = word;
        } else cur = next;
      }
      lines.push(cur);
    }
    const head = pad + lines.length * lineH + 4;
    const rowH = LABEL_H + WAVE_H + SPEC_H + ROW_GAP;
    const out = document.createElement('canvas');
    out.width = ROW_SLOTS + pad * 2;
    out.height = head + rows * rowH + pad;
    const g = out.getContext('2d')!;
    g.fillStyle = '#14171c';
    g.fillRect(0, 0, out.width, out.height);
    g.fillStyle = '#e6e9ee';
    g.textBaseline = 'top';
    g.font = '11px system-ui, sans-serif';
    lines.forEach((l, i) => g.fillText(l, pad, pad + i * lineH));

    for (let r = 0; r < rows; r++) {
      const i0 = r * ROW_SLOTS;
      const i1 = Math.min(n, i0 + ROW_SLOTS);
      const y0 = head + r * rowH;
      const wy = y0 + LABEL_H;
      const sy = wy + WAVE_H;
      g.fillStyle = '#e6e9ee';
      g.font = '10px system-ui, sans-serif';
      g.fillText(`${(i0 / REC_RATE).toFixed(1)} – ${(i1 / REC_RATE).toFixed(1)} s`, pad, y0 + 2);
      // waveform
      g.fillStyle = '#0b0f16';
      g.fillRect(pad, wy, ROW_SLOTS, WAVE_H);
      const mid = wy + WAVE_H / 2;
      const scale = WAVE_H / 2 / 1.3;
      g.fillStyle = 'rgba(255,255,255,0.12)';
      g.fillRect(pad, Math.round(mid), ROW_SLOTS, 1);
      g.fillStyle = 'rgba(255,74,61,0.45)';
      g.fillRect(pad, Math.round(mid - scale), ROW_SLOTS, 1);
      g.fillRect(pad, Math.round(mid + scale), ROW_SLOTS, 1);
      for (let i = i0; i < i1; i++) {
        const lo = this.minV[i];
        const hi = this.maxV[i];
        g.fillStyle = hi >= CLIP || lo <= -CLIP ? '#ff4a3d' : '#6cb6ff';
        const ya = mid - hi * scale;
        g.fillRect(pad + (i - i0), ya, 1, Math.max(1, mid - lo * scale - ya));
      }
      // spectrogram
      const img = g.createImageData(ROW_SLOTS, SPEC_H);
      const rowBin = new Int32Array(SPEC_H);
      for (let y = 0; y < SPEC_H; y++) rowBin[y] = Math.min(usable - 1, Math.floor(((SPEC_H - 1 - y) / SPEC_H) * usable));
      const grid = new Set([1000, 2000, 4000, 6000].map((hz) => Math.min(SPEC_H - 1, Math.round(SPEC_H - 1 - (hz / MAX_HZ) * SPEC_H))));
      for (let x = 0; x < ROW_SLOTS; x++) {
        const col = this.spec[i0 + x];
        for (let y = 0; y < SPEC_H; y++) {
          let rr = 0;
          let gg = 0;
          let bb = 40;
          if (col) {
            const li = col[rowBin[y]] * 3;
            rr = LUT[li];
            gg = LUT[li + 1];
            bb = LUT[li + 2];
          }
          if (grid.has(y)) {
            rr = Math.min(255, rr + 40);
            gg = Math.min(255, gg + 40);
            bb = Math.min(255, bb + 40);
          }
          const p = (y * ROW_SLOTS + x) * 4;
          img.data[p] = rr;
          img.data[p + 1] = gg;
          img.data[p + 2] = bb;
          img.data[p + 3] = 255;
        }
      }
      g.putImageData(img, pad, sy);
      g.font = '9px system-ui, sans-serif';
      g.fillStyle = 'rgba(255,255,255,0.8)';
      for (const [hz, label] of [[1000, '1k'], [2000, '2k'], [4000, '4k'], [6000, '6k']] as const) g.fillText(label, pad + 2, sy + (1 - hz / MAX_HZ) * SPEC_H - 10);
      // second ticks and syllable marks across both
      g.fillStyle = 'rgba(255,255,255,0.35)';
      for (let t = Math.ceil(i0 / REC_RATE); t * REC_RATE < i1; t++) {
        const x = pad + t * REC_RATE - i0;
        g.fillRect(x, wy, 1, 4);
        if (t % 2 === 0) g.fillText(`${t}`, x + 2, wy + 4);
      }
      g.font = '10px system-ui, sans-serif';
      for (const m of this.marks) {
        if (m.slot < i0 || m.slot >= i1) continue;
        const x = pad + (m.slot - i0);
        g.fillStyle = 'rgba(240,180,41,0.85)';
        g.fillRect(x, wy, 1, WAVE_H + SPEC_H);
        g.fillText(m.label, x + 3, wy + WAVE_H - 12);
      }
    }
    return out;
  }
}
