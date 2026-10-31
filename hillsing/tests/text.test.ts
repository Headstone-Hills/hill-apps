import { describe, expect, it } from 'vitest';
import { guessPhonemes } from '../src/text/g2p';
import { buildLyrics, splitSpelling, tokenize } from '../src/text/lyrics';
import { LyricPointer } from '../src/text/pointer';
import { syllabify } from '../src/text/syllabify';
import { makeLookup } from '../src/text/cmudict';
import { realDict, realLookup } from './helpers';

const syl = (word: string) =>
  syllabify(realDict().get(word)!).map((s) => [...s.onset, s.nucleus, ...s.coda].join(' '));

describe('syllabify (real CMUdict entries)', () => {
  it('splits simple words', () => {
    expect(syl('hello')).toEqual(['HH AH0', 'L OW1']);
    expect(syl('string')).toEqual(['S T R IH1 NG']);
    expect(syl('computer')).toEqual(['K AH0 M', 'P Y UW1', 'T ER0']);
  });
  it('uses maximal onset for tense vowels and stress-sensitive coda for lax vowels', () => {
    expect(syl('paper')).toEqual(['P EY1', 'P ER0']);
    expect(syl('lemon')).toEqual(['L EH1 M', 'AH0 N']);
    expect(syl('little')).toEqual(['L IH1 T', 'AH0 L']);
  });
  it('keeps illegal onset clusters split across syllables', () => {
    expect(syl('wonder')).toEqual(['W AH1 N', 'D ER0']);
    expect(syl('twinkle')).toEqual(['T W IH1 NG', 'K AH0 L']);
  });
  it('never leaves NG, or HH/W/Y after a stressed lax vowel, in the wrong place', () => {
    expect(syl('ahead')).toEqual(['AH0', 'HH EH1 D']);
  });
  it('syllabifies every dictionary word: vowel nuclei, phonemes conserved in order', () => {
    let vowelless = 0;
    for (const [word, phones] of realDict()) {
      const s = syllabify(phones);
      expect(s.length, word).toBeGreaterThan(0);
      expect(s.every((x) => /\d$/.test(x.nucleus)), word).toBe(true);
      const flat = s.flatMap((x) => [...x.onset, x.nucleus, ...x.coda]);
      if (phones.some((p) => /\d$/.test(p))) {
        expect(s.length, word).toBe(phones.filter((p) => /\d$/.test(p)).length);
        expect(flat, word).toEqual(phones);
      } else {
        vowelless++; // e.g. "hmm": one syllable on a default schwa
        expect(s.length, word).toBe(1);
        expect(flat.slice(0, phones.length), word).toEqual(phones);
      }
    }
    expect(vowelless).toBeLessThan(50);
  });
  it('conserves phoneme order for words with vowels', () => {
    for (const w of ['banana', 'extraordinary', 'strengths', 'international']) {
      const phones = realDict().get(w)!;
      expect(syllabify(phones).flatMap((x) => [...x.onset, x.nucleus, ...x.coda])).toEqual(phones);
    }
  });
});

describe('tokenize / lookup / g2p', () => {
  it('handles apostrophes, curly quotes, digits and punctuation', () => {
    expect(tokenize("Don't stop—it's 42!")).toEqual(['Don\'t', 'stop', "it's", 'four', 'two']);
    expect(tokenize('rock’n’roll')).toEqual(["rock'n'roll"]);
  });
  it('builds possessives from the base word', () => {
    const lookup = makeLookup(realDict());
    expect(lookup("dog's")).toEqual([...realDict().get('dog')!, 'Z']);
    expect(lookup("cat's")?.slice(-1)).toEqual(['S']);
    expect(lookup("church's")?.slice(-2)).toEqual(['AH0', 'Z']);
    expect(lookup('zzzzqx')).toBeUndefined();
  });
  it('guesses a pronunciation for any word, always with a vowel', () => {
    for (const w of ['zorblax', 'xyz', 'brrr', 'qat', 'knight', 'make', 'bike', 'a', "o'er"]) {
      const p = guessPhonemes(w);
      expect(p.some((x) => /\d$/.test(x)), w).toBe(true);
    }
    expect(guessPhonemes('make')).toEqual(['M', 'EY1', 'K']);
  });
  it('flags out-of-dictionary words in lyrics', () => {
    const l = buildLyrics('hello zorblax', realLookup());
    expect(l.words.map((w) => w.inDict)).toEqual([true, false]);
    expect(l.syllables.length).toBeGreaterThanOrEqual(3);
  });
  it('splits spelling into labels that rejoin to the word', () => {
    for (const w of ['hello', 'banana', 'twinkle', 'extraordinary', 'a']) {
      const l = buildLyrics(w, realLookup());
      expect(l.syllables.map((s) => s.label).join('')).toBe(w);
    }
    expect(splitSpelling('hello', [2, 2])).toEqual(['hel', 'lo']);
  });
});

describe('LyricPointer', () => {
  const make = () => {
    const p = new LyricPointer();
    p.setLyrics(buildLyrics('hello big world', realLookup())); // hel-lo | big | world => 4 syllables? (hello=2)
    return p;
  };
  it('advances one syllable per call and goes silent at the end', () => {
    const p = make();
    expect(p.length).toBe(4);
    expect(p.advance()?.wordIndex).toBe(0);
    expect(p.advance()?.wordIndex).toBe(0);
    expect(p.advance()?.wordIndex).toBe(1);
    expect(p.advance()?.wordIndex).toBe(2);
    expect(p.atEnd).toBe(true);
    expect(p.advance()).toBeNull();
  });
  it('wraps when enabled', () => {
    const p = make();
    p.wrap = true;
    for (let i = 0; i < 4; i++) p.advance();
    expect(p.index).toBe(0);
  });
  it('steps back/forward and clamps', () => {
    const p = make();
    p.back();
    expect(p.index).toBe(0);
    p.forward(); p.forward();
    expect(p.index).toBe(2);
    p.setIndex(99);
    expect(p.index).toBe(4);
    p.forward();
    expect(p.index).toBe(4);
  });
  it('moves by word, handling mid-word and end of text', () => {
    const p = make();
    p.setIndex(1); // mid "hello"
    p.nextWord();
    expect(p.index).toBe(2);
    p.setIndex(1);
    p.prevWord(); // back to start of current word
    expect(p.index).toBe(0);
    p.setIndex(3);
    p.prevWord(); // at start of 'world' -> start of 'big'
    expect(p.index).toBe(2);
    p.setIndex(3);
    p.nextWord(); // last word -> end
    expect(p.atEnd).toBe(true);
    p.prevWord(); // from end -> start of last word
    expect(p.index).toBe(3);
  });
  it('is safe with empty lyrics', () => {
    const p = new LyricPointer();
    p.setLyrics(buildLyrics('', realLookup()));
    p.nextWord(); p.prevWord(); p.back(); p.forward();
    expect(p.advance()).toBeNull();
  });
});

describe('melisma token', () => {
  const lyr = (t: string) => buildLyrics(t, realLookup());
  it('tokenizes ~ and _ as markers', () => {
    expect(tokenize('love ~ ~ you_')).toEqual(['love', '~', '~', 'you', '_']);
  });
  it('adds a continuation note per marker on the same vowel, no new onset', () => {
    const l = lyr('love ~ ~');
    expect(l.syllables).toHaveLength(3);
    expect(l.syllables.map((x) => x.melisma ?? false)).toEqual([false, true, true]);
    expect(l.syllables.every((x) => x.nucleus === 'AH1')).toBe(true);
    expect(l.syllables[1].onset).toEqual([]);
    expect(l.words).toHaveLength(1);
    expect(l.words[0].count).toBe(3);
  });
  it('moves the final consonants to the last note of the melisma', () => {
    const l = lyr('hold ~ ~'); // HH OW1 L D
    expect(l.syllables.map((x) => x.coda)).toEqual([[], [], ['L', 'D']]);
    expect(l.syllables[0].onset).toEqual(['HH']);
  });
  it('marks every note except the last as "hold first vowel" (for diphthongs)', () => {
    const l = lyr('fly ~ ~');
    expect(l.syllables.map((x) => x.holdFirst ?? false)).toEqual([true, true, false]);
  });
  it('works mid-word and across words; stray or leading markers are ignored', () => {
    expect(lyr('~ hello').syllables).toHaveLength(2);
    const l = lyr('hello ~ world');
    expect(l.syllables.map((x) => x.label)).toEqual(['hel', 'lo', '~', 'world']);
    expect(l.words.map((w) => [w.start, w.count])).toEqual([[0, 3], [3, 1]]);
  });
  it('word navigation treats the melisma as part of its word', () => {
    const p = new LyricPointer();
    p.setLyrics(lyr('hello ~ world'));
    p.setIndex(1);
    p.nextWord();
    expect(p.index).toBe(3);
  });
});
