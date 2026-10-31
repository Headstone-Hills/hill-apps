// Built-in listening phrases for judging a voice. Each isolates one thing to listen for, so
// feedback can be specific ("the codas phrase: /k/ is too weak").

import type { MidiNote } from '../midi/smf';
import type { PhonemeLookup } from '../text/cmudict';
import { buildLyrics, type LyricSyllable } from '../text/lyrics';
import { buildSequence, type Sequence } from './sequence';

export type AuditionId = 'vowels' | 'consonants' | 'codas' | 'legato' | 'words' | 'melisma';

export interface AuditionDef {
  id: AuditionId;
  label: string;
  /** What to listen for. */
  hint: string;
}

export const AUDITIONS: ReadonlyArray<AuditionDef> = [
  { id: 'vowels', label: 'Vowels', hint: 'One pitch, ten vowels. Are they distinct and equally loud? Do ai/ow glide smoothly?' },
  { id: 'consonants', label: 'Opening consonants', hint: 'Each consonant + "ah". Which are too weak, harsh or confused with another?' },
  { id: 'codas', label: 'Final consonants', hint: '"ah" + each consonant. Can you hear the word ending? (Dropped endings are a real problem in singing.)' },
  { id: 'legato', label: 'Legato words', hint: 'Overlapping notes. Every consonant should still be heard between the vowels.' },
  { id: 'words', label: 'Sentence', hint: 'A spoken-style sentence on a melody. Is it intelligible?' },
  { id: 'melisma', label: 'Melisma', hint: 'One syllable held across several notes, with the word ending only on the last.' },
];

export interface AuditionOptions {
  /** MIDI note around which phrases are sung (the voice's centre). */
  centerNote: number;
  consonantScale: number;
  lookup: PhonemeLookup;
}

interface Spec {
  syllables: LyricSyllable[];
  /** Semitone offsets from centerNote, cycled across notes. */
  pitches: number[];
  noteSec: number;
  gapSec: number;
}

const syl = (onset: string[], nucleus: string, coda: string[], label: string): LyricSyllable => ({
  onset,
  nucleus,
  coda,
  stress: 1,
  label,
  wordIndex: 0,
  syllableInWord: 0,
});

const ah = (onset: string[], coda: string[], label: string) => syl(onset, 'AA1', coda, label);

function specFor(id: AuditionId, o: AuditionOptions): Spec {
  switch (id) {
    case 'vowels': {
      const v = ['AA', 'IY', 'UW', 'EH', 'AO', 'AE', 'ER', 'IH', 'UH', 'AH'];
      const d = ['AY', 'OW', 'EY'];
      return {
        syllables: [...v, ...d].map((n) => syl([], n + '1', [], n.toLowerCase())),
        pitches: [0],
        noteSec: 0.6,
        gapSec: 0.12,
      };
    }
    case 'consonants': {
      const groups = ['S', 'SH', 'F', 'TH', 'Z', 'V', 'P', 'T', 'K', 'B', 'D', 'G', 'M', 'N', 'L', 'R', 'W', 'Y', 'HH', 'CH', 'JH'];
      return {
        syllables: groups.map((c) => ah([c], [], c.toLowerCase() + 'ah')),
        pitches: [0],
        noteSec: 0.5,
        gapSec: 0.15,
      };
    }
    case 'codas': {
      const ends: string[][] = [['S'], ['SH'], ['F'], ['TH'], ['T'], ['K'], ['P'], ['D'], ['M'], ['N'], ['NG'], ['L'], ['N', 'D'], ['S', 'T'], ['K', 'S', 'T']];
      return {
        syllables: ends.map((c) => ah([], c, 'ah' + c.join('').toLowerCase())),
        pitches: [0],
        noteSec: 0.5,
        gapSec: 0.2,
      };
    }
    case 'legato':
      return {
        syllables: buildLyrics('sing strong and stand still till the end', o.lookup).syllables,
        pitches: [0, 2, 4, 5, 4, 2, 0, -3],
        noteSec: 0.45,
        gapSec: -0.14, // overlap: every note is legato
      };
    case 'words':
      return {
        syllables: buildLyrics('the quick brown fox jumps over the lazy dog', o.lookup).syllables,
        pitches: [0, 0, 4, 4, 5, 5, 4, 2, 2, 0, 0, -3],
        noteSec: 0.36,
        gapSec: 0.1,
      };
    case 'melisma':
      return {
        syllables: buildLyrics('love ~ ~ ~ hold ~ ~ fly ~ ~', o.lookup).syllables,
        pitches: [0, 2, 4, 5, 7, 5, 4, 2, 0, 4],
        noteSec: 0.3,
        gapSec: 0.02,
      };
  }
}

export function buildAudition(id: AuditionId, o: AuditionOptions): Sequence {
  const spec = specFor(id, o);
  const notes: MidiNote[] = spec.syllables.map((_, i) => {
    const start = i * (spec.noteSec + spec.gapSec);
    return {
      start,
      end: start + spec.noteSec,
      note: o.centerNote + spec.pitches[i % spec.pitches.length],
      velocity: 0.8,
      channel: 0,
    };
  });
  return buildSequence(notes, spec.syllables, {
    rate: 1,
    vowelOnBeat: true,
    repeatLyrics: false,
    consonantScale: o.consonantScale,
  });
}
