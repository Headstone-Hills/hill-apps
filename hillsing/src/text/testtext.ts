// Ready-made lyrics for listening tests.

/** Every sound of English at least once: all the vowels, diphthongs and consonants, one short word each. */
export const ALL_SOUNDS_TEXT =
  'beat bit bait bet bat father law but boat book boot bite bout boy bird about ' + // vowels, diphthongs, ER, schwa
  'pop bob tip dad kick gag church judge fish very think this sun zoo shoe vision ' + // stops, affricates, fricatives
  'hello my no sing love red we you'; // h, nasals, liquids, glides

/** A sentence to sing as a melody: ordinary words in ordinary order. */
export const SENTENCE_TEXT = 'The quick brown fox jumps over the lazy dog while you sing a long happy song';

/** The 39 ARPAbet sounds of English (without stress marks). */
export const ENGLISH_SOUNDS: ReadonlyArray<string> = [
  'AA', 'AE', 'AH', 'AO', 'AW', 'AY', 'B', 'CH', 'D', 'DH', 'EH', 'ER', 'EY', 'F', 'G', 'HH', 'IH', 'IY', 'JH', 'K', 'L', 'M',
  'N', 'NG', 'OW', 'OY', 'P', 'R', 'S', 'SH', 'T', 'TH', 'UH', 'UW', 'V', 'W', 'Y', 'Z', 'ZH',
];
