// MP3 export. The encoder (lamejs, LGPL-3.0) is loaded only when an MP3 is asked for, so it adds nothing to start-up.

import { joinChunks } from './wav';

const FRAME = 1152; // samples per MP3 frame

/**
 * Encode two channels of 16-bit samples to MP3. Works in slices and lets the page breathe between them, so a long
 * recording does not freeze the window; `progress` is called with 0..1.
 */
export async function encodeMp3(
  left: ReadonlyArray<Int16Array>,
  right: ReadonlyArray<Int16Array>,
  sampleRate: number,
  kbps = 192,
  progress: (fraction: number) => void = () => {},
): Promise<Uint8Array> {
  const { Mp3Encoder } = await import('@breezystack/lamejs');
  const l = joinChunks(left);
  const r = joinChunks(right);
  const n = Math.min(l.length, r.length);
  const encoder = new Mp3Encoder(2, sampleRate, kbps);
  const parts: Uint8Array[] = [];
  const slice = FRAME * 48;
  for (let at = 0; at < n; at += slice) {
    const end = Math.min(n, at + slice);
    const chunk = encoder.encodeBuffer(l.subarray(at, end), r.subarray(at, end));
    if (chunk.length) parts.push(chunk);
    progress(end / n);
    if ((at / slice) % 8 === 7) await new Promise((res) => setTimeout(res, 0));
  }
  const tail = encoder.flush();
  if (tail.length) parts.push(tail);
  let total = 0;
  for (const p of parts) total += p.length;
  const out = new Uint8Array(total);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  progress(1);
  return out;
}
