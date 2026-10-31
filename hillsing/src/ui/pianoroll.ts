// Piano-roll editor canvas: pitch rows, a time ruler, editable notes and a syllable lane under them.
//
// The canvas only covers the visible window (scrollSec .. scrollSec + width/pps), so a long song
// never needs a canvas wider than the screen. All editing happens on a working copy of the notes while
// the pointer is down; the change is committed (and the score rebuilt) once on pointer-up.

import {
  addNote, DEFAULT_NOTE_SEC, makeNote, movedNote, nudgeNotes, quantizeNotes, removeNotes, replaceNote, resizedNote,
  resizedNoteStart, snap, transposeNotes, type EditNote,
} from '../editor/notes';

export interface LaneCell {
  label: string;
  /** False when this note has no syllable (more notes than lyrics, or a chord's hidden notes). */
  sung: boolean;
  melisma: boolean;
}

export interface RollView {
  notes: EditNote[];
  /** Lyric lane content by note id. */
  lane: Map<number, LaneCell>;
  /** Playhead position in file seconds, or null to hide it. */
  playheadSec: number | null;
  playing: boolean;
  /** Length of the score in file seconds (scroll range). */
  lengthSec: number;
}

export interface RollHooks {
  commit(notes: EditNote[]): void;
  seek(fileSec: number): void;
  toggleMelisma(noteId: number): void;
  renameSyllable(noteId: number, text: string): void;
  undo(): void;
  redo(): void;
  /** Selection changed (for enabling toolbar buttons). */
  selectionChanged(ids: ReadonlySet<number>): void;
}

const RULER_H = 22;
const LANE_H = 28;
const GUTTER_W = 42;
const EDGE_PX = 6;
const MIN_PPS = 20;
const MAX_PPS = 800;

const COLORS = {
  bg: '#14171c',
  rowAlt: '#181c22',
  rowBlack: '#10131800',
  grid: '#232a33',
  beat: '#2d343e',
  ruler: '#1c2128',
  text: '#8c96a3',
  note: '#6cb6ff',
  melisma: '#8fe0b0',
  unsung: '#5a6470',
  selected: '#f0b429',
  label: '#0b1620',
  laneBg: '#1a1f26',
  error: '#e5736b',
  playhead: '#f0b429',
};

const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
const isBlack = (n: number): boolean => [1, 3, 6, 8, 10].includes(n % 12);
export const noteLabel = (n: number): string => `${NAMES[n % 12]}${Math.floor(n / 12) - 1}`;

type Drag =
  | { kind: 'seek' }
  | { kind: 'move' | 'right' | 'left'; id: number; x0: number; y0: number; orig: Map<number, EditNote>; rowH: number; changed: boolean };

export class PianoRoll {
  private view: RollView = { notes: [], lane: new Map(), playheadSec: null, playing: false, lengthSec: 4 };
  private pps = 120;
  private scrollSec = 0;
  private grid = 0;
  private selection = new Set<number>();
  private drag: Drag | null = null;
  private lo = 48;
  private hi = 72;
  private input: HTMLInputElement | null = null;

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly scrollbar: HTMLInputElement,
    private readonly wrap: HTMLElement,
    private readonly hooks: RollHooks,
  ) {
    canvas.tabIndex = 0;
    canvas.addEventListener('pointerdown', (e) => this.onDown(e));
    canvas.addEventListener('pointermove', (e) => this.onMove(e));
    canvas.addEventListener('pointerup', (e) => this.onUp(e));
    canvas.addEventListener('pointercancel', (e) => this.onUp(e));
    canvas.addEventListener('dblclick', (e) => this.onDouble(e));
    canvas.addEventListener('wheel', (e) => this.onWheel(e), { passive: false });
    canvas.addEventListener('keydown', (e) => this.onKey(e));
    scrollbar.addEventListener('input', () => {
      this.scrollSec = Number(scrollbar.value);
      this.render();
    });
    window.addEventListener('resize', () => this.render());
  }

  // ---- public API ---------------------------------------------------------------------
  setView(view: RollView): void {
    this.view = view;
    // Drop selection of notes that no longer exist.
    const ids = new Set(view.notes.map((n) => n.id));
    let changed = false;
    for (const id of [...this.selection]) if (!ids.has(id)) { this.selection.delete(id); changed = true; }
    if (changed) this.hooks.selectionChanged(this.selection);
    if (view.playing && view.playheadSec !== null && !this.drag) this.follow(view.playheadSec);
    this.render();
  }

  setGrid(sec: number): void {
    this.grid = sec;
    this.render();
  }

  get selectedIds(): ReadonlySet<number> {
    return this.selection;
  }

  select(ids: Iterable<number>): void {
    this.selection = new Set(ids);
    this.hooks.selectionChanged(this.selection);
    this.render();
  }

  zoomBy(factor: number, anchorX = this.width / 2): void {
    const t = this.timeAt(anchorX);
    this.pps = Math.min(MAX_PPS, Math.max(MIN_PPS, this.pps * factor));
    this.scrollSec = Math.max(0, t - (anchorX - GUTTER_W) / this.pps);
    this.render();
  }

  /** Zoom so the whole score fits the window. */
  fit(): void {
    const span = Math.max(1, this.view.lengthSec);
    this.pps = Math.min(MAX_PPS, Math.max(MIN_PPS, (this.width - GUTTER_W - 8) / span));
    this.scrollSec = 0;
    this.render();
  }

  /** Scroll so a position is visible (used when a note is added or the playhead leaves the window). */
  reveal(sec: number): void {
    if (sec < this.scrollSec || sec > this.scrollSec + this.visibleSec) this.scrollSec = Math.max(0, sec - this.visibleSec * 0.2);
    this.render();
  }

  deleteSelected(): void {
    if (this.selection.size === 0) return;
    this.hooks.commit(removeNotes(this.view.notes, this.selection));
    this.select([]);
  }
  transposeSelected(semis: number): void {
    if (this.selection.size) this.hooks.commit(transposeNotes(this.view.notes, this.selection, semis));
  }
  nudgeSelected(dSec: number): void {
    if (this.selection.size) this.hooks.commit(nudgeNotes(this.view.notes, this.selection, dSec));
  }
  quantize(): void {
    if (this.grid <= 0) return;
    this.hooks.commit(quantizeNotes(this.view.notes, this.selection.size ? this.selection : null, this.grid));
  }
  toggleMelismaOfSelection(): void {
    if (this.selection.size === 1) this.hooks.toggleMelisma([...this.selection][0]);
  }

  // ---- geometry ------------------------------------------------------------------------
  private get width(): number {
    return this.canvas.clientWidth || 600;
  }
  private get height(): number {
    return this.canvas.clientHeight || 300;
  }
  private get visibleSec(): number {
    return (this.width - GUTTER_W) / this.pps;
  }
  private get rollTop(): number {
    return RULER_H;
  }
  private get rollBottom(): number {
    return this.height - LANE_H;
  }
  private get rowH(): number {
    return (this.rollBottom - this.rollTop) / (this.hi - this.lo + 1);
  }
  private xOf(t: number): number {
    return GUTTER_W + (t - this.scrollSec) * this.pps;
  }
  private timeAt(x: number): number {
    return this.scrollSec + (x - GUTTER_W) / this.pps;
  }
  private yOf(note: number): number {
    return this.rollTop + (this.hi - note) * this.rowH;
  }
  private pitchAt(y: number): number {
    return this.hi - Math.floor((y - this.rollTop) / this.rowH);
  }

  /** Visible pitch range follows the notes (padded), and stays fixed while a drag is in progress. */
  private fitPitchRange(): void {
    if (this.drag && this.drag.kind !== 'seek') return;
    const ns = this.view.notes;
    let lo = 60, hi = 72;
    if (ns.length) {
      lo = Math.min(...ns.map((n) => n.note));
      hi = Math.max(...ns.map((n) => n.note));
    }
    lo -= 3;
    hi += 3;
    while (hi - lo < 23) {
      lo--;
      if (hi - lo < 23) hi++;
    }
    this.lo = Math.max(0, lo);
    this.hi = Math.min(127, hi);
  }

  private follow(sec: number): void {
    const margin = 40 / this.pps;
    if (sec > this.scrollSec + this.visibleSec - margin || sec < this.scrollSec) {
      this.scrollSec = Math.max(0, sec - this.visibleSec * 0.15);
    }
  }

  // ---- hit testing -------------------------------------------------------------------------
  private hit(x: number, y: number): { note: EditNote; zone: 'body' | 'left' | 'right' } | null {
    const inLane = y >= this.rollBottom;
    const t = this.timeAt(x);
    let best: { note: EditNote; zone: 'body' | 'left' | 'right' } | null = null;
    for (const n of this.view.notes) {
      if (inLane) {
        if (t >= n.start && t <= n.end) best = { note: n, zone: 'body' };
        continue;
      }
      const x0 = this.xOf(n.start), x1 = this.xOf(n.end);
      const y0 = this.yOf(n.note);
      if (y < y0 || y > y0 + this.rowH || x < x0 - 2 || x > x1 + 2) continue;
      const w = x1 - x0;
      const edge = Math.min(EDGE_PX, w / 3);
      best = { note: n, zone: x >= x1 - edge ? 'right' : x <= x0 + edge ? 'left' : 'body' };
    }
    return best;
  }

  private local(e: MouseEvent): { x: number; y: number } {
    const r = this.canvas.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  // ---- pointer ---------------------------------------------------------------------------------
  private onDown(e: PointerEvent): void {
    this.closeInput();
    this.canvas.focus();
    const { x, y } = this.local(e);
    this.canvas.setPointerCapture(e.pointerId);
    if (y < RULER_H || x < GUTTER_W) {
      this.drag = { kind: 'seek' };
      this.hooks.seek(Math.max(0, this.timeAt(x)));
      return;
    }
    const h = this.hit(x, y);
    if (!h) {
      if (!e.shiftKey) this.select([]);
      this.drag = { kind: 'seek' }; // clicking empty space moves the playhead
      this.hooks.seek(Math.max(0, this.timeAt(x)));
      return;
    }
    const id = h.note.id;
    if (e.shiftKey) {
      const next = new Set(this.selection);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      this.select(next);
    } else if (!this.selection.has(id)) {
      this.select([id]);
    }
    if (y >= this.rollBottom) return; // the syllable lane selects but does not drag
    const group = h.zone === 'body' ? this.selection : new Set([id]);
    const orig = new Map<number, EditNote>();
    for (const n of this.view.notes) if (group.has(n.id)) orig.set(n.id, n);
    this.drag = { kind: h.zone === 'body' ? 'move' : h.zone, id, x0: x, y0: y, orig, rowH: this.rowH, changed: false };
  }

  private onMove(e: PointerEvent): void {
    const { x, y } = this.local(e);
    const d = this.drag;
    if (!d) {
      const h = y >= RULER_H ? this.hit(x, y) : null;
      this.canvas.style.cursor = !h ? 'default' : h.zone === 'body' ? 'grab' : 'ew-resize';
      return;
    }
    if (d.kind === 'seek') {
      if (e.buttons) this.hooks.seek(Math.max(0, this.timeAt(x)));
      return;
    }
    const dt = (x - d.x0) / this.pps;
    const dp = -Math.round((y - d.y0) / d.rowH);
    let notes = this.view.notes;
    if (d.kind === 'move') {
      const primary = d.orig.get(d.id)!;
      const snapped = movedNote(primary, dt, 0, this.grid).start - primary.start; // snap by the grabbed note
      for (const o of d.orig.values()) notes = replaceNote(notes, movedNote(o, snapped, dp, 0));
    } else if (d.kind === 'right') {
      const o = d.orig.get(d.id)!;
      notes = replaceNote(notes, resizedNote(o, o.end + dt, this.grid));
    } else {
      const o = d.orig.get(d.id)!;
      notes = replaceNote(notes, resizedNoteStart(o, o.start + dt, this.grid));
    }
    d.changed = true;
    this.view = { ...this.view, notes };
    this.render();
  }

  private onUp(e: PointerEvent): void {
    const d = this.drag;
    this.drag = null;
    if (this.canvas.hasPointerCapture(e.pointerId)) this.canvas.releasePointerCapture(e.pointerId);
    if (d && d.kind !== 'seek' && d.changed) this.hooks.commit(this.view.notes);
  }

  private onDouble(e: MouseEvent): void {
    const { x, y } = this.local(e);
    if (y < RULER_H || x < GUTTER_W) return;
    const h = this.hit(x, y);
    if (y >= this.rollBottom) {
      if (h) this.openInput(h.note);
      return;
    }
    if (h) return;
    const start = Math.max(0, snap(this.timeAt(x), this.grid));
    const note = makeNote(start, this.pitchAt(y), Math.max(this.grid, DEFAULT_NOTE_SEC)); // at least a quarter second: long enough to read the word
    this.hooks.commit(addNote(this.view.notes, note));
    this.select([note.id]);
  }

  private onWheel(e: WheelEvent): void {
    e.preventDefault();
    if (e.ctrlKey || e.metaKey) {
      const { x } = this.local(e);
      this.zoomBy(Math.exp(-e.deltaY * 0.01), x);
      return;
    }
    const delta = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
    this.scrollSec = Math.max(0, this.scrollSec + delta / this.pps);
    this.render();
  }

  // ---- keyboard ----------------------------------------------------------------------------------
  private onKey(e: KeyboardEvent): void {
    const mod = e.metaKey || e.ctrlKey;
    let handled = true;
    if (mod && e.code === 'KeyZ') (e.shiftKey ? this.hooks.redo() : this.hooks.undo());
    else if (mod && e.code === 'KeyY') this.hooks.redo();
    else if (mod && e.code === 'KeyA') this.select(this.view.notes.map((n) => n.id));
    else if (e.code === 'Delete' || e.code === 'Backspace') this.deleteSelected();
    else if (e.code === 'ArrowUp') this.transposeSelected(e.shiftKey ? 12 : 1);
    else if (e.code === 'ArrowDown') this.transposeSelected(e.shiftKey ? -12 : -1);
    else if (e.code === 'ArrowLeft') this.nudgeSelected(-(this.grid > 0 ? this.grid : 0.05) * (e.shiftKey ? 4 : 1));
    else if (e.code === 'ArrowRight') this.nudgeSelected((this.grid > 0 ? this.grid : 0.05) * (e.shiftKey ? 4 : 1));
    else if (e.code === 'KeyQ' && !mod) this.quantize();
    else if (e.code === 'KeyM' && !mod) this.toggleMelismaOfSelection();
    else if (e.code === 'Equal' || e.code === 'NumpadAdd') this.zoomBy(1.25);
    else if (e.code === 'Minus' || e.code === 'NumpadSubtract') this.zoomBy(0.8);
    else if (e.code === 'Escape') this.select([]);
    else handled = false;
    if (handled) {
      e.preventDefault();
      e.stopPropagation(); // keep editing keys from reaching the page's play-by-keyboard handler
    }
  }

  // ---- inline syllable editing ---------------------------------------------------------------------
  private openInput(n: EditNote): void {
    this.closeInput();
    const cell = this.view.lane.get(n.id);
    const input = document.createElement('input');
    input.className = 'roll-input';
    input.type = 'text';
    input.value = cell?.sung ? (cell.melisma ? '~' : cell.label) : '';
    input.placeholder = 'word';
    input.setAttribute('aria-label', 'Word for this note');
    const x = this.xOf(n.start);
    const w = Math.max(90, this.xOf(n.end) - x);
    input.style.left = `${Math.max(GUTTER_W, Math.min(x, this.width - w))}px`;
    input.style.top = `${this.rollBottom}px`;
    input.style.width = `${w}px`;
    input.style.height = `${LANE_H}px`;
    const finish = (apply: boolean): void => {
      if (this.input !== input) return;
      this.input = null;
      const v = input.value;
      input.remove();
      this.canvas.focus();
      if (apply) this.hooks.renameSyllable(n.id, v);
    };
    input.addEventListener('keydown', (e) => {
      e.stopPropagation();
      if (e.code === 'Enter') finish(true);
      else if (e.code === 'Escape') finish(false);
    });
    input.addEventListener('blur', () => finish(false));
    this.wrap.append(input);
    this.input = input;
    input.focus();
    input.select();
  }

  private closeInput(): void {
    this.input?.remove();
    this.input = null;
  }

  // ---- drawing --------------------------------------------------------------------------------------
  render(): void {
    const dpr = window.devicePixelRatio || 1;
    const w = this.width, h = this.height;
    const pw = Math.max(1, Math.floor(w * dpr)), ph = Math.max(1, Math.floor(h * dpr));
    if (this.canvas.width !== pw || this.canvas.height !== ph) {
      this.canvas.width = pw;
      this.canvas.height = ph;
    }
    const g = this.canvas.getContext('2d');
    if (!g) return;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.fitPitchRange();

    // Keep the scroll bar in step with the score length.
    const maxScroll = Math.max(0, this.view.lengthSec + 1 - this.visibleSec);
    this.scrollSec = Math.min(Math.max(0, this.scrollSec), maxScroll);
    this.scrollbar.max = String(maxScroll);
    this.scrollbar.step = 'any';
    this.scrollbar.value = String(this.scrollSec);
    this.scrollbar.disabled = maxScroll <= 0;

    g.fillStyle = COLORS.bg;
    g.fillRect(0, 0, w, h);
    const rowH = this.rowH;

    // pitch rows
    g.font = '10px system-ui, sans-serif';
    g.textBaseline = 'middle';
    for (let n = this.lo; n <= this.hi; n++) {
      const y = this.yOf(n);
      g.fillStyle = isBlack(n) ? COLORS.rowAlt : COLORS.bg;
      g.fillRect(GUTTER_W, y, w - GUTTER_W, rowH);
      if (n % 12 === 0) {
        g.fillStyle = COLORS.grid;
        g.fillRect(GUTTER_W, y + rowH - 1, w - GUTTER_W, 1);
      }
    }
    // time grid
    const gridStep = this.grid > 0 && this.grid * this.pps >= 8 ? this.grid : 0;
    const t0 = this.scrollSec, t1 = this.scrollSec + this.visibleSec;
    if (gridStep) {
      g.fillStyle = COLORS.grid;
      for (let t = Math.ceil(t0 / gridStep) * gridStep; t <= t1; t += gridStep) g.fillRect(this.xOf(t), this.rollTop, 1, this.rollBottom - this.rollTop);
    }
    // ruler
    g.fillStyle = COLORS.ruler;
    g.fillRect(0, 0, w, RULER_H);
    const steps = [0.1, 0.25, 0.5, 1, 2, 5, 10, 30, 60];
    const step = steps.find((s) => s * this.pps >= 70) ?? 60;
    g.fillStyle = COLORS.text;
    g.textBaseline = 'middle';
    for (let t = Math.ceil(t0 / step) * step; t <= t1; t += step) {
      const x = this.xOf(t);
      g.fillStyle = COLORS.beat;
      g.fillRect(x, RULER_H - 6, 1, 6);
      g.fillRect(x, this.rollTop, 1, this.rollBottom - this.rollTop);
      g.fillStyle = COLORS.text;
      const m = Math.floor(t / 60);
      g.fillText(`${m}:${(t - m * 60).toFixed(step < 1 ? 2 : 0).padStart(step < 1 ? 5 : 2, '0')}`, x + 3, RULER_H / 2 - 1);
    }
    // key gutter (fixed at the left)
    g.fillStyle = COLORS.ruler;
    g.fillRect(0, this.rollTop, GUTTER_W, this.rollBottom - this.rollTop);
    for (let n = this.lo; n <= this.hi; n++) {
      const y = this.yOf(n);
      if (isBlack(n)) {
        g.fillStyle = '#0d1014';
        g.fillRect(0, y, GUTTER_W - 10, rowH);
      }
      if (n % 12 === 0 || rowH >= 12) {
        g.fillStyle = COLORS.text;
        g.fillText(noteLabel(n), 4, y + rowH / 2);
      }
    }
    // lane background
    g.fillStyle = COLORS.laneBg;
    g.fillRect(0, this.rollBottom, w, LANE_H);
    g.fillStyle = COLORS.text;
    g.fillText('words', 4, this.rollBottom + LANE_H / 2);

    // notes + lane cells (clipped to the window area right of the gutter)
    g.save();
    g.beginPath();
    g.rect(GUTTER_W, 0, w - GUTTER_W, h);
    g.clip();
    g.font = `${Math.max(9, Math.min(12, rowH - 2))}px system-ui, sans-serif`;
    for (const n of this.view.notes) {
      const x0 = this.xOf(n.start), x1 = this.xOf(n.end);
      if (x1 < GUTTER_W || x0 > w) continue;
      const cell = this.view.lane.get(n.id);
      const sel = this.selection.has(n.id);
      const nw = Math.max(3, x1 - x0 - 1);
      const y = this.yOf(n.note);
      g.fillStyle = !cell || !cell.sung ? COLORS.unsung : cell.melisma ? COLORS.melisma : COLORS.note;
      g.fillRect(x0, y + 1, nw, Math.max(3, rowH - 2));
      if (sel) {
        g.strokeStyle = COLORS.selected;
        g.lineWidth = 2;
        g.strokeRect(x0 + 1, y + 2, Math.max(1, nw - 2), Math.max(1, rowH - 4));
      }
      const label = cell?.sung ? cell.label : '';
      g.textBaseline = 'middle';
      if (label && g.measureText(label).width + 6 < nw) {
        g.fillStyle = COLORS.label;
        g.fillText(label, x0 + 4, y + rowH / 2 + 1);
      }
      // lane cell
      g.fillStyle = sel ? '#2a2f38' : COLORS.laneBg;
      g.fillRect(x0, this.rollBottom + 2, nw, LANE_H - 4);
      g.strokeStyle = sel ? COLORS.selected : COLORS.beat;
      g.lineWidth = 1;
      g.strokeRect(x0 + 0.5, this.rollBottom + 2.5, nw - 1, LANE_H - 5);
      g.font = '12px system-ui, sans-serif';
      const txt = !cell ? '' : cell.sung ? cell.label : '—';
      g.fillStyle = !cell || !cell.sung ? COLORS.error : cell.melisma ? COLORS.melisma : '#e6e9ee';
      if (txt) {
        const tw = g.measureText(txt).width;
        if (tw + 6 <= nw) g.fillText(txt, x0 + 4, this.rollBottom + LANE_H / 2);
        else if (nw > 12) g.fillText('…', x0 + 3, this.rollBottom + LANE_H / 2);
      }
      g.font = `${Math.max(9, Math.min(12, rowH - 2))}px system-ui, sans-serif`;
    }
    // empty state
    if (this.view.notes.length === 0) {
      g.fillStyle = COLORS.text;
      g.font = '13px system-ui, sans-serif';
      g.fillText('Load a MIDI file, or double-click here to draw notes.', GUTTER_W + 16, (this.rollTop + this.rollBottom) / 2);
    }
    // playhead
    if (this.view.playheadSec !== null) {
      const px = this.xOf(this.view.playheadSec);
      if (px >= GUTTER_W && px <= w) {
        g.fillStyle = COLORS.playhead;
        g.fillRect(px, 0, 1.5, h);
      }
    }
    g.restore();
  }
}
