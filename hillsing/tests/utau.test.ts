// UTAU sample playback, stage 1. These tests use synthetic data only: voicebanks are copyrighted and
// must never be added to this repository. A second block runs against a real bank when
// HILLSING_UTAU_BANK points at its folder (skipped otherwise).

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';
import { decodeText, frqPathFor, parseFrq, parseOto, parseWav } from '../src/utau/files';
import { estimateF0, mixInto, pitchMarks, renderSegment, type PsolaSource } from '../src/utau/psola';
import { planUnits, xsampaVowel } from '../src/utau/planner';
import { renderUtau } from '../src/utau/render';
import { alignOffset, buildAttack, buildRelease, capPeak, seamlessLoop } from '../src/utau/livenote';
import { steadyWindow } from '../src/utau/spectral';
import { Voicebank, type BankFile } from '../src/utau/voicebank';
import { openZip } from '../src/utau/zip';
import { deflateRawSync } from 'node:zlib';
import { buildLyrics } from '../src/text/lyrics';
import { realLookup } from './helpers';

const SR = 44100;

/** A voiced-ish test tone: harmonics of f0 with a decaying spectrum, optionally with a noise burst first. */
function tone(f0: number, seconds: number, burstSamples = 0): Float32Array {
  const n = Math.round(seconds * SR);
  const x = new Float32Array(n);
  let seed = 12345;
  for (let i = 0; i < n; i++) {
    let s = 0;
    for (let k = 1; k <= 8; k++) s += Math.sin((2 * Math.PI * k * f0 * i) / SR) / k;
    x[i] = 0.3 * s;
    if (i < burstSamples) {
      seed = (seed * 1664525 + 1013904223) >>> 0;
      x[i] = ((seed / 4294967296) * 2 - 1) * 0.15;
    }
  }
  return x;
}

function frqFor(f0: number, samples: number, voicedFrom = 0): { interval: number; f0: Float64Array } {
  const frames = Math.ceil(samples / 256);
  return { interval: 256, f0: Float64Array.from({ length: frames }, (_, i) => (i * 256 >= voicedFrom ? f0 : 0)) };
}

const rms = (x: Float32Array, a = 0, b = x.length): number => {
  let s = 0;
  for (let i = a; i < b; i++) s += x[i] * x[i];
  return Math.sqrt(s / Math.max(1, b - a));
};

function wavBytes(samples: Float32Array, sr = SR, bits: 16 | 24 = 16, channels = 1): Uint8Array {
  const bps = bits / 8;
  const data = samples.length * channels * bps;
  const b = new Uint8Array(44 + data);
  const v = new DataView(b.buffer);
  const tag = (o: number, s: string) => [...s].forEach((c, i) => (b[o + i] = c.charCodeAt(0)));
  tag(0, 'RIFF'); v.setUint32(4, 36 + data, true); tag(8, 'WAVE'); tag(12, 'fmt ');
  v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, channels, true); v.setUint32(24, sr, true);
  v.setUint32(28, sr * channels * bps, true); v.setUint16(32, channels * bps, true); v.setUint16(34, bits, true);
  tag(36, 'data'); v.setUint32(40, data, true);
  samples.forEach((s, i) => {
    for (let c = 0; c < channels; c++) {
      const o = 44 + (i * channels + c) * bps;
      const q = Math.round(Math.max(-1, Math.min(1, s)) * (bits === 16 ? 32767 : 8388607));
      if (bits === 16) v.setInt16(o, q, true);
      else { b[o] = q & 255; b[o + 1] = (q >> 8) & 255; b[o + 2] = (q >> 16) & 255; }
    }
  });
  return b;
}

function frqBytes(f0: Float64Array, interval = 256): Uint8Array {
  const b = new Uint8Array(40 + f0.length * 16);
  const v = new DataView(b.buffer);
  [...'FREQ0003'].forEach((c, i) => (b[i] = c.charCodeAt(0)));
  v.setInt32(8, interval, true);
  v.setFloat64(12, 200, true);
  v.setInt32(36, f0.length, true);
  f0.forEach((hz, i) => { v.setFloat64(40 + i * 16, hz, true); v.setFloat64(48 + i * 16, 1000, true); });
  return b;
}

describe('oto.ini', () => {
  it('parses entries, defaults the alias to the file name, and skips junk lines', () => {
    const text = [
      'a.wav=あ,100,50.5,-200,30,15',
      'ka.wav=,10,20,30,40,50',
      'broken line without equals',
      '',
      'b.wav=x y,1.5',
    ].join('\r\n');
    const e = parseOto(text, 'bank/');
    expect(e).toHaveLength(3);
    expect(e[0]).toEqual({ wavPath: 'bank/a.wav', alias: 'あ', offset: 100, consonant: 50.5, cutoff: -200, preutter: 30, overlap: 15 });
    expect(e[1].alias).toBe('ka');
    expect(e[2]).toMatchObject({ alias: 'x y', offset: 1.5, consonant: 0, cutoff: 0 });
  });

  it('decodes UTF-8 and Shift-JIS text', () => {
    expect(decodeText(new TextEncoder().encode('\uFEFFあ=1'))).toBe('あ=1');
    const sjis = new Uint8Array([0x82, 0xa0, 0x3d, 0x31]); // "あ=1" in Shift-JIS
    expect(decodeText(sjis)).toBe('あ=1');
  });
});

describe('WAV and .frq', () => {
  it('reads 16-bit and 24-bit PCM, mixing channels to mono', () => {
    const src = tone(220, 0.05);
    const w16 = parseWav(wavBytes(src));
    expect(w16.sampleRate).toBe(SR);
    expect(w16.samples).toHaveLength(src.length);
    expect(Math.abs(w16.samples[100] - src[100])).toBeLessThan(1e-3);
    const w24 = parseWav(wavBytes(src, 22050, 24, 2));
    expect(w24.sampleRate).toBe(22050);
    expect(Math.abs(w24.samples[100] - src[100])).toBeLessThan(1e-4);
    expect(() => parseWav(new Uint8Array(20))).toThrow();
  });

  it('reads the pitch track and maps out-of-range values to unvoiced', () => {
    const f = parseFrq(frqBytes(Float64Array.from([0, 200, 220, 5, 3000])));
    expect(f.interval).toBe(256);
    expect([...f.f0]).toEqual([0, 200, 220, 0, 0]);
    expect(() => parseFrq(new Uint8Array(50))).toThrow();
    expect(frqPathFor('dir/a b.wav')).toBe('dir/a b_wav.frq');
  });
});

describe('PSOLA', () => {
  const makeSource = (f0: number, seconds = 0.6, burst = 0): PsolaSource => {
    const samples = tone(f0, seconds, burst);
    return { samples, sampleRate: SR, frq: frqFor(f0, samples.length, burst) };
  };

  it('finds roughly one pitch mark per period in voiced parts, and none in unvoiced parts', () => {
    const src = makeSource(200, 0.6, 2000);
    const marks = pitchMarks(src, 0, src.samples.length);
    expect(marks.length).toBeGreaterThan(0.8 * ((0.6 * SR - 2000) / (SR / 200)));
    expect(marks[0]).toBeGreaterThanOrEqual(1900);
  });

  it('shifts pitch to the target and keeps the loudness', () => {
    for (const target of [150, 200, 260, 300]) {
      const src = makeSource(200);
      const out = renderSegment(src, { start: 0, end: src.samples.length, fixed: 0, length: src.samples.length, f0: target });
      const mid = out.subarray(Math.round(0.15 * SR), Math.round(0.45 * SR));
      expect(Math.abs(estimateF0(mid, SR) / target - 1), `target ${target}`).toBeLessThan(0.03);
      const ratio = rms(out, 8000, out.length - 8000) / rms(src.samples, 8000, src.samples.length - 8000);
      expect(ratio, `target ${target}`).toBeGreaterThan(0.7);
      expect(ratio, `target ${target}`).toBeLessThan(1.4);
    }
  });

  it('stretches the vowel to the requested length without changing its pitch', () => {
    const src = makeSource(200);
    const out = renderSegment(src, { start: 0, end: src.samples.length, fixed: 0, length: Math.round(1.5 * SR), f0: 200 });
    expect(out).toHaveLength(Math.round(1.5 * SR));
    expect(Math.abs(estimateF0(out.subarray(Math.round(0.4 * SR), Math.round(1.0 * SR)), SR) / 200 - 1)).toBeLessThan(0.03);
    // loudness is steady across the stretched part (no gaps or pumping)
    const a = rms(out, Math.round(0.3 * SR), Math.round(0.6 * SR));
    const b = rms(out, Math.round(0.9 * SR), Math.round(1.2 * SR));
    expect(Math.abs(20 * Math.log10(a / b))).toBeLessThan(1.5);
  });

  it('keeps the consonant (fixed) part at natural speed and unshifted', () => {
    const burst = 3000;
    const src = makeSource(200, 0.6, burst);
    const out = renderSegment(src, { start: 0, end: src.samples.length, fixed: burst, length: Math.round(1.2 * SR), f0: 300 });
    // the noise burst occupies the same first ~3000 samples in the output
    const burstLevel = rms(out, 200, burst - 200);
    const gapAfter = rms(out, burst + 200, burst + 1200);
    expect(burstLevel).toBeGreaterThan(0.05);
    expect(gapAfter).toBeGreaterThan(0.05); // voiced part follows without a hole
    expect(Math.abs(estimateF0(out.subarray(Math.round(0.4 * SR), Math.round(0.9 * SR)), SR) / 300 - 1)).toBeLessThan(0.03);
  });

  it('crossfades one rendered segment into another', () => {
    const a = new Float32Array(1000).fill(1);
    const b = new Float32Array(1000).fill(1);
    const m = mixInto(a, b, 600, 200);
    expect(m).toHaveLength(1600);
    expect(m[100]).toBe(1);
    expect(m[1400]).toBe(1);
    expect(Math.abs(m[700] - 1)).toBeLessThan(0.45); // through the fade the level stays close to unity
  });
});

describe('voicebank loading', () => {
  const file = (path: string, bytes: Uint8Array): BankFile => ({ path, read: async () => bytes });

  it('indexes aliases, resolves cutoffs, and renders a sample at a new pitch', async () => {
    const x = tone(200, 0.8, 2205); // 50 ms noise burst, then a 200 Hz vowel
    const oto = 'vowel.wav=- a,0,50,-400,40,20\nvowel.wav=a,100,30,300,10,5\n';
    const bank = await Voicebank.load(
      [
        file('Bank/oto.ini', new TextEncoder().encode(oto)),
        file('Bank/vowel.wav', wavBytes(x)),
        file('Bank/vowel_wav.frq', frqBytes(frqFor(200, x.length, 2205).f0)),
      ],
      'Bank',
    );
    expect(bank.entries).toHaveLength(2);
    expect(bank.has('- a')).toBe(true);
    expect(bank.aliases()).toEqual(['- a', 'a']);
    const src = (await bank.source('Bank/vowel.wav'))!;
    expect(src.frq).not.toBeNull();
    // cutoff -400: 400 ms usable after the offset; cutoff 300 (positive): trimmed 300 ms from the file's end
    const r1 = bank.region(bank.entry('- a')!, src);
    expect((r1.end - r1.start) / SR).toBeCloseTo(0.4, 3);
    expect(r1.fixed / SR).toBeCloseTo(0.05, 3);
    const r2 = bank.region(bank.entry('a')!, src);
    expect(r2.start / SR).toBeCloseTo(0.1, 3);
    expect(r2.end / SR).toBeCloseTo(0.5, 3);

    const out = (await bank.renderAlias('- a', 300, 700))!;
    expect(out.sampleRate).toBe(SR);
    expect(out.samples).toHaveLength(Math.round(0.7 * SR));
    expect(Math.abs(estimateF0(out.samples.subarray(Math.round(0.25 * SR), Math.round(0.6 * SR)), SR) / 300 - 1)).toBeLessThan(0.03);
    expect(await bank.renderAlias('nope', 200)).toBeNull();
  });

  it('matches wav names case-insensitively and across Unicode normal forms, and rejects a folder with no oto.ini', async () => {
    const x = tone(200, 0.3);
    const nfd = 'ぱ.wav'.normalize('NFD');
    const bank = await Voicebank.load(
      [file('B/oto.ini', new TextEncoder().encode(`${'ぱ.wav'.normalize('NFC')}=ぱ,0,10,-100,0,0`)), file(`B/${nfd.toUpperCase()}`, wavBytes(x))],
      'B',
    );
    expect(await bank.renderAlias('ぱ', 200)).not.toBeNull();
    await expect(Voicebank.load([file('x/a.wav', wavBytes(x))])).rejects.toThrow(/oto\.ini/);
  });
});

describe('alias planner', () => {
  const bank = new Set(['- s', '- h@', '@ loU', 'oU -', '- bE', 'E d', '- rE', '- b', '- st', '- tE', 'E -', 'i i', '@ i', '- i', 'A A', '- A', 'I d', 'E b']);
  const has = (a: string) => bank.has(a);
  const ctx = { prevVowel: null, releaseAtEnd: false, melisma: false };

  it('maps ARPAbet vowels to X-SAMPA, with stress deciding schwa versus the cup vowel', () => {
    expect(xsampaVowel('AH0')).toBe('@');
    expect(xsampaVowel('AH1')).toBe('V');
    expect(xsampaVowel('EY1')).toBe('eI');
    expect(xsampaVowel('ER0')).toBe('3');
    expect(xsampaVowel('IY2')).toBe('i');
  });

  it('chains a word as start, vowel join, release', () => {
    const he = planUnits(has, { onset: ['HH'], nucleus: 'AH0', coda: [] }, ctx);
    expect(he.map((u) => u.alias)).toEqual(['- h@']);
    const lo = planUnits(has, { onset: ['L'], nucleus: 'OW1', coda: [] }, { ...ctx, prevVowel: '@', releaseAtEnd: true });
    expect(lo.map((u) => `${u.role}:${u.alias}`)).toEqual(['vowel:@ loU', 'end:oU -']); // this fake bank has the joined form
    // in a CVVC bank the vowel closes into the consonant first, then the syllable starts fresh
    const cvvc = new Set(['- lE', '@ l', 'E -']);
    expect(planUnits((a) => cvvc.has(a), { onset: ['L'], nucleus: 'EH1', coda: [] }, { ...ctx, prevVowel: '@', releaseAtEnd: true }).map((u) => `${u.role}:${u.alias}`)).toEqual(['join:@ l', 'vowel:- lE', 'end:E -']);
  });

  it('puts the previous vowel in front only when the notes flow together, and falls back to "-" otherwise', () => {
    expect(planUnits(has, { onset: ['L'], nucleus: 'OW1', coda: [] }, { ...ctx, prevVowel: '@' })[0].alias).toBe('@ loU');
    // no "@ bE" in this bank: falls back to the start form
    expect(planUnits(has, { onset: ['B'], nucleus: 'EH1', coda: [] }, { ...ctx, prevVowel: '@' })[0].alias).toBe('- bE');
  });

  it('sings consonant clusters as a cluster sample (or single consonants) plus the last consonant with the vowel', () => {
    const stE = planUnits(has, { onset: ['S', 'T'], nucleus: 'EH1', coda: [] }, ctx);
    expect(stE.map((u) => u.alias)).toEqual(['- s', '- tE']);
    // three consonants: the first two as a cluster sample ("- st"), then the last with the vowel
    const strE = planUnits(has, { onset: ['S', 'T', 'R'], nucleus: 'EH1', coda: [] }, ctx);
    expect(strE.map((u) => u.alias)).toEqual(['- st', '- rE']);
    const bread = planUnits(has, { onset: ['B', 'R'], nucleus: 'EH1', coda: ['D'] }, ctx);
    expect(bread.map((u) => `${u.role}:${u.alias}`)).toEqual(['onset:- b', 'vowel:- rE', 'coda:E d']);
  });

  it('uses a whole-ending sample for a word-final cluster ("l d-"), and one consonant at a time otherwise', () => {
    const b = new Set(['- w3', '3 l', 'l d-', '3 d', 'E n', 'E d']);
    const world = planUnits((a) => b.has(a), { onset: ['W'], nucleus: 'ER1', coda: ['L', 'D'] }, { ...ctx, releaseAtEnd: true });
    expect(world.map((u) => u.alias)).toEqual(['- w3', '3 l', 'l d-']);
    // not at a word end: no "-" ending samples; the second consonant falls back to the vowel form
    const mid = planUnits((a) => b.has(a), { onset: ['W'], nucleus: 'ER1', coda: ['L', 'D'] }, ctx);
    expect(mid.map((u) => u.alias)).toEqual(['- w3', '3 l', '3 d']);
  });

  it('holds a melisma on the vowel-to-vowel sample and skips what the bank lacks', () => {
    expect(planUnits(has, { onset: [], nucleus: 'IY1', coda: [] }, { ...ctx, melisma: true }).map((u) => u.alias)).toEqual(['i i']);
    expect(planUnits(has, { onset: ['Z', 'Z'], nucleus: 'AH0', coda: [] }, ctx)).toEqual([]); // nothing usable at all
  });
});

/** A minimal zip writer for tests: stored or deflated entries, names as raw bytes with optional UTF-8 flag. */
function makeZip(entries: Array<{ name: Uint8Array; data: Uint8Array; deflate?: boolean; utf8?: boolean }>, extra: { junk?: boolean } = {}): Uint8Array {
  const parts: Uint8Array[] = [];
  const cen: Uint8Array[] = [];
  let offset = 0;
  const u16 = (n: number) => new Uint8Array([n & 255, (n >> 8) & 255]);
  const u32 = (n: number) => new Uint8Array([n & 255, (n >> 8) & 255, (n >> 16) & 255, (n >>> 24) & 255]);
  const cat = (...a: Uint8Array[]) => { const o = new Uint8Array(a.reduce((t, x) => t + x.length, 0)); let p = 0; for (const x of a) { o.set(x, p); p += x.length; } return o; };
  for (const e of entries) {
    const comp = e.deflate ? new Uint8Array(deflateRawSync(e.data)) : e.data;
    const flags = e.utf8 ? 0x800 : 0;
    const method = e.deflate ? 8 : 0;
    const local = cat(u32(0x04034b50), u16(20), u16(flags), u16(method), u16(0), u16(0), u32(0), u32(comp.length), u32(e.data.length), u16(e.name.length), u16(0), e.name, comp);
    cen.push(cat(u32(0x02014b50), u16(20), u16(20), u16(flags), u16(method), u16(0), u16(0), u32(0), u32(comp.length), u32(e.data.length), u16(e.name.length), u16(0), u16(0), u16(0), u16(0), u32(0), u32(offset), e.name));
    parts.push(local);
    offset += local.length;
  }
  const cd = cat(...cen);
  void extra;
  return cat(...parts, cd, u32(0x06054b50), u16(0), u16(0), u16(entries.length), u16(entries.length), u32(cd.length), u32(offset), u16(0));
}

describe('zip voicebanks', () => {
  const enc = new TextEncoder();
  it('lists and reads stored and deflated entries, skipping folders and macOS junk', async () => {
    const x = tone(200, 0.4, 1000);
    const wav = wavBytes(x);
    const zip = makeZip([
      { name: enc.encode('Bank/'), data: new Uint8Array(0) },
      { name: enc.encode('Bank/oto.ini'), data: enc.encode('t.wav=- a,0,40,-300,0,0\n'), deflate: true, utf8: true },
      { name: enc.encode('Bank/t.wav'), data: wav, deflate: true, utf8: true },
      { name: enc.encode('Bank/t_wav.frq'), data: frqBytes(frqFor(200, x.length, 1000).f0) },
      { name: enc.encode('__MACOSX/Bank/._t.wav'), data: enc.encode('junk') },
    ]);
    const files = await openZip(new Blob([zip as BlobPart]));
    expect(files.map((f) => f.path)).toEqual(['Bank/oto.ini', 'Bank/t.wav', 'Bank/t_wav.frq']);
    expect(Buffer.from(await files[1].read()).equals(Buffer.from(wav))).toBe(true); // deflated entry round-trips
    const bank = await Voicebank.load(files, 'Bank');
    expect(bank.has('- a')).toBe(true);
    expect(await bank.renderAlias('- a', 220)).not.toBeNull();
  });

  it('decodes Shift-JIS folder names written without the UTF-8 flag', async () => {
    const sjis = new Uint8Array([0x82, 0xa0, 0x2f, 0x6f, 0x74, 0x6f, 0x2e, 0x69, 0x6e, 0x69]); // "あ/oto.ini"
    const zip = makeZip([{ name: sjis, data: enc.encode('x.wav=a,0,1,-1,0,0\n') }]);
    const files = await openZip(new Blob([zip as BlobPart]));
    expect(files[0].path).toBe('あ/oto.ini');
  });

  it('gives clear errors for things that are not usable zips', async () => {
    await expect(openZip(new Blob([new Uint8Array(100)]))).rejects.toThrow(/zip/i);
    await expect(openZip(new Blob([makeZip([]) as BlobPart]))).rejects.toThrow(/no files/);
  });
});

describe('live notes', () => {
  const file = (path: string, bytes: Uint8Array): BankFile => ({ path, read: async () => bytes });
  async function liveBank() {
    const x = tone(200, 1.2, 2205);
    const oto = ['v.wav=- A,0,50,-900,40,20', 'v.wav=A d,600,60,-500,200,50', 'v.wav=A -,700,40,-400,150,40'].join('\n');
    return Voicebank.load([file('B/oto.ini', new TextEncoder().encode(oto)), file('B/v.wav', wavBytes(x)), file('B/v_wav.frq', frqBytes(frqFor(200, x.length, 2205).f0))], 'B');
  }

  it('builds an attack plus a seamless, in-pitch loop for a held vowel', async () => {
    const bank = await liveBank();
    for (const target of [160, 220, 300]) {
      const a = (await buildAttack(bank, { onset: [], nucleus: 'AA1', coda: [] }, target, null))!;
      expect(a.aliases).toEqual(['- A']);
      expect(a.samples.length).toBeGreaterThan(0.15 * SR); // consonants play 1.5x faster live, so the attack is short
      expect(a.loop).not.toBeNull();
      expect(a.loop!.length / SR).toBeGreaterThan(0.1);
      // the loop repeats at the target pitch...
      const twice = new Float32Array(a.loop!.length * 2);
      twice.set(a.loop!);
      twice.set(a.loop!, a.loop!.length);
      expect(Math.abs(estimateF0(twice.subarray(0, Math.min(twice.length, 9000)), SR) / target - 1)).toBeLessThan(0.04);
      // ...and has no jump where it wraps from its end back to its start
      let maxStep = 0;
      for (let i = 1; i < a.loop!.length; i++) maxStep = Math.max(maxStep, Math.abs(a.loop![i] - a.loop![i - 1]));
      const wrap = Math.abs(a.loop![0] - a.loop![a.loop!.length - 1]);
      expect(wrap, `wrap step at ${target} Hz`).toBeLessThan(maxStep * 1.5 + 1e-3);
    }
  });

  it('sustains a held vowel for seconds without repeating a short slice', async () => {
    const bank = await liveBank();
    const a = (await buildAttack(bank, { onset: [], nucleus: 'AA1', coda: [] }, 220, null))!;
    // a short loop would repeat 7-10 times a second (audible as a stutter); this one lasts several seconds
    expect(a.loop!.length / SR).toBeGreaterThan(4);
    // and its loudness does not pulse: 30 ms windows stay within 25% of the average
    const w = Math.round(0.03 * SR);
    const levels: number[] = [];
    for (let i = 0; i + w <= a.loop!.length; i += w) levels.push(rms(a.loop!, i, i + w));
    const mean = levels.reduce((p, c) => p + c, 0) / levels.length;
    expect(Math.min(...levels) / mean).toBeGreaterThan(0.75);
    expect(Math.max(...levels) / mean).toBeLessThan(1.25);
  });

  it('starts an attack from silence (no click) and matches the release level to the sustain', async () => {
    const bank = await liveBank();
    const a = (await buildAttack(bank, { onset: [], nucleus: 'AA1', coda: [] }, 220, null))!;
    const peak = Math.max(...a.samples.map(Math.abs));
    expect(Math.abs(a.samples[0])).toBeLessThan(0.02 * peak);
    expect(a.loopRms).toBeGreaterThan(0.01);
    const opening = (r: { samples: Float32Array; overlap: number }) => rms(r.samples, 0, Math.round(Math.max(0.08, r.overlap) * SR));
    const raw = (await buildRelease(bank, { onset: [], nucleus: 'AA1', coda: ['D'] }, 220))!;
    const matched = (await buildRelease(bank, { onset: [], nucleus: 'AA1', coda: ['D'] }, 220, a.loopRms))!;
    const dbOff = Math.abs(20 * Math.log10(opening(matched) / a.loopRms));
    expect(dbOff).toBeLessThan(1);
    expect(Math.abs(20 * Math.log10(opening(raw) / a.loopRms))).toBeGreaterThanOrEqual(0); // (the raw level is whatever the recording had)
  });

  it('trims silence at the start of a sample (a stop\'s closure) so a key press sounds at once', async () => {
    const silent = new Float32Array(Math.round(0.1 * SR)); // 100 ms of nothing, then a vowel
    const tonePart = tone(200, 0.8);
    const x = new Float32Array(silent.length + tonePart.length);
    x.set(tonePart, silent.length);
    const bank = await Voicebank.load(
      [file('B/oto.ini', new TextEncoder().encode('v.wav=- A,0,200,-700,150,20\n')), file('B/v.wav', wavBytes(x)), file('B/v_wav.frq', frqBytes(frqFor(200, x.length, silent.length).f0))],
      'B',
    );
    const a = (await buildAttack(bank, { onset: [], nucleus: 'AA1', coda: [] }, 220, null))!;
    const peak = Math.max(...a.samples.map(Math.abs));
    let first = 0;
    while (first < a.samples.length && Math.abs(a.samples[first]) < peak * 0.05) first++;
    expect(first / SR).toBeLessThan(0.02); // sound within 20 ms, not after the 100 ms of silence
  });

  it('plays the consonant faster live so the vowel arrives sooner, without changing pitch', async () => {
    const bank = await liveBank();
    const plan = { onset: [], nucleus: 'AA1', coda: [] };
    const natural = (await buildAttack(bank, plan, 220, null, { consonantSpeed: 1 }))!;
    const fast = (await buildAttack(bank, plan, 220, null, { consonantSpeed: 1.5 }))!;
    // the sample's fixed (consonant) part is 50 ms: 1.5x saves about 17 ms
    expect((natural.samples.length - fast.samples.length) / SR).toBeGreaterThan(0.008);
    expect(Math.abs(estimateF0(fast.loop!.subarray(0, 9000), SR) / 220 - 1)).toBeLessThan(0.04);
    expect(fast.hasLoop).toBe(true);
  });

  it('finds the steady part of a vowel that glides first, so a diphthong is held on its first element', () => {
    // harmonics shaped by a "formant" that moves from 500 to 1500 Hz over 180 ms, then stays put for 300 ms
    const f0 = 200, n = Math.round(0.48 * SR), x = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      const t = i / SR, formant = t < 0.18 ? 500 + (1000 * t) / 0.18 : 1500;
      let v = 0;
      for (let h = 1; h * f0 < 4000; h++) v += Math.sin((2 * Math.PI * h * f0 * i) / SR) * Math.exp(-(((h * f0 - formant) / 350) ** 2));
      x[i] = 0.2 * v;
    }
    const w = steadyWindow(x, 0, n, SR);
    expect(w.start / SR).toBeGreaterThan(0.12); // not inside the glide
    expect(w.drift).toBeLessThanOrEqual(3.5);
    expect((w.end - w.start) / SR).toBeGreaterThanOrEqual(0.12);
    // a plain steady vowel is held from near its start
    const flat = steadyWindow(tone(200, 0.5), 0, Math.round(0.5 * SR), SR);
    expect(flat.start / SR).toBeLessThan(0.05);
  });

  it('keeps lowered pitches from turning into bursts with silence between them (no gating, no inflated peaks)', () => {
    const src = { samples: tone(280, 0.8), sampleRate: SR, frq: frqFor(280, Math.round(0.8 * SR)) };
    const out = renderSegment(src, { start: 0, end: src.samples.length, fixed: 0, length: Math.round(0.6 * SR), f0: 131, matchLevel: true });
    const period = Math.round(SR / 131), win = Math.round(0.0005 * SR);
    let deepest = 0, count = 0;
    for (let s = period * 8; s + period < out.length - period * 4 && count < 40; s += period, count++) {
      let hi = 0, lo = Infinity;
      for (let a = s; a + win <= s + period; a += win) {
        let e = 0;
        for (let i = a; i < a + win; i++) e += out[i] * out[i];
        const r = Math.sqrt(e / win);
        hi = Math.max(hi, r);
        lo = Math.min(lo, r);
      }
      deepest = Math.max(deepest, 20 * Math.log10((hi + 1e-9) / (lo + 1e-9)));
    }
    expect(deepest).toBeLessThan(30); // two-period grains left an 80 dB gap every cycle
  });

  it('caps buffer peaks at full scale without ever turning anything up', () => {
    const loud = Float32Array.from({ length: 100 }, (_, i) => Math.sin(i / 5) * 1.7);
    capPeak(loud);
    expect(Math.max(...loud.map(Math.abs))).toBeCloseTo(0.95, 5);
    const quiet = Float32Array.from({ length: 100 }, (_, i) => Math.sin(i / 5) * 0.3);
    capPeak(quiet);
    expect(Math.max(...quiet.map(Math.abs))).toBeCloseTo(0.3, 5);
  });

  it('smooths a loop seam', () => {
    const x = Float32Array.from({ length: 4000 }, (_, i) => Math.sin((2 * Math.PI * 300 * i) / SR) * (i < 2000 ? 1 : 0.5));
    const l = seamlessLoop(x, 400);
    expect(l).toHaveLength(3600);
    expect(Math.abs(l[0] - l[l.length - 1])).toBeLessThan(0.3);
  });

  it('builds a release from the final consonant sample, or from the vowel ending when there is no coda', async () => {
    const bank = await liveBank();
    const withCoda = (await buildRelease(bank, { onset: [], nucleus: 'AA1', coda: ['D'] }, 220))!;
    expect(withCoda.aliases).toEqual(['A d']);
    expect(withCoda.samples.length).toBeGreaterThan(0.05 * SR);
    expect(withCoda.overlap).toBeGreaterThanOrEqual(0.06);
    const noCoda = (await buildRelease(bank, { onset: [], nucleus: 'AA1', coda: [] }, 220))!;
    expect(noCoda.aliases).toEqual(['A -']);
    // a vowel ending is stretched so the word dies away gently: longer than the recorded part it came from
    expect(noCoda.samples.length / SR).toBeGreaterThan(0.2);
    expect(await buildRelease(bank, { onset: [], nucleus: 'IY1', coda: [] }, 220)).toBeNull(); // nothing for this vowel
    expect(await buildAttack(bank, { onset: [], nucleus: 'IY1', coda: [] }, 220, null)).toBeNull();
  });
});

// ---- Optional: a real voicebank on this machine -------------------------------------------------------------
describe('live sustain alignment', () => {
  it('finds the offset that puts a loop in step with the attack it follows, so the crossfade does not cancel', () => {
    const f0 = 220;
    const loop = tone(f0, 1);
    const shift = 57; // samples: the attack's tail is the same vowel at a different phase
    const tail = loop.slice(shift, shift + 660);
    const lag = alignOffset(tail, loop, Math.ceil((1.25 * SR) / f0));
    expect(lag).toBe(shift);
    // Linear crossfade of the aligned pair keeps the level; unaligned (in antiphase) it collapses.
    const mix = (off: number) => {
      let e = 0;
      for (let i = 0; i < 660; i++) { const g = i / 660; const v = tail[i] * (1 - g) + loop[(i + off) % loop.length] * g; e += v * v; }
      return Math.sqrt(e / 660);
    };
    const half = Math.round(SR / f0 / 2);
    expect(mix(lag)).toBeGreaterThan(mix(shift + half) * 1.3);
  });
});

const bankDir = process.env.HILLSING_UTAU_BANK;

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? walk(p) : [p];
  });
}

describe.skipIf(!bankDir)('real voicebank (HILLSING_UTAU_BANK)', () => {
  it('loads, finds every referenced WAV, and renders aliases at the requested pitch', async () => {
    const root = bankDir!;
    const files: BankFile[] = walk(root).map((p) => ({ path: relative(root, p).split('\\').join('/'), read: async () => new Uint8Array(readFileSync(p)) }));
    const bank = await Voicebank.load(files, 'real');
    expect(bank.entries.length).toBeGreaterThan(100);
    const missing = new Set<string>();
    for (const e of bank.entries) if (!(await bank.source(e.wavPath))) missing.add(e.wavPath);
    console.log(`bank: ${bank.entries.length} entries, ${bank.aliases().length} aliases, ${bank.otoFiles.length} oto.ini, ${missing.size} missing wavs`);
    expect([...missing].slice(0, 5)).toEqual([]);

    // Voiced sample rendered at several pitches: measured pitch must follow the request.
    const alias = bank.aliases().find((a) => /^[a-zA-Z@{3]+$/.test(a) && a.length <= 3 && bank.entry(a)!.cutoff < 0) ?? bank.aliases()[0];
    for (const target of [180, 220, 262, 330]) {
      const out = (await bank.renderAlias(alias, target, 900))!;
      const a = Math.round(out.sampleRate * 0.45), b = Math.round(out.sampleRate * 0.8);
      const f = estimateF0(out.samples.subarray(a, b), out.sampleRate);
      console.log(`alias "${alias}" @${target} Hz -> measured ${f.toFixed(1)} Hz, rms ${rms(out.samples, a, b).toFixed(3)}`);
      expect(Math.abs(f / target - 1), `${alias} at ${target}`).toBeLessThan(0.05);
    }
  });

  it('sings English lyrics: every word finds aliases, pitch follows the notes, levels are sane', async () => {
    const root = bankDir!;
    const files: BankFile[] = walk(root).map((p) => ({ path: relative(root, p).split('\\').join('/'), read: async () => new Uint8Array(readFileSync(p)) }));
    const bank = await Voicebank.load(files, 'real');
    const text = 'hello world I love to sing a little song';
    const lyrics = buildLyrics(text, realLookup());
    const pitches = [60, 62, 64, 65, 67, 65, 64, 62, 60, 64, 67, 72];
    const notes = lyrics.syllables.map((_, i) => ({ start: i * 0.6, end: i * 0.6 + 0.5, note: pitches[i % pitches.length], velocity: 0.8, syllable: i, legato: false }));
    const r = await renderUtau(bank, notes, lyrics.syllables);
    console.log('aliases:\n' + r.log.join('\n'));
    console.log(`missing: ${r.missing.join(', ') || 'none'}; length ${(r.samples.length / r.sampleRate).toFixed(2)} s, lead ${r.leadSec.toFixed(3)} s`);
    expect(r.missing).toEqual([]);
    expect(r.samples.every(Number.isFinite)).toBe(true);
    let pk = 0;
    for (const v of r.samples) pk = Math.max(pk, Math.abs(v));
    expect(pk).toBeGreaterThan(0.1);
    expect(pk).toBeLessThanOrEqual(0.96);
    // each note's vowel sustains at about the note's pitch
    let ok = 0, tested = 0;
    for (const n of notes) {
      const a = Math.round((r.leadSec + n.start + 0.12) * r.sampleRate), b = Math.round((r.leadSec + n.start + 0.32) * r.sampleRate); // the vowel core, clear of joins and endings
      if (b - a < 1500) continue;
      const seg = r.samples.subarray(a, b);
      if (rms(seg) < 0.02) continue; // an unvoiced or silent stretch
      const target = 440 * Math.pow(2, (n.note - 69) / 12);
      const f = estimateF0(seg, r.sampleRate, target * 0.6, target * 1.6);
      tested++;
      if (Math.abs(f / target - 1) < 0.06) ok++;
      else console.log(`  note ${n.note} (${target.toFixed(0)} Hz) measured ${f.toFixed(0)} Hz`);
    }
    console.log(`pitch follows the notes on ${ok} of ${tested} vowels`);
    expect(ok).toBeGreaterThan(tested * 0.8);
  });

  it('builds live notes fast enough for a key press, with the loop at the played pitch', async () => {
    const root = bankDir!;
    const files: BankFile[] = walk(root).map((p) => ({ path: relative(root, p).split('\\').join('/'), read: async () => new Uint8Array(readFileSync(p)) }));
    const bank = await Voicebank.load(files, 'real');
    const t0 = performance.now();
    await bank.preload();
    console.log(`preload: ${(performance.now() - t0).toFixed(0)} ms for ${new Set(bank.entries.map((e) => e.wavPath)).size} files`);
    const syllables = buildLyrics('hello world sing', realLookup()).syllables;
    const times: number[] = [];
    for (const [i, syl] of syllables.entries()) {
      const note = [60, 64, 67, 72][i % 4];
      const f0 = 440 * Math.pow(2, (note - 69) / 12);
      const plan = { onset: syl.onset, nucleus: syl.nucleus, coda: syl.coda };
      const t1 = performance.now();
      const a = await buildAttack(bank, plan, f0, null);
      const r = await buildRelease(bank, plan, f0);
      const dt = performance.now() - t1;
      times.push(dt);
      expect(a, syl.label).not.toBeNull();
      console.log(`${syl.label.padEnd(6)} attack ${a!.aliases.join('+').padEnd(14)} ${(a!.samples.length / SR).toFixed(2)} s, loop ${(a!.loop ? a!.loop.length / SR : 0).toFixed(2)} s; release ${r ? r.aliases.join('+') : '-'}; ${dt.toFixed(1)} ms`);
      if (a!.loop) {
        const f = estimateF0(a!.loop.subarray(0, Math.min(a!.loop.length, 9000)), SR, f0 * 0.6, f0 * 1.6);
        expect(Math.abs(f / f0 - 1), `${syl.label} loop pitch`).toBeLessThan(0.06);
      }
    }
    expect(Math.max(...times)).toBeLessThan(80);
    for (const syl of syllables) {
      const a = (await buildAttack(bank, { onset: syl.onset, nucleus: syl.nucleus, coda: syl.coda }, 262, null))!;
      if (a.loop) expect(a.loop.length / SR, `${syl.label} sustain length`).toBeGreaterThan(4);
    }
  });

  it('hands over from the sustain to the release without a jump in volume', async () => {
    const root = bankDir!;
    const files: BankFile[] = walk(root).map((p) => ({ path: relative(root, p).split('\\').join('/'), read: async () => new Uint8Array(readFileSync(p)) }));
    const bank = await Voicebank.load(files, 'real');
    const syllables = buildLyrics('hello world sing love ah song', realLookup()).syllables;
    let worstBefore = 0, worstAfter = 0;
    for (const syl of syllables) {
      const plan = { onset: syl.onset, nucleus: syl.nucleus, coda: syl.coda };
      const f0 = 196;
      const a = await buildAttack(bank, plan, f0, null);
      const raw = await buildRelease(bank, plan, f0);
      const fixed = await buildRelease(bank, plan, f0, a!.loopRms);
      if (!a || !raw || !fixed) continue;
      const open = (r: { samples: Float32Array; overlap: number }) => rms(r.samples, 0, Math.round(Math.max(0.08, r.overlap) * SR));
      const before = Math.abs(20 * Math.log10(open(raw) / a.loopRms));
      const after = Math.abs(20 * Math.log10(open(fixed) / a.loopRms));
      worstBefore = Math.max(worstBefore, before);
      worstAfter = Math.max(worstAfter, after);
      console.log(`${syl.label.padEnd(6)} release ${fixed.aliases.join('+').padEnd(10)} level step at handover: ${before.toFixed(1)} dB before matching, ${after.toFixed(1)} dB after`);
    }
    console.log(`worst: ${worstBefore.toFixed(1)} dB before, ${worstAfter.toFixed(1)} dB after`);
    expect(worstAfter).toBeLessThan(2.5);
  });
});

const bankZip = process.env.HILLSING_UTAU_ZIP;
describe.skipIf(!bankZip)('real voicebank zip (HILLSING_UTAU_ZIP)', () => {
  it('loads straight from the zip and sings', async () => {
    const files = await openZip(new Blob([new Uint8Array(readFileSync(bankZip!))]));
    const bank = await Voicebank.load(files, 'zip');
    console.log(`zip: ${files.length} files, ${bank.entries.length} entries, ${bank.aliases().length} aliases`);
    expect(bank.entries.length).toBeGreaterThan(2000);
    const out = (await bank.renderAlias('- h@', 262, 600))!;
    expect(out.samples.length).toBeGreaterThan(10000);
    const f = estimateF0(out.samples.subarray(Math.round(0.3 * out.sampleRate), Math.round(0.55 * out.sampleRate)), out.sampleRate);
    expect(Math.abs(f / 262 - 1)).toBeLessThan(0.05);
  });
});
