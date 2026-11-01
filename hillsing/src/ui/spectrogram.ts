import type { SingerEngine } from '../synth/engine';

const MAX_HZ = 8000;
const PIXELS_PER_SECOND = 110;
/** Grid / label frequencies (the matching labels are HTML text over the canvas). */
const LABEL_HZ = [1000, 2000, 4000, 6000];

/** Map 0..255 (analyser dB scale) to a dark-blue -> cyan -> yellow -> white colour. */
export function colour(v: number): [number, number, number] {
  const t = v / 255;
  if (t < 0.35) return [0, 0, Math.round(40 + 400 * t)];
  if (t < 0.65) {
    const u = (t - 0.35) / 0.3;
    return [Math.round(40 * u), Math.round(200 * u), Math.round(180 - 60 * u)];
  }
  const u = (t - 0.65) / 0.35;
  return [Math.round(40 + 215 * u), Math.round(200 + 55 * u), Math.round(120 + 135 * u)];
}

/** Scrolling spectrogram (0-8 kHz) of the synth's output: shows formants, noise and bursts. */
export class Spectrogram {
  private engine: SingerEngine | null = null;
  private last = 0;
  private carry = 0;
  private data = new Uint8Array(0);

  constructor(private readonly canvas: HTMLCanvasElement) {
    this.fit();
    window.addEventListener('resize', () => this.fit());
    this.clear();
  }

  attach(engine: SingerEngine): void {
    this.engine = engine;
    this.data = new Uint8Array(engine.spectrumBins);
    this.last = performance.now();
    requestAnimationFrame(this.frame);
  }

  private fit(): void {
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(200, Math.floor(this.canvas.clientWidth * dpr));
    const h = Math.max(100, Math.floor(this.canvas.clientHeight * dpr));
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
      this.clear();
    }
  }

  private clear(): void {
    const g = this.canvas.getContext('2d');
    if (!g) return;
    g.fillStyle = '#000028';
    g.fillRect(0, 0, this.canvas.width, this.canvas.height);
  }

  private frame = (now: number): void => {
    const engine = this.engine;
    if (!engine) return;
    const g = this.canvas.getContext('2d');
    if (g) {
      const w = this.canvas.width;
      const h = this.canvas.height;
      const dpr = window.devicePixelRatio || 1;
      this.carry += ((now - this.last) / 1000) * PIXELS_PER_SECOND * dpr;
      this.last = now;
      const step = Math.floor(this.carry);
      if (step >= 1) {
        this.carry -= step;
        engine.spectrum(this.data);
        const usable = Math.floor((MAX_HZ / (engine.context.sampleRate / 2)) * this.data.length);
        g.drawImage(this.canvas, step, 0, w - step, h, 0, 0, w - step, h);
        const col = g.createImageData(step, h);
        const gridRows = new Set(LABEL_HZ.map((hz) => Math.min(h - 1, Math.round(h - 1 - (hz / MAX_HZ) * h))));
        for (let y = 0; y < h; y++) {
          const bin = Math.min(usable - 1, Math.floor(((h - 1 - y) / h) * usable));
          let [r, gr, b] = colour(this.data[bin]);
          if (gridRows.has(y)) {
            // Faint frequency grid, drawn into the new column only (never re-drawn over old ones).
            r = Math.min(255, r + 45);
            gr = Math.min(255, gr + 45);
            b = Math.min(255, b + 45);
          }
          for (let x = 0; x < step; x++) {
            const i = (y * step + x) * 4;
            col.data[i] = r;
            col.data[i + 1] = gr;
            col.data[i + 2] = b;
            col.data[i + 3] = 255;
          }
        }
        g.putImageData(col, w - step, 0);
      }
    }
    requestAnimationFrame(this.frame);
  };
}
