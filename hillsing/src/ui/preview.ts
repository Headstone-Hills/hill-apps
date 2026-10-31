import type { SungNote } from '../engine/sequence';

export interface PreviewData {
  notes: SungNote[];
  /** Label per note (same order), usually the syllable text. */
  labels: string[];
  /** Score-time length shown across the full width. */
  lengthSec: number;
}

const COLORS = {
  bg: '#14171c',
  grid: '#232a33',
  note: '#6cb6ff',
  legato: '#8fe0b0',
  label: '#0b1620',
  playhead: '#f0b429',
  empty: '#8c96a3',
};

/** Read-only piano-roll preview with syllable labels and a playhead. */
export function drawPreview(canvas: HTMLCanvasElement, data: PreviewData | null, playheadSec: number): void {
  const dpr = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.floor(canvas.clientWidth * dpr));
  const h = Math.max(1, Math.floor(canvas.clientHeight * dpr));
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  const g = canvas.getContext('2d');
  if (!g) return;
  g.fillStyle = COLORS.bg;
  g.fillRect(0, 0, w, h);

  if (!data || data.notes.length === 0) {
    g.fillStyle = COLORS.empty;
    g.font = `${12 * dpr}px system-ui, sans-serif`;
    g.fillText('Load a MIDI file to preview the melody and its syllables.', 12 * dpr, h / 2);
    return;
  }

  const lo = Math.min(...data.notes.map((n) => n.note)) - 1;
  const hi = Math.max(...data.notes.map((n) => n.note)) + 1;
  const rowH = h / (hi - lo + 1);
  const x = (t: number): number => (t / data.lengthSec) * w;

  g.strokeStyle = COLORS.grid;
  g.lineWidth = 1;
  for (let n = lo; n <= hi; n++) {
    if (n % 12 !== 0) continue; // faint line at every C
    const y = h - (n - lo + 0.5) * rowH;
    g.beginPath();
    g.moveTo(0, y);
    g.lineTo(w, y);
    g.stroke();
  }

  g.font = `${Math.min(12, Math.max(9, rowH / dpr)) * dpr}px system-ui, sans-serif`;
  g.textBaseline = 'middle';
  data.notes.forEach((n, i) => {
    const nx = x(n.start);
    const nw = Math.max(2 * dpr, x(n.end) - nx - 1);
    const ny = h - (n.note - lo + 1) * rowH;
    g.fillStyle = n.legato ? COLORS.legato : COLORS.note;
    g.fillRect(nx, ny + 1, nw, Math.max(3, rowH - 2));
    const label = data.labels[i] ?? '';
    if (label && g.measureText(label).width + 4 * dpr < nw) {
      g.fillStyle = COLORS.label;
      g.fillText(label, nx + 3 * dpr, ny + rowH / 2 + 1);
    }
  });

  const px = Math.min(w, Math.max(0, x(playheadSec)));
  g.fillStyle = COLORS.playhead;
  g.fillRect(px, 0, Math.max(1, dpr), h);
}

/** Seconds (score time) for a click at `clientX` on the canvas. */
export function timeAtClick(canvas: HTMLCanvasElement, clientX: number, lengthSec: number): number {
  const r = canvas.getBoundingClientRect();
  return Math.max(0, Math.min(1, (clientX - r.left) / r.width)) * lengthSec;
}
