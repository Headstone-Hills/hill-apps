// Editing the lyric *text* from the piano roll. The text stays the single source of truth for
// syllables (one syllable per note, in order); the editor changes a note's syllable by rewriting the
// word in the text, and makes a note a melisma continuation by inserting a "~" before its word.

import type { PhonemeLookup } from '../text/cmudict';
import { buildLyrics, MELISMA_TOKENS, TOKEN_PATTERN } from '../text/lyrics';

export interface TokenSpan {
  /** Character range of the token in the text. */
  start: number;
  end: number;
  text: string;
  /** Index of the token's first syllable, and how many syllables it produces. */
  firstSyllable: number;
  syllables: number;
  melisma: boolean;
}

/** Map each token of `text` to the syllables it produces, exactly as buildLyrics counts them. */
export function tokenSpans(text: string, lookup: PhonemeLookup): TokenSpan[] {
  const spans: TokenSpan[] = [];
  let syl = 0;
  for (const m of text.matchAll(new RegExp(TOKEN_PATTERN, 'g'))) {
    const raw = m[0];
    const melisma = MELISMA_TOKENS.has(raw);
    const count = melisma ? (syl > 0 ? 1 : 0) : buildLyrics(raw, lookup).syllables.length;
    spans.push({ start: m.index, end: m.index + raw.length, text: raw, firstSyllable: syl, syllables: count, melisma });
    syl += count;
  }
  return spans;
}

export function spanForSyllable(spans: ReadonlyArray<TokenSpan>, index: number): TokenSpan | undefined {
  return spans.find((s) => index >= s.firstSyllable && index < s.firstSyllable + s.syllables);
}

/** Replace the word that produces syllable `index`. Empty text removes the word. Null if there is no such word. */
export function renameSyllable(text: string, lookup: PhonemeLookup, index: number, replacement: string): string | null {
  const span = spanForSyllable(tokenSpans(text, lookup), index);
  if (!span) return null;
  const word = replacement.trim();
  if (word === '') return removeSpan(text, span);
  return text.slice(0, span.start) + word + text.slice(span.end);
}

/**
 * Toggle "hold the previous vowel" for the note that sings syllable `index`: a melisma token is
 * removed, a word that starts at `index` gets a "~" in front of it (so this note continues the
 * previous word's vowel and the word moves to the next note). Null when that is not possible
 * (the first syllable has nothing to continue; the middle of a multi-syllable word cannot be split).
 */
export function toggleMelisma(text: string, lookup: PhonemeLookup, index: number): string | null {
  const span = spanForSyllable(tokenSpans(text, lookup), index);
  if (!span) return null;
  if (span.melisma) return removeSpan(text, span);
  if (index === 0 || index !== span.firstSyllable) return null;
  return text.slice(0, span.start) + '~ ' + text.slice(span.start);
}

/** Remove a token and one adjacent space so words do not run together or leave double spaces. */
function removeSpan(text: string, span: TokenSpan): string {
  let { start, end } = span;
  if (text[end] === ' ') end++;
  else if (start > 0 && text[start - 1] === ' ') start--;
  return text.slice(0, start) + text.slice(end);
}
