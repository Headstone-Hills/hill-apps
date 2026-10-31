import type { PhonemeLookup } from './cmudict';
import { guessPhonemes } from './g2p';
import { syllabify, type SyllablePhones } from './syllabify';

export interface LyricSyllable extends SyllablePhones {
  /** Display label: a best-effort split of the word's spelling (display only). */
  label: string;
  wordIndex: number;
  syllableInWord: number;
}

export interface LyricWord {
  text: string;
  /** Index of this word's first syllable in Lyrics.syllables. */
  start: number;
  count: number;
  /** False when the word was missing from the dictionary and pronunciation was guessed. */
  inDict: boolean;
}

export interface Lyrics {
  words: LyricWord[];
  syllables: LyricSyllable[];
}

const DIGIT_WORDS = ['zero', 'one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine'];

/** Words and numbers. Digits are read one at a time ("12" -> "one two"). */
export function tokenize(text: string): string[] {
  const tokens: string[] = [];
  for (const m of text.matchAll(/[A-Za-z]+(?:['’][A-Za-z]+)*|\d+/g)) {
    const t = m[0].replace(/’/g, "'");
    if (/^\d+$/.test(t)) for (const d of t) tokens.push(DIGIT_WORDS[Number(d)]);
    else tokens.push(t);
  }
  return tokens;
}

const isVowelLetter = (c: string): boolean => 'aeiouy'.includes(c);

/**
 * Split a word's spelling into n pieces for display, proportional to phoneme counts and
 * snapped to nearby vowel/consonant boundaries. Purely cosmetic.
 */
export function splitSpelling(word: string, phoneCounts: number[]): string[] {
  const n = phoneCounts.length;
  const w = word.toLowerCase();
  if (n <= 1 || w.length < n) return [word];
  const total = phoneCounts.reduce((a, b) => a + b, 0);
  const candidates: number[] = [];
  for (let p = 1; p < w.length; p++) {
    if (isVowelLetter(w[p - 1]) !== isVowelLetter(w[p]) || w[p - 1] === w[p]) candidates.push(p);
  }
  const cuts: number[] = [];
  let acc = 0;
  for (let i = 0; i < n - 1; i++) {
    acc += phoneCounts[i];
    const ideal = Math.round((w.length * acc) / total);
    const minCut = (cuts[cuts.length - 1] ?? 0) + 1;
    const maxCut = w.length - (n - 1 - i);
    let best = Math.min(Math.max(ideal, minCut), maxCut);
    let bestDist = Infinity;
    for (const c of candidates) {
      if (c < minCut || c > maxCut) continue;
      const d = Math.abs(c - ideal);
      if (d < bestDist) {
        best = c;
        bestDist = d;
      }
    }
    cuts.push(best);
  }
  const pieces: string[] = [];
  let prev = 0;
  for (const c of cuts) {
    pieces.push(word.slice(prev, c));
    prev = c;
  }
  pieces.push(word.slice(prev));
  return pieces;
}

export function buildLyrics(text: string, lookup: PhonemeLookup): Lyrics {
  const words: LyricWord[] = [];
  const syllables: LyricSyllable[] = [];
  for (const token of tokenize(text)) {
    const found = lookup(token);
    const phones = found ?? guessPhonemes(token);
    const sylls = syllabify(phones);
    const counts = sylls.map((s) => s.onset.length + 1 + s.coda.length);
    const labels = splitSpelling(token, counts);
    const wordIndex = words.length;
    words.push({ text: token, start: syllables.length, count: sylls.length, inDict: found !== undefined });
    sylls.forEach((s, i) => {
      syllables.push({ ...s, label: labels[i] ?? token, wordIndex, syllableInWord: i });
    });
  }
  return { words, syllables };
}
