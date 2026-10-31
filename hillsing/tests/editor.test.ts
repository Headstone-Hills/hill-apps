import { describe, expect, it } from 'vitest';
import {
  addNote, History, makeNote, MIN_NOTE_SEC, movedNote, nudgeNotes, quantizeNotes, removeNotes,
  resizedNote, resizedNoteStart, sameNotes, snap, transposeNotes, withIds, type EditNote,
} from '../src/editor/notes';
import { renameSyllable, spanForSyllable, toggleMelisma, tokenSpans } from '../src/editor/lyricEdit';
import { buildLyrics } from '../src/text/lyrics';
import { realLookup } from './helpers';

const lookup = realLookup();
const mk = (start: number, end: number, note: number) => ({ start, end, note, velocity: 0.8, channel: 0 });

describe('note operations', () => {
  it('snaps to the grid, or leaves the time alone when the grid is off', () => {
    expect(snap(0.37, 0.25)).toBeCloseTo(0.25);
    expect(snap(0.38, 0.25)).toBeCloseTo(0.5);
    expect(snap(0.37, 0)).toBe(0.37);
  });

  it('assigns unique ids and keeps notes sorted by start', () => {
    const n = withIds([mk(2, 3, 60), mk(0, 1, 62), mk(1, 2, 64)]);
    expect(n.map((x) => x.start)).toEqual([0, 1, 2]);
    expect(new Set(n.map((x) => x.id)).size).toBe(3);
  });

  it('moves a note keeping its length, from the original position, within the allowed range', () => {
    const [n] = withIds([mk(1, 1.5, 60)]);
    const m = movedNote(n, 0.3, 2, 0.25);
    expect(m.start).toBeCloseTo(1.25);
    expect(m.end - m.start).toBeCloseTo(0.5);
    expect(m.note).toBe(62);
    expect(movedNote(n, -5, 0, 0).start).toBe(0); // never before time 0
    expect(movedNote(n, 0, 1000, 0).note).toBeLessThanOrEqual(108);
    expect(movedNote(n, 0, -1000, 0).note).toBeGreaterThanOrEqual(24);
  });

  it('resizes from either edge without going shorter than the minimum', () => {
    const [n] = withIds([mk(1, 2, 60)]);
    expect(resizedNote(n, 3, 0).end).toBe(3);
    expect(resizedNote(n, 0, 0).end).toBeCloseTo(1 + MIN_NOTE_SEC);
    expect(resizedNoteStart(n, 0.5, 0).start).toBe(0.5);
    expect(resizedNoteStart(n, 5, 0).start).toBeCloseTo(2 - MIN_NOTE_SEC);
  });

  it('adds, removes, transposes and nudges only what it is asked to', () => {
    let notes = withIds([mk(0, 1, 60), mk(1, 2, 62)]);
    const added = makeNote(0.5, 64.4);
    notes = addNote(notes, added);
    expect(notes.map((n) => n.start)).toEqual([0, 0.5, 1]);
    expect(added.note).toBe(64);
    const sel = new Set([notes[0].id]);
    expect(transposeNotes(notes, sel, 12)[0].note).toBe(72);
    expect(transposeNotes(notes, sel, 12)[1].note).toBe(64);
    expect(nudgeNotes(notes, sel, 0.5)[0].start).toBe(0.5);
    expect(nudgeNotes(notes, sel, -5)[0].start).toBe(0);
    expect(removeNotes(notes, sel)).toHaveLength(2);
  });

  it('quantizes starts and lengths to the grid, only for the selection when there is one', () => {
    const notes = withIds([mk(0.1, 0.48, 60), mk(1.13, 1.4, 62)]);
    const q = quantizeNotes(notes, null, 0.25);
    expect(q[0].start).toBe(0);
    expect(q[0].end).toBeCloseTo(0.5);
    expect(q[1].start).toBeCloseTo(1.25);
    expect(q[1].end - q[1].start).toBeGreaterThanOrEqual(0.25);
    const only = quantizeNotes(notes, new Set([notes[1].id]), 0.25);
    expect(only[0].start).toBe(0.1);
  });
});

describe('undo history', () => {
  it('undoes and redoes, drops redo history on a new edit, and ignores no-op edits', () => {
    const a = withIds([mk(0, 1, 60)]);
    const h = new History<EditNote[]>(a, sameNotes);
    const b = addNote(a, makeNote(1, 62));
    expect(h.push(b)).toBe(true);
    expect(h.push(b)).toBe(false); // same content: nothing recorded
    expect(h.canUndo).toBe(true);
    expect(h.undo()).toBe(a);
    expect(h.canRedo).toBe(true);
    expect(h.redo()).toBe(b);
    h.undo();
    h.push(addNote(a, makeNote(3, 65)));
    expect(h.canRedo).toBe(false);
    h.reset(withIds([mk(0, 1, 60)]));
    expect(h.canUndo).toBe(false);
  });

  it('can carry more than notes: a snapshot with lyrics undoes both together', () => {
    type Snap = { notes: EditNote[]; lyrics: string };
    const same = (x: Snap, y: Snap) => sameNotes(x.notes, y.notes) && x.lyrics === y.lyrics;
    const notes = withIds([mk(0, 1, 60)]);
    const h = new History<Snap>({ notes, lyrics: 'a b' }, same);
    expect(h.push({ notes, lyrics: 'a ~ b' })).toBe(true);
    expect(h.push({ notes, lyrics: 'a ~ b' })).toBe(false);
    expect(h.undo()?.lyrics).toBe('a b');
    expect(h.redo()?.lyrics).toBe('a ~ b');
  });
});

describe('editing the lyric text', () => {
  const text = 'twinkle twinkle little star';

  it('maps tokens to the syllables buildLyrics produces', () => {
    const spans = tokenSpans(text, lookup);
    expect(spans.map((s) => s.syllables)).toEqual(buildLyrics(text, lookup).words.map((w) => w.count));
    expect(spans[1].firstSyllable).toBe(spans[0].syllables);
    expect(spanForSyllable(spans, 0)?.text).toBe('twinkle');
    expect(spanForSyllable(spans, 999)).toBeUndefined();
  });

  it('replaces the word that produces a syllable, and removes it when the replacement is empty', () => {
    const star = tokenSpans(text, lookup).find((s) => s.text === 'star')!;
    expect(renameSyllable(text, lookup, star.firstSyllable, 'moon')).toBe('twinkle twinkle little moon');
    expect(renameSyllable(text, lookup, star.firstSyllable, '  ')).toBe('twinkle twinkle little');
    expect(renameSyllable(text, lookup, 999, 'x')).toBeNull();
  });

  it('turns a note into a melisma by inserting ~ before its word, and back again', () => {
    const little = tokenSpans(text, lookup).find((s) => s.text === 'little')!;
    const withMel = toggleMelisma(text, lookup, little.firstSyllable)!;
    expect(withMel).toBe('twinkle twinkle ~ little star');
    // the ~ now owns the syllable index little used to have
    const spans = tokenSpans(withMel, lookup);
    expect(spans[2].melisma).toBe(true);
    expect(spans[2].firstSyllable).toBe(little.firstSyllable);
    expect(toggleMelisma(withMel, lookup, little.firstSyllable)).toBe(text);
  });

  it('refuses a melisma where it makes no sense', () => {
    expect(toggleMelisma(text, lookup, 0)).toBeNull(); // nothing to continue
    const twinkle = tokenSpans(text, lookup)[0];
    expect(twinkle.syllables).toBeGreaterThan(1);
    expect(toggleMelisma(text, lookup, twinkle.firstSyllable + 1)).toBeNull(); // middle of a word
  });

  it('counts a leading ~ as no syllable', () => {
    expect(tokenSpans('~ hello', lookup)[0].syllables).toBe(0);
  });
});
