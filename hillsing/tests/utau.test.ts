// UTAU sample playback, stage 1. These tests use synthetic data only: voicebanks are copyrighted and
// must never be added to this repository. A second block runs against a real bank when
// HILLSING_UTAU_BANK points at its folder (skipped otherwise).

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';
import { decodeText, frqPathFor, parseFrq, parseOto, parseWav } from '../src/utau/files';
import { estimateF0, mixInto, pitchMarks, renderSegment, type PsolaSource } from '../src/utau/psola';
import { Voicebank, type BankFile } from '../src/utau/voicebank';

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

// ---- Optional: a real voicebank on this machine -------------------------------------------------------------
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
});
