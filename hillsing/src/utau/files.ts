// Readers for the three file types in a UTAU voicebank: oto.ini (text, usually Shift-JIS), WAV samples
// and .frq pitch analyses. Pure functions over bytes, so they run in the browser and in Node tests.
// Nothing here ships any voicebank: banks are loaded from the user's own folder.

export interface OtoEntry {
  /** Path of the WAV relative to the voicebank root, '/' separated. */
  wavPath: string;
  alias: string;
  /** All in milliseconds, as in oto.ini. */
  offset: number;
  consonant: number;
  /** Positive: end position from the start of the file. Zero or negative: -value from the end. */
  cutoff: number;
  preutter: number;
  overlap: number;
}

/** oto.ini is Shift-JIS in most banks and UTF-8 in some; try strict UTF-8 first. */
export function decodeText(bytes: Uint8Array): string {
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes).replace(/^﻿/, '');
  } catch {
    return new TextDecoder('shift_jis').decode(bytes);
  }
}

/** Parse oto.ini text. `dir` is the folder the oto.ini sits in (with a trailing '/', or ''). */
export function parseOto(text: string, dir = ''): OtoEntry[] {
  const out: OtoEntry[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    const eq = line.indexOf('=');
    if (eq <= 0) continue;
    const wav = line.slice(0, eq).trim();
    const f = line.slice(eq + 1).split(',');
    const num = (i: number): number => {
      const v = parseFloat(f[i] ?? '');
      return Number.isFinite(v) ? v : 0;
    };
    // An empty alias means "the file name without its extension".
    const alias = (f[0] ?? '').trim() || wav.replace(/\.wav$/i, '');
    out.push({ wavPath: dir + wav, alias, offset: num(1), consonant: num(2), cutoff: num(3), preutter: num(4), overlap: num(5) });
  }
  return out;
}

export interface Wav {
  sampleRate: number;
  /** Mono (channels averaged), -1..1. */
  samples: Float32Array;
}

/** Minimal RIFF/WAVE reader: PCM 8/16/24/32-bit and 32-bit float, any channel count. */
export function parseWav(bytes: Uint8Array): Wav {
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tag = (o: number): string => String.fromCharCode(bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]);
  if (bytes.length < 12 || tag(0) !== 'RIFF' || tag(8) !== 'WAVE') throw new Error('Not a WAV file');
  let fmt: { format: number; channels: number; rate: number; bits: number } | null = null;
  let dataAt = -1;
  let dataLen = 0;
  for (let p = 12; p + 8 <= bytes.length; ) {
    const size = v.getUint32(p + 4, true);
    if (tag(p) === 'fmt ') {
      fmt = { format: v.getUint16(p + 8, true), channels: v.getUint16(p + 10, true), rate: v.getUint32(p + 12, true), bits: v.getUint16(p + 22, true) };
      if (fmt.format === 0xfffe && size >= 26) fmt.format = v.getUint16(p + 8 + 24, true); // WAVE_FORMAT_EXTENSIBLE
    } else if (tag(p) === 'data') {
      dataAt = p + 8;
      dataLen = Math.min(size, bytes.length - dataAt);
      break;
    }
    p += 8 + size + (size & 1);
  }
  if (!fmt || dataAt < 0) throw new Error('WAV has no fmt/data chunk');
  const { channels, bits, format, rate } = fmt;
  const bps = bits / 8;
  const frames = Math.floor(dataLen / (bps * channels));
  const out = new Float32Array(frames);
  const read = (o: number): number => {
    if (format === 3 && bits === 32) return v.getFloat32(o, true);
    if (bits === 16) return v.getInt16(o, true) / 32768;
    if (bits === 8) return (bytes[o] - 128) / 128;
    if (bits === 24) return (((bytes[o] | (bytes[o + 1] << 8) | (bytes[o + 2] << 16)) << 8) >> 8) / 8388608;
    if (bits === 32) return v.getInt32(o, true) / 2147483648;
    throw new Error(`Unsupported WAV bit depth ${bits}`);
  };
  for (let i = 0; i < frames; i++) {
    let s = 0;
    for (let c = 0; c < channels; c++) s += read(dataAt + (i * channels + c) * bps);
    out[i] = s / channels;
  }
  return { sampleRate: rate, samples: out };
}

export interface Frq {
  /** Samples per analysis frame (256 in UTAU's tools). */
  interval: number;
  /** Fundamental frequency per frame in Hz; 0 (or tiny) means unvoiced. */
  f0: Float64Array;
}

/** UTAU .frq: "FREQ0003", int32 interval, double average f0, 16 reserved bytes, int32 count, then (f0, amplitude) doubles. */
export function parseFrq(bytes: Uint8Array): Frq {
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const magic = String.fromCharCode(...bytes.subarray(0, 8));
  if (!magic.startsWith('FREQ') || bytes.length < 40) throw new Error('Not a .frq file');
  const interval = v.getInt32(8, true) || 256;
  const count = Math.min(v.getInt32(36, true), Math.floor((bytes.length - 40) / 16));
  const f0 = new Float64Array(count);
  for (let i = 0; i < count; i++) {
    const hz = v.getFloat64(40 + i * 16, true);
    f0[i] = Number.isFinite(hz) && hz > 40 && hz < 1500 ? hz : 0;
  }
  return { interval, f0 };
}

/** Path of the .frq that UTAU writes next to `foo.wav`: "foo_wav.frq". */
export const frqPathFor = (wavPath: string): string => wavPath.replace(/\.wav$/i, '_wav.frq');
