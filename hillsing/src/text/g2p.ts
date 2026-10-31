// Crude letter-to-sound fallback for words missing from CMUdict (names, slang, typos).
// Deliberately simple: longest-match grapheme rules, "magic e" lengthening, first vowel
// stressed. It will mispronounce plenty of words; it exists so no word is ever skipped.

const VOWEL_LETTERS = new Set(['a', 'e', 'i', 'o', 'u']);

// Longest rule first within each starting position (checked by length below).
const VOWEL_RULES: Array<[string, string]> = [
  ['eigh', 'EY'], ['ough', 'AO'],
  ['ee', 'IY'], ['ea', 'IY'], ['oo', 'UW'], ['ou', 'AW'], ['ow', 'OW'], ['oi', 'OY'],
  ['oy', 'OY'], ['ai', 'EY'], ['ay', 'EY'], ['au', 'AO'], ['aw', 'AO'], ['oa', 'OW'],
  ['ie', 'IY'], ['ey', 'EY'], ['ew', 'UW'], ['ue', 'UW'],
  ['a', 'AE'], ['e', 'EH'], ['i', 'IH'], ['o', 'AA'], ['u', 'AH'],
];
const LONG: Record<string, string> = { a: 'EY', e: 'IY', i: 'AY', o: 'OW', u: 'UW' };

const CONS_RULES: Array<[string, string[]]> = [
  ['tch', ['CH']], ['sch', ['S', 'K']], ['dge', ['JH']],
  ['ch', ['CH']], ['sh', ['SH']], ['th', ['TH']], ['ph', ['F']], ['ng', ['NG']],
  ['ck', ['K']], ['wh', ['W']], ['qu', ['K', 'W']], ['gh', ['G']],
  ['x', ['K', 'S']], ['j', ['JH']], ['q', ['K']],
  ['b', ['B']], ['d', ['D']], ['f', ['F']], ['h', ['HH']], ['k', ['K']], ['l', ['L']],
  ['m', ['M']], ['n', ['N']], ['p', ['P']], ['r', ['R']], ['s', ['S']], ['t', ['T']],
  ['v', ['V']], ['w', ['W']], ['z', ['Z']],
];

export function guessPhonemes(rawWord: string): string[] {
  let w = rawWord.toLowerCase().replace(/[^a-z]/g, '');
  if (!w) return ['AH0'];
  w = w.replace(/(.)\1+/g, '$1'); // collapse doubled letters
  if (w.startsWith('kn')) w = w.slice(1);
  if (w.startsWith('wr')) w = w.slice(1);

  // "Magic e": vowel + consonant + final e lengthens the vowel (make, bike, tune).
  const magic = /([aeiou])([^aeiouy])e$/.exec(w);
  let magicVowelAt = -1;
  if (magic) {
    magicVowelAt = magic.index;
    w = w.slice(0, -1);
  }

  const out: string[] = [];
  let i = 0;
  while (i < w.length) {
    const ch = w[i];
    if (i === magicVowelAt) {
      out.push(LONG[ch] + '#');
      i += 1;
      continue;
    }
    if (ch === 'y') {
      out.push(i === 0 ? 'Y' : i === w.length - 1 ? (i <= 1 ? 'AY#' : 'IY#') : 'IH#');
      i += 1;
      continue;
    }
    if (ch === 'c') {
      out.push(/[eiy]/.test(w[i + 1] ?? '') ? 'S' : 'K');
      i += 1;
      continue;
    }
    if (ch === 'g') {
      out.push(/[ei]/.test(w[i + 1] ?? '') ? 'JH' : 'G');
      i += 1;
      continue;
    }
    if (VOWEL_LETTERS.has(ch)) {
      const rule = VOWEL_RULES.find(([g]) => w.startsWith(g, i));
      if (rule) {
        out.push(rule[1] + '#');
        i += rule[0].length;
        continue;
      }
    }
    const crule = CONS_RULES.filter(([g]) => w.startsWith(g, i)).sort((a, b) => b[0].length - a[0].length)[0];
    if (crule) {
      out.push(...crule[1]);
      i += crule[0].length;
      continue;
    }
    i += 1; // unknown character: skip
  }

  // First vowel gets primary stress, the rest are unstressed.
  let first = true;
  const result = out.map((p) => {
    if (!p.endsWith('#')) return p;
    const s = first ? '1' : '0';
    first = false;
    return p.slice(0, -1) + s;
  });
  if (!result.some((p) => /\d$/.test(p))) result.push('AH0');
  return result;
}
