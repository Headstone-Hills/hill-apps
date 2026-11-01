import { describe, expect, it } from 'vitest';
import { encodeMp3 } from '../src/audio/mp3';
import { encodeWav, floatToInt16, joinChunks } from '../src/audio/wav';

const sine = (hz: number, seconds: number, sr: number): Float32Array => Float32Array.from({ length: Math.round(seconds * sr) }, (_, i) => 0.5 * Math.sin((2 * Math.PI * hz * i) / sr));

describe('recording export', () => {
  it('converts floats to 16-bit with clamping', () => {
    expect(Array.from(floatToInt16(Float32Array.from([0, 1, -1, 2, -2, 0.5])))).toEqual([0, 32767, -32768, 32767, -32768, 16384]);
  });

  it('joins chunks in order', () => {
    expect(Array.from(joinChunks([Int16Array.from([1, 2]), Int16Array.from([3])]))).toEqual([1, 2, 3]);
  });

  it('writes a valid stereo 16-bit WAV: header fields, length and interleaving', () => {
    const l = [Int16Array.from([1, 2, 3]), Int16Array.from([4])];
    const r = [Int16Array.from([-1, -2]), Int16Array.from([-3, -4])];
    const wav = encodeWav(l, r, 48000);
    const v = new DataView(wav.buffer);
    const tag = (o: number) => String.fromCharCode(wav[o], wav[o + 1], wav[o + 2], wav[o + 3]);
    expect([tag(0), tag(8), tag(12), tag(36)]).toEqual(['RIFF', 'WAVE', 'fmt ', 'data']);
    expect(wav.length).toBe(44 + 4 * 4);
    expect(v.getUint32(4, true)).toBe(wav.length - 8);
    expect(v.getUint16(22, true)).toBe(2);
    expect(v.getUint32(24, true)).toBe(48000);
    expect(v.getUint16(34, true)).toBe(16);
    expect(v.getUint32(40, true)).toBe(16);
    const samples = Array.from({ length: 8 }, (_, i) => v.getInt16(44 + i * 2, true));
    expect(samples).toEqual([1, -1, 2, -2, 3, -3, 4, -4]); // left, right, left, right...
  });

  it('encodes MP3: frame sync at the start, and about the expected size for the bit rate', async () => {
    const sr = 44100;
    const tone = floatToInt16(sine(440, 2, sr));
    const mp3 = await encodeMp3([tone], [tone], sr, 128);
    expect(mp3[0]).toBe(0xff);
    expect(mp3[1] & 0xe0).toBe(0xe0); // the 11 sync bits
    // 2 s at 128 kb/s is about 32 kB
    expect(mp3.length).toBeGreaterThan(28000);
    expect(mp3.length).toBeLessThan(36000);
  });

  it('reports progress up to 1', async () => {
    const sr = 44100;
    const tone = floatToInt16(sine(220, 1, sr));
    const seen: number[] = [];
    await encodeMp3([tone], [tone], sr, 96, (f) => seen.push(f));
    expect(seen[seen.length - 1]).toBe(1);
    for (let i = 1; i < seen.length; i++) expect(seen[i]).toBeGreaterThanOrEqual(seen[i - 1]);
  });
});
