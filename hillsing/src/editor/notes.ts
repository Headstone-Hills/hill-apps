// Editable melody model for the piano-roll editor: immutable note operations plus an undo history.
// Times are seconds in the file's own tempo (the Tempo slider only changes playback speed), so
// editing never depends on the playback rate.

import type { MidiNote } from '../midi/smf';

export interface EditNote extends MidiNote {
  /** Stable identity across edits (selection, drag, lyric lane). */
  id: number;
}

export const MIN_NOTE_SEC = 0.05;
export const DEFAULT_NOTE_SEC = 0.25;
export const MIN_PITCH = 24; // C1
export const MAX_PITCH = 108; // C8

let nextId = 1;

export function withIds(notes: ReadonlyArray<MidiNote>): EditNote[] {
  return notes.map((n) => ({ ...n, id: nextId++ })).sort(byStart);
}

export const byStart = (a: MidiNote, b: MidiNote): number => a.start - b.start || a.note - b.note;

const clamp = (x: number, lo: number, hi: number): number => Math.min(hi, Math.max(lo, x));

/** Round `t` to the nearest multiple of `grid` seconds (no snapping when grid <= 0). */
export function snap(t: number, grid: number): number {
  return grid > 0 ? Math.round(t / grid) * grid : t;
}

export function makeNote(start: number, note: number, len = DEFAULT_NOTE_SEC, velocity = 0.8): EditNote {
  const s = Math.max(0, start);
  return { id: nextId++, start: s, end: s + Math.max(MIN_NOTE_SEC, len), note: clamp(Math.round(note), MIN_PITCH, MAX_PITCH), velocity, channel: 0 };
}

export function addNote(notes: ReadonlyArray<EditNote>, n: EditNote): EditNote[] {
  return [...notes, n].sort(byStart);
}

export function removeNotes(notes: ReadonlyArray<EditNote>, ids: ReadonlySet<number>): EditNote[] {
  return notes.filter((n) => !ids.has(n.id));
}

/**
 * Move a note: its start snaps to the grid, its length is kept. `orig` is the note as it was when
 * the drag began, so the move is always computed from there and does not accumulate rounding.
 */
export function movedNote(orig: EditNote, dSec: number, dPitch: number, grid: number): EditNote {
  const len = orig.end - orig.start;
  const start = Math.max(0, snap(orig.start + dSec, grid));
  return { ...orig, start, end: start + len, note: clamp(Math.round(orig.note + dPitch), MIN_PITCH, MAX_PITCH) };
}

/** Resize from the right edge: the end snaps to the grid and the note never gets shorter than MIN_NOTE_SEC. */
export function resizedNote(orig: EditNote, endSec: number, grid: number): EditNote {
  const end = Math.max(orig.start + MIN_NOTE_SEC, snap(endSec, grid));
  return { ...orig, end };
}

/** Resize from the left edge, keeping the end where it is. */
export function resizedNoteStart(orig: EditNote, startSec: number, grid: number): EditNote {
  const start = clamp(snap(startSec, grid), 0, orig.end - MIN_NOTE_SEC);
  return { ...orig, start };
}

export function replaceNote(notes: ReadonlyArray<EditNote>, n: EditNote): EditNote[] {
  return notes.map((x) => (x.id === n.id ? n : x)).sort(byStart);
}

export function transposeNotes(notes: ReadonlyArray<EditNote>, ids: ReadonlySet<number>, semis: number): EditNote[] {
  return notes.map((n) => (ids.has(n.id) ? { ...n, note: clamp(n.note + semis, MIN_PITCH, MAX_PITCH) } : n));
}

export function nudgeNotes(notes: ReadonlyArray<EditNote>, ids: ReadonlySet<number>, dSec: number): EditNote[] {
  return notes
    .map((n) => {
      if (!ids.has(n.id)) return n;
      const start = Math.max(0, n.start + dSec);
      return { ...n, start, end: start + (n.end - n.start) };
    })
    .sort(byStart);
}

/** Snap every selected note's start to the grid, keeping lengths (at least one grid step long). */
export function quantizeNotes(notes: ReadonlyArray<EditNote>, ids: ReadonlySet<number> | null, grid: number): EditNote[] {
  if (grid <= 0) return [...notes];
  return notes
    .map((n) => {
      if (ids && !ids.has(n.id)) return n;
      const start = Math.max(0, snap(n.start, grid));
      const len = Math.max(grid, snap(n.end - n.start, grid));
      return { ...n, start, end: start + len };
    })
    .sort(byStart);
}

/** Snapshots with undo/redo. Edits that change nothing (per `same`) are not recorded. */
export class History<T> {
  private past: T[] = [];
  private future: T[] = [];
  constructor(
    private cur: T,
    private readonly same: (a: T, b: T) => boolean,
  ) {}

  /** The current snapshot (callers may refresh fields of it, e.g. the lyric text, before pushing). */
  get current(): T {
    return this.cur;
  }
  get canUndo(): boolean {
    return this.past.length > 0;
  }
  get canRedo(): boolean {
    return this.future.length > 0;
  }

  push(next: T): boolean {
    if (this.same(this.cur, next)) return false;
    this.past.push(this.cur);
    if (this.past.length > 200) this.past.shift();
    this.cur = next;
    this.future = [];
    return true;
  }
  undo(): T | null {
    const prev = this.past.pop();
    if (prev === undefined) return null;
    this.future.push(this.cur);
    this.cur = prev;
    return prev;
  }
  redo(): T | null {
    const next = this.future.pop();
    if (next === undefined) return null;
    this.past.push(this.cur);
    this.cur = next;
    return next;
  }
  /** Start over with a new score (loading a file). */
  reset(initial: T): void {
    this.cur = initial;
    this.past = [];
    this.future = [];
  }
}

export function sameNotes(a: ReadonlyArray<EditNote>, b: ReadonlyArray<EditNote>): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i], y = b[i];
    if (x.id !== y.id || x.start !== y.start || x.end !== y.end || x.note !== y.note || x.velocity !== y.velocity) return false;
  }
  return true;
}
