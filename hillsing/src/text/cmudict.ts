// CMU Pronouncing Dictionary access. Data: public/cmudict.txt, built from
// https://github.com/cmusphinx/cmudict (BSD-style license, see public/cmudict-LICENSE.txt).
// Each line is "word PH0 PH1 ..." with ARPAbet phonemes; vowels carry a stress digit.

export type PhonemeLookup = (word: string) => string[] | undefined;

export function parseDict(text: string): Map<string, string[]> {
  const map = new Map<string, string[]>();
  for (const line of text.split('\n')) {
    if (!line) continue;
    const sp = line.indexOf(' ');
    if (sp < 0) continue;
    map.set(line.slice(0, sp), line.slice(sp + 1).split(' '));
  }
  return map;
}

export async function loadDict(url: string): Promise<Map<string, string[]>> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Could not load dictionary (${res.status})`);
  return parseDict(await res.text());
}

const VOICELESS_END = new Set(['P', 'T', 'K', 'F', 'TH']);
const SIBILANT_END = new Set(['S', 'Z', 'SH', 'ZH', 'CH', 'JH']);

/** Dictionary lookup with a possessive/contraction fallback ("dog's" -> "dog" + Z). */
export function makeLookup(dict: Map<string, string[]>): PhonemeLookup {
  return (word) => {
    const key = word.toLowerCase();
    const direct = dict.get(key);
    if (direct) return direct;
    if (key.endsWith("'s")) {
      const base = dict.get(key.slice(0, -2));
      if (base) {
        const last = base[base.length - 1];
        if (SIBILANT_END.has(last)) return [...base, 'AH0', 'Z'];
        return [...base, VOICELESS_END.has(last) ? 'S' : 'Z'];
      }
    }
    return undefined;
  };
}
