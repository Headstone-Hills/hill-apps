// Builds Standard MIDI File bytes for tests.
export function vlq(n: number): number[] {
  const out = [n & 0x7f];
  while ((n >>= 7) > 0) out.unshift((n & 0x7f) | 0x80);
  return out;
}

/** Events are [deltaTicks, ...bytes]. An end-of-track meta is appended unless `end` is false. */
export function track(events: number[][], end = true): number[] {
  const body: number[] = [];
  for (const [delta, ...bytes] of events) body.push(...vlq(delta), ...bytes);
  if (end) body.push(0, 0xff, 0x2f, 0);
  return [0x4d, 0x54, 0x72, 0x6b, ...be32(body.length), ...body];
}

const be32 = (n: number): number[] => [(n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255];
const be16 = (n: number): number[] => [(n >> 8) & 255, n & 255];

export function smf(opts: { format?: number; division?: number; tracks: number[][] }): Uint8Array {
  const { format = 1, division = 480, tracks } = opts;
  return Uint8Array.from([
    0x4d, 0x54, 0x68, 0x64, ...be32(6), ...be16(format), ...be16(tracks.length), ...be16(division),
    ...tracks.flat(),
  ]);
}

export const noteOn = (delta: number, note: number, vel = 100, ch = 0): number[] => [delta, 0x90 | ch, note, vel];
export const noteOff = (delta: number, note: number, ch = 0): number[] => [delta, 0x80 | ch, note, 0];
export const tempo = (delta: number, usPerQuarter: number): number[] => [
  delta, 0xff, 0x51, 3, (usPerQuarter >> 16) & 255, (usPerQuarter >> 8) & 255, usPerQuarter & 255,
];
export const trackName = (name: string): number[] => [0, 0xff, 0x03, name.length, ...[...name].map((c) => c.charCodeAt(0))];
