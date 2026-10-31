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
}

/** The aliases that sing one syllable, in order. `has` says whether the bank defines an alias. */
export function planUnits(has: (alias: string) => boolean, plan: SyllablePlan, ctx: SyllableContext): Unit[] {
  const first = (...cands: string[]): string | undefined => cands.find((a) => has(a));
  const V = xsampaVowel(plan.nucleus);
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
