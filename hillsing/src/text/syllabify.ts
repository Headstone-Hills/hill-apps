// Phoneme-level syllabification of ARPAbet words.
//
// Method: maximal-onset principle (Selkirk-style legal English onsets), plus one
// stress-sensitive correction: a single consonant between a stressed lax vowel and an
// unstressed vowel stays with the first syllable ("lem-on", not "le-mon"), which is how
// English is usually divided for singing. Syllable boundaries are inherently ambiguous
// (ambisyllabic consonants); this is a deterministic, documented choice, not ground truth.

export interface SyllablePhones {
  onset: string[];
  /** Vowel phoneme with its stress digit, e.g. "AH0". */
  nucleus: string;
  coda: string[];
  stress: number;
}

const LAX = new Set(['AE', 'EH', 'IH', 'AH', 'UH']);
const CANNOT_END_SYLLABLE = new Set(['HH', 'W', 'Y']);

const ONSET2 = new Set<string>();
const ONSET3 = new Set<string>();
function addPairs(firsts: string[], seconds: string[]): void {
  for (const a of firsts) for (const b of seconds) ONSET2.add(`${a} ${b}`);
}
addPairs(['P', 'B', 'K', 'G', 'F'], ['L']);
addPairs(['P', 'B', 'T', 'D', 'K', 'G', 'F', 'TH', 'SH'], ['R']);
addPairs(['T', 'D', 'K', 'G', 'TH'], ['W']);
addPairs(['S'], ['P', 'T', 'K', 'M', 'N', 'L', 'W', 'F']);
addPairs(['P', 'B', 'T', 'D', 'K', 'G', 'F', 'V', 'TH', 'S', 'Z', 'M', 'N', 'L', 'HH'], ['Y']);
for (const a of ['P']) for (const b of ['L', 'R', 'Y']) ONSET3.add(`S ${a} ${b}`);
for (const b of ['R', 'Y']) ONSET3.add(`S T ${b}`);
for (const b of ['L', 'R', 'W', 'Y']) ONSET3.add(`S K ${b}`);

export const isVowelPhoneme = (p: string): boolean => /\d$/.test(p);
const stressOf = (p: string): number => Number(p.slice(-1));
const baseOf = (p: string): string => p.slice(0, -1);

function legalOnset(cluster: string[]): boolean {
  if (cluster.length === 1) return cluster[0] !== 'NG';
  const key = cluster.join(' ');
  if (cluster.length === 2) return ONSET2.has(key);
  if (cluster.length === 3) return ONSET3.has(key);
  return false;
}

export function syllabify(phones: string[]): SyllablePhones[] {
  const vowelAt: number[] = [];
  phones.forEach((p, i) => {
    if (isVowelPhoneme(p)) vowelAt.push(i);
  });

  // No vowel at all (e.g. "hmm", "psst"): sing the whole thing on a schwa-less onset.
  if (vowelAt.length === 0) {
    return [{ onset: phones.slice(), nucleus: 'AH0', coda: [], stress: 0 }];
  }

  const syls: SyllablePhones[] = vowelAt.map((i) => ({
    onset: [],
    nucleus: phones[i],
    coda: [],
    stress: stressOf(phones[i]),
  }));
  syls[0].onset = phones.slice(0, vowelAt[0]);
  syls[syls.length - 1].coda = phones.slice(vowelAt[vowelAt.length - 1] + 1);

  for (let k = 0; k < syls.length - 1; k++) {
    const cons = phones.slice(vowelAt[k] + 1, vowelAt[k + 1]);
    // Number of consonants that stay with the previous syllable.
    let keep = cons.length;
    for (let len = Math.min(3, cons.length); len >= 1; len--) {
      if (legalOnset(cons.slice(cons.length - len))) {
        keep = cons.length - len;
        break;
      }
    }
    if (
      keep === 0 &&
      cons.length === 1 &&
      stressOf(syls[k].nucleus) === 1 &&
      LAX.has(baseOf(syls[k].nucleus)) &&
      stressOf(syls[k + 1].nucleus) === 0 &&
      !CANNOT_END_SYLLABLE.has(cons[0])
    ) {
      keep = 1;
    }
    syls[k].coda = cons.slice(0, keep);
    syls[k + 1].onset = cons.slice(keep);
  }
  return syls;
}
