import type { LyricSyllable, Lyrics } from './lyrics';

/**
 * Tracks which syllable the next note will sing. `index` is the *next* syllable; after
 * singing syllable i, index is i + 1. index === length means "end of text".
 */
export class LyricPointer {
  private lyrics: Lyrics = { words: [], syllables: [] };
  private _index = 0;
  wrap = false;
  onChange: (() => void) | null = null;

  get index(): number {
    return this._index;
  }
  get length(): number {
    return this.lyrics.syllables.length;
  }
  get atEnd(): boolean {
    return this._index >= this.length;
  }

  setLyrics(lyrics: Lyrics, keepPosition = false): void {
    this.lyrics = lyrics;
    this._index = keepPosition ? Math.min(this._index, lyrics.syllables.length) : 0;
    this.changed();
  }

  peek(): LyricSyllable | null {
    return this.lyrics.syllables[this._index] ?? null;
  }

  /** Consume the next syllable (called on each fresh note). Null at end of text. */
  advance(): LyricSyllable | null {
    const syl = this.peek();
    if (!syl) return null;
    this._index += 1;
    if (this._index >= this.length && this.wrap) this._index = 0;
    this.changed();
    return syl;
  }

  setIndex(i: number): void {
    this._index = Math.max(0, Math.min(this.length, i));
    this.changed();
  }
  reset(): void {
    this.setIndex(0);
  }
  back(): void {
    this.setIndex(this._index - 1);
  }
  forward(): void {
    this.setIndex(this._index + 1);
  }

  /** Word containing the next syllable (the last word when at the end). */
  private currentWordIndex(): number {
    const { words, syllables } = this.lyrics;
    if (words.length === 0) return 0;
    const s = syllables[Math.min(this._index, syllables.length - 1)];
    return s ? s.wordIndex : 0;
  }

  nextWord(): void {
    const { words } = this.lyrics;
    if (words.length === 0) return;
    const w = this.currentWordIndex();
    this.setIndex(w + 1 < words.length ? words[w + 1].start : this.length);
  }

  /** Start of the current word if mid-word, otherwise start of the previous word. */
  prevWord(): void {
    const { words } = this.lyrics;
    if (words.length === 0) return;
    if (this.atEnd) {
      this.setIndex(words[words.length - 1].start);
      return;
    }
    const w = this.currentWordIndex();
    if (this._index > words[w].start) this.setIndex(words[w].start);
    else this.setIndex(words[Math.max(0, w - 1)].start);
  }

  private changed(): void {
    this.onChange?.();
  }
}
