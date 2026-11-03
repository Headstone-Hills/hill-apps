// Turn an English syllable (ARPAbet phonemes from the dictionary) into the chain of UTAU aliases that sings
// it. Written for X-SAMPA English banks in the VCV style (aliases such as "- bE", "E b", "@ hi", "E -",
// and clusters like "- st"), which is how the English Kasane Teto bank is built; every choice falls back
// to something that exists, so a bank with a different set of aliases still sings (less smoothly).

import type { SyllablePlan } from '../synth/types';

export const ARPA_VOWEL: Record<string, string> = {
  AA: 'A', AE: '{', AO: 'O', AW: 'aU', AY: 'aI', EH: 'E', EY: 'eI', IH: 'I', IY: 'i',
  OW: 'oU', OY: 'OI', UH: 'U', UW: 'u', ER: '3',
};

export const ARPA_CONSONANT: Record<string, string> = {
  B: 'b', CH: 'tS', D: 'd', DH: 'D', F: 'f', G: 'g', HH: 'h', JH: 'dZ', K: 'k', L: 'l', M: 'm', N: 'n', NG: 'N',
  P: 'p', R: 'r', S: 's', SH: 'S', T: 't', TH: 'T', V: 'v', W: 'w', Y: 'j', Z: 'z', ZH: 'Z',
};

/** ARPAbet nucleus with its stress digit -> X-SAMPA vowel. Unstressed AH is schwa; stressed AH is the "cup" vowel. */
export function xsampaVowel(nucleus: string): string {
  const base = nucleus.replace(/\d$/, '');
  const stress = nucleus.match(/\d$/)?.[0];
  if (base === 'AH') return stress === '0' || stress === undefined ? '@' : 'V';
  if (base === 'ER' && stress === '0') return '3';
  return ARPA_VOWEL[base] ?? '@';
}

export const xsampaConsonant = (c: string): string => ARPA_CONSONANT[c] ?? '';

/**
 * How a bank spells its sounds. "xsampa": X-SAMPA like the English Kasane Teto bank ("- bE", "E d"). "arpabet": the
 * lower-case ARPAbet used by ARPAsing diphone banks ("- ay", "b ae", "ae t") and by phoneme-per-file banks ("- aa", "aa", "t").
 */
export type Dialect = 'xsampa' | 'arpabet';

const XSAMPA_TO_ARPA_VOWEL: Record<string, string> = {
  A: 'aa', '{': 'ae', O: 'ao', aU: 'aw', aI: 'ay', E: 'eh', eI: 'ey', I: 'ih', i: 'iy', oU: 'ow', OI: 'oy', U: 'uh', u: 'uw', '3': 'er', V: 'ah', '@': 'ax',
};

/**
 * Other ways an X-SAMPA English bank may spell a vowel (many banks write "a" for A, "e" for E, "o" for oU, and have no
 * diphthong samples, so a diphthong is sung as its first vowel). Tried in order; the standard spelling comes first, so a
 * bank that uses it (like Kasane Teto) is planned exactly as before.
 */
const VOWEL_SPELLINGS: Record<string, string[]> = {
  A: ['A', 'a'], E: ['E', 'e'], V: ['V', '@', 'a', 'A'], O: ['O', 'o'], oU: ['oU', 'o', 'O'], eI: ['eI', 'ei', 'e', 'E'],
  aI: ['aI', 'ai', 'a', 'A'], aU: ['aU', 'au', 'a', 'A'], OI: ['OI', 'Oi', 'O', 'o'], '3': ['3', '@'], I: ['I', 'i'], U: ['U', 'u'],
};

/** The spelling of an X-SAMPA vowel that this bank actually has (any of "- V", "V -", "V", or a consonant joined to it). */
function bankVowel(has: (alias: string) => boolean, v: string): string {
  for (const c of VOWEL_SPELLINGS[v] ?? [v]) {
    if (has(`- ${c}`) || has(`${c} -`) || has(c) || has(`- b${c}`) || has(`b${c}`) || has(`- d${c}`) || has(`- k${c}`)) return c;
  }
  return v;
}

/** Tell which spelling a bank uses by which of a few characteristic aliases it has. */
export function detectDialect(has: (alias: string) => boolean): Dialect {
  const arpa = ['- aa', '- ae', '- ah', '- eh', '- ih', '- iy', '- ow', '- uw', 'aa -', 'ae -', 'ih -', 'iy -', 'ey -', 'ay -'].filter(has).length;
  const xs = ['- A', '- {', '- E', '- I', '- i', '- @', '- u', 'A -', '{ -', 'I -', 'i -', 'E -', '@ -', 'u -'].filter(has).length;
  return arpa >= 3 && arpa > xs ? 'arpabet' : 'xsampa';
}

/** `join` is the previous vowel closing into this syllable's first consonant ("@ l"); it overlaps the start of the syllable. */
export type UnitRole = 'join' | 'onset' | 'vowel' | 'coda' | 'end';

export interface Unit {
  alias: string;
  role: UnitRole;
}

export interface SyllableContext {
  /** The previous syllable's vowel when it flows straight into this one (legato or a tiny gap); null after a rest. */
  prevVowel: string | null;
  /** This syllable ends a word and nothing flows on from it: add the "V -" release if the bank has one. */
  releaseAtEnd: boolean;
  /** Melisma continuation: another note on the same vowel, no onset. */
  melisma: boolean;
  /** How the bank spells its sounds (default X-SAMPA). */
  dialect?: Dialect;
}

/** The aliases that sing one syllable, in order. `has` says whether the bank defines an alias. */
export function planUnits(has: (alias: string) => boolean, plan: SyllablePlan, ctx: SyllableContext): Unit[] {
  if (ctx.dialect === 'arpabet') return planArpabet(has, plan, ctx);
  const first = (...cands: string[]): string | undefined => cands.find((a) => has(a));
  const V = bankVowel(has, xsampaVowel(plan.nucleus));
  const onset = plan.onset.map(xsampaConsonant).filter(Boolean);
  const coda = plan.coda.map(xsampaConsonant).filter(Boolean);
  const X = ctx.prevVowel ?? '-';
  const units: Unit[] = [];
  const add = (alias: string | undefined, role: UnitRole): boolean => {
    if (alias === undefined) return false;
    units.push({ alias, role });
    return true;
  };

  if (ctx.melisma) {
    // Hold the previous vowel on a new pitch: the steady vowel-to-vowel sample.
    add(first(`${V} ${V}`, V, `- ${V}`), 'vowel');
  } else if (onset.length === 0) {
    add(first(`${X} ${V}`, `- ${V}`, V), 'vowel');
  } else {
    // Flowing on from a vowel: let the vowel close into the first consonant, then start the syllable fresh.
    let X2 = X;
    if (ctx.prevVowel && has(`${ctx.prevVowel} ${onset[0]}`)) {
      add(`${ctx.prevVowel} ${onset[0]}`, 'join');
      X2 = '-';
    }
    const last = onset[onset.length - 1];
    const head = onset.slice(0, -1);
    // Consonants before the last one: as a cluster sample if the bank has one ("- st"), else one by one.
    if (head.length > 0) {
      const cluster = head.join('');
      if (!add(first(`${X2} ${cluster}`, `- ${cluster}`), 'onset')) {
        head.forEach((c, i) => add(first(`${i === 0 ? X2 : '-'} ${c}`, `- ${c}`, c), 'onset'));
      }
    }
    // The last consonant joined to the vowel ("bE", "ju" for b+j+u...), starting from the previous vowel only when nothing precedes it.
    const cv = last + V;
    const prefix = head.length === 0 ? X2 : '-';
    const ok = add(first(`${prefix} ${cv}`, `- ${cv}`, cv), 'vowel');
    if (!ok) {
      // No CV sample: the consonant alone, then the vowel.
      add(first(`${prefix} ${last}`, `- ${last}`, last), 'onset');
      add(first(`${last} ${V}`, `- ${V}`, V), 'vowel');
    }
  }

  // Final consonants: vowel -> consonant samples ("E d"), chained when there are several.
  let prev = V;
  for (let i = 0; i < coda.length; ) {
    const rest = coda.slice(i);
    // At the end of a word the bank may have a sample for the whole remaining ending ("l d-"): it needs no vowel.
    if (ctx.releaseAtEnd && add(first(`${prev} ${rest.join('')}-`), 'coda')) break;
    const c = coda[i];
    const a = first(`${prev} ${c}`, i === 0 ? `${V} ${c}` : `${V} ${c}`, c);
    if (add(a, 'coda')) prev = c;
    i++;
  }
  if (coda.length === 0 && ctx.releaseAtEnd) add(first(`${V} -`), 'end');
  return units;
}

/**
 * The same job for a bank spelled in ARPAbet. A diphone bank ("- b", "b ae", "ae t", "t -") is sung as that chain of
 * transitions; a phoneme-per-file bank ("- b", "ae", "t", "t -") has no transition samples, so each step falls back to
 * the single sound. Either way every choice falls back to something that exists.
 */
function planArpabet(has: (alias: string) => boolean, plan: SyllablePlan, ctx: SyllableContext): Unit[] {
  const first = (...cands: string[]): string | undefined => cands.find((a) => has(a));
  const base = plan.nucleus.replace(/\d$/, '');
  const stress = plan.nucleus.match(/\d$/)?.[0];
  const schwa = base === 'AH' && (stress === '0' || stress === undefined);
  const V = schwa ? (has('- ax') || has('ax') ? 'ax' : 'ah') : base.toLowerCase();
  const onset = plan.onset.map((c) => c.toLowerCase());
  const coda = plan.coda.map((c) => c.toLowerCase());
  const prevV = ctx.prevVowel ? (XSAMPA_TO_ARPA_VOWEL[ctx.prevVowel] ?? null) : null;
  const units: Unit[] = [];
  const add = (alias: string | undefined, role: UnitRole): boolean => {
    if (alias === undefined) return false;
    units.push({ alias, role });
    return true;
  };

  if (ctx.melisma) {
    add(first(`${V} ${V}`, V, `- ${V}`), 'vowel');
  } else if (onset.length === 0) {
    add(first(prevV ? `${prevV} ${V}` : `- ${V}`, `- ${V}`, V), 'vowel');
  } else {
    // - c1, c1 c2, ... then the last consonant into the vowel.
    onset.forEach((c, i) => {
      const lastOne = i === onset.length - 1;
      const from = i === 0 ? '-' : onset[i - 1];
      if (lastOne) return;
      add(first(`${from} ${c}`, i === 0 ? `- ${c}` : c, c), 'onset');
    });
    const last = onset[onset.length - 1];
    const from = onset.length === 1 ? '-' : onset[onset.length - 2];
    if (onset.length === 1 || !units.some((u) => u.alias === `${from} ${last}`)) add(first(`${from} ${last}`, `- ${last}`, last), 'onset');
    add(first(`${last} ${V}`, V, `- ${V}`), 'vowel');
  }

  let prev = V;
  for (const c of coda) {
    if (add(first(`${prev} ${c}`, `${V} ${c}`, c), 'coda')) prev = c;
  }
  if (ctx.releaseAtEnd) {
    add(coda.length ? first(`${prev} -`) : first(`${V} -`), 'end');
  }
  return units;
}

/** The English vowels and consonants a bank can sing, found by planning one simple syllable for each. */
export function englishCoverage(has: (alias: string) => boolean, dialect: Dialect): { vowels: [number, number]; consonants: [number, number]; fraction: number } {
  const vowels = ['AA1', 'AE1', 'AH1', 'AH0', 'AO1', 'AW1', 'AY1', 'EH1', 'ER1', 'EY1', 'IH1', 'IY1', 'OW1', 'OY1', 'UH1', 'UW1'];
  const consonants = Object.keys(ARPA_CONSONANT);
  const sings = (p: SyllablePlan): boolean =>
    planUnits(has, p, { prevVowel: null, releaseAtEnd: true, melisma: false, dialect }).some((u) => u.role === 'vowel');
  const v = vowels.filter((n) => sings({ onset: ['B'], nucleus: n, coda: [] })).length;
  const c = consonants.filter((k) => sings({ onset: [k], nucleus: 'AA1', coda: [] })).length;
  return { vowels: [v, vowels.length], consonants: [c, consonants.length], fraction: (v + c) / (vowels.length + consonants.length) };
}
