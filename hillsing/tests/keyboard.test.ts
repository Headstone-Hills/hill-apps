import { describe, expect, it } from 'vitest';
import { noteLabel, pianoLayout } from '../src/ui/keyboard';

describe('on-screen piano layout', () => {
  it('lays out white keys from a C, with black keys after C, D, F, G and A only', () => {
    const keys = pianoLayout(8, 60); // C4 up to C5
    const whites = keys.filter((k) => !k.black).map((k) => k.note);
    expect(whites).toEqual([60, 62, 64, 65, 67, 69, 71, 72]);
    const blacks = keys.filter((k) => k.black).map((k) => k.note);
    expect(blacks).toEqual([61, 63, 66, 68, 70]); // none between E-F or B-C, none after the last white key
  });

  it('puts every key inside the keyboard, with black keys centred on the line between two white ones', () => {
    for (const k of pianoLayout(15, 48)) {
      expect(k.left).toBeGreaterThanOrEqual(0);
      expect(k.left + k.width).toBeLessThanOrEqual(1 + 1e-9);
    }
    const keys = pianoLayout(8, 60);
    const cSharp = keys.find((k) => k.note === 61)!;
    expect(cSharp.left + cSharp.width / 2).toBeCloseTo(1 / 8, 6);
  });

  it('names notes the usual way (middle C is C4)', () => {
    expect(noteLabel(60)).toBe('C4');
    expect(noteLabel(69)).toBe('A4');
    expect(noteLabel(61)).toBe('C#4');
  });
});
