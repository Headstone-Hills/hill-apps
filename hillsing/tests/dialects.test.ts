import { describe, expect, it } from 'vitest';
import { repairFileNameInPath, repairName } from '../src/utau/names';
import { detectDialect, planUnits, type SyllableContext } from '../src/utau/planner';
import { Voicebank, type BankFile } from '../src/utau/voicebank';
import { buildLyrics } from '../src/text/lyrics';
import { realLookup } from './helpers';

const plan = (word: string) => {
  const s = buildLyrics(word, realLookup()).syllables[0];
  return { onset: s.onset, nucleus: s.nucleus, coda: s.coda };
};
const ctx = (over: Partial<SyllableContext> = {}): SyllableContext => ({ prevVowel: null, releaseAtEnd: true, melisma: false, dialect: 'arpabet', ...over });
const has = (list: string[]) => (a: string) => list.includes(a);

describe('mangled Japanese names', () => {
  it('repairs Shift-JIS names that were unzipped as Windows-1252, and leaves everything else alone', () => {
    expect(repairName('ˆÃ–Âƒjƒ…ƒC EnglishVCCV -DIVA-')).toBe('暗鳴ニュイ EnglishVCCV -DIVA-');
    expect(repairName('Kamenne Taro C+V 1.0')).toBe('Kamenne Taro C+V 1.0');
    expect(repairName('暗鳴ニュイ_English-neo-')).toBe('暗鳴ニュイ_English-neo-');
    expect(repairName('café')).toBe('café');
  });
  it('repairs only the file name of a path (folders keep their on-disk names so paths stay consistent)', () => {
    expect(repairFileNameInPath('Bank/ˆÃ–Âƒjƒ…ƒC/ˆÃ–Â1.wav')).toBe('Bank/ˆÃ–Âƒjƒ…ƒC/暗鳴1.wav'); // the folder is left as it is on disk
    expect(repairFileNameInPath('Bank/plain.wav')).toBeNull();
  });
  it('finds a sample by its real name after repair', async () => {
    const mangled = 'ˆÃ–Â1.wav'; // "暗鳴1.wav" as it sits on disk
    const files: BankFile[] = [
      { path: 'B/oto.ini', read: async () => new TextEncoder().encode('暗鳴1.wav=a,0,10,-100,5,2\n') },
      { path: `B/${mangled}`, read: async () => new Uint8Array() },
    ];
    // utf-8 oto.ini is read as UTF-8
    const b = await Voicebank.load(files, 'B');
    expect(await b.source('B/暗鳴1.wav')).toBeNull(); // the (empty) file is found but is not a WAV: null, not "missing"
    expect(b.entries[0].wavPath).toBe('B/暗鳴1.wav');
  });
});

describe('multi-pitch banks', () => {
  const oto = (suffixes: string[], per: number) =>
    suffixes.flatMap((s) => Array.from({ length: per }, (_, i) => `${s}/${i}.wav=ay${i % 3} ${s},0,100,-300,50,10`)).join('\n');
  const mk = async (suffixes: string[]) => {
    const files: BankFile[] = [{ path: 'M/oto.ini', read: async () => new TextEncoder().encode(oto(suffixes, 120)) }];
    return Voicebank.load(files, 'M');
  };

  it('folds "name + pitch" aliases into one name per sound and picks the recording nearest the note', async () => {
    const b = await mk(['Bb2', 'D3', 'Bb3', 'D4']);
    expect(b.multiPitch).toBe(true);
    expect(b.has('ay0')).toBe(true);
    expect(b.has('ay0 Bb2')).toBe(false);
    expect(b.entry('ay0', 117)!.wavPath).toBe('M/Bb2/0.wav'); // ~Bb2
    expect(b.entry('ay0', 150)!.wavPath).toBe('M/D3/0.wav');
    expect(b.entry('ay0', 230)!.wavPath).toBe('M/Bb3/0.wav');
    expect(b.entry('ay0', 300)!.wavPath).toBe('M/D4/0.wav');
    expect(b.entry('ay0')!.wavPath).toMatch(/Bb3|D4/); // no pitch given: the one nearest middle C
  });

  it('does not treat a lone alias that looks like a note ("D3" = dh + er in X-SAMPA) as a pitch', async () => {
    const files: BankFile[] = [{ path: 'T/oto.ini', read: async () => new TextEncoder().encode('a.wav=- D3,0,10,-100,5,2\nb.wav=- @,0,10,-100,5,2\n') }];
    const b = await Voicebank.load(files, 'T');
    expect(b.multiPitch).toBe(false);
    expect(b.has('- D3')).toBe(true);
  });
});

describe('alias dialects', () => {
  const xs = ['- A', '- {', '- E', '- I', '- i', 'A -', 'i -', '- bE', 'E d'];
  const arpaDiphone = ['- b', 'b eh', 'eh t', 't -', '- ae', '- iy', '- ih', 'iy -', 'ae -', 'eh -', 'b iy', 'iy t', '- s', 's t', 't ih', 'ih n', 'n -'];
  const arpaPhoneme = ['- b', 'b', 'b -', '- eh', 'eh', 'eh -', '- ae', 'ae', 'ae -', '- ih', 'ih', 'ih -', 'iy -', 't', 't -', 'n', 'n -'];

  it('tells X-SAMPA banks from ARPAbet banks', () => {
    expect(detectDialect(has(xs))).toBe('xsampa');
    expect(detectDialect(has(arpaDiphone))).toBe('arpabet');
    expect(detectDialect(has(arpaPhoneme))).toBe('arpabet');
    expect(detectDialect(has([]))).toBe('xsampa');
  });

  it('sings "bet" through an ARPAbet diphone bank as - b, b eh, eh t, t -', () => {
    const units = planUnits(has(arpaDiphone), plan('bet'), ctx());
    expect(units.map((u) => `${u.role}:${u.alias}`)).toEqual(['onset:- b', 'vowel:b eh', 'coda:eh t', 'end:t -']);
  });

  it('sings the same word through a phoneme-per-file bank by falling back to single sounds', () => {
    const units = planUnits(has(arpaPhoneme), plan('bet'), ctx());
    expect(units.map((u) => `${u.role}:${u.alias}`)).toEqual(['onset:- b', 'vowel:eh', 'coda:t', 'end:t -']);
  });

  it('a vowel-first word starts from silence, and a word with no coda ends on the vowel', () => {
    const inn = planUnits(has(arpaDiphone), plan('in'), ctx());
    expect(inn.map((u) => `${u.role}:${u.alias}`)).toEqual(['vowel:- ih', 'coda:ih n', 'end:n -']);
    const be = planUnits(has(arpaDiphone), plan('bee'), ctx());
    expect(be.map((u) => `${u.role}:${u.alias}`)).toEqual(['onset:- b', 'vowel:b iy', 'end:iy -']);
  });

  it('clusters chain diphones: "st" is - s, s t, then t into the vowel', () => {
    const u = planUnits(has(arpaDiphone), { onset: ['S', 'T'], nucleus: 'IH1', coda: ['N'] }, ctx());
    expect(u.map((x) => x.alias)).toEqual(['- s', 's t', 't ih', 'ih n', 'n -']);
  });

  it('leaves the X-SAMPA planning exactly as it was', () => {
    const units = planUnits(has(xs), plan('bet'), { prevVowel: null, releaseAtEnd: false, melisma: false });
    expect(units.map((u) => `${u.role}:${u.alias}`)).toEqual(['vowel:- bE']);
  });
});

describe('banks that spell vowels their own way, and how well a bank covers English', () => {
  // a made-up X-SAMPA bank in the "a / e / o" style: no oU, eI, aI, aU, A, E, V samples under the standard names
  const neoish = ['- @', '- {', '- 3', '- a', '- e', '- i', '- I', '- o', '- O', '- u', '- U', '- ba', '- be', '- bo', 'a -', 'e -', 'o -', '@ -', 'a b', '- b@', '- b{', '- bi', '- bI', '- bu', '- bU', '- bO', '- b3'];
  it('sings a diphthong or a vowel through the spelling the bank has', () => {
    const word = (w: string) => planUnits(has(neoish), plan(w), { prevVowel: null, releaseAtEnd: false, melisma: false });
    expect(word('boat').map((u) => u.alias)[0]).toBe('- bo'); // oU -> o
    expect(word('bet').map((u) => u.alias)[0]).toBe('- be'); // E -> e
    expect(word('bait').map((u) => u.alias)[0]).toBe('- be'); // eI -> e
    expect(word('but').map((u) => u.alias)[0]).toBe('- b@'); // stressed AH (V) -> @
  });
  it('coverage: a bank with the standard names covers more than one with none', async () => {
    const { englishCoverage } = await import('../src/utau/planner');
    const full = englishCoverage(has(['- bA', '- b{', '- bE', '- bI', '- bi', '- bU', '- bu', '- bO', '- bV', '- b@', '- b3', '- boU', '- beI', '- baI', '- baU', '- bOI', '- ba', '- be', '- bo', '- kA', '- tA', '- pA', '- dA', '- gA', '- fA', '- vA', '- sA', '- zA', '- SA', '- ZA', '- TA', '- DA', '- hA', '- mA', '- nA', '- NA', '- lA', '- rA', '- wA', '- jA', '- tSA', '- dZA']), 'xsampa');
    const none = englishCoverage(has(['- foo', 'bar']), 'xsampa');
    expect(full.fraction).toBeGreaterThan(0.9);
    expect(none.fraction).toBe(0);
  });
});
