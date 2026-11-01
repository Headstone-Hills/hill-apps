// Writing a recording out as a WAV file (16-bit PCM, stereo). Pure functions, no Web Audio, so they are testable in Node.

/** Join chunks of samples into one array. */
export function joinChunks(chunks: ReadonlyArray<Int16Array>): Int16Array {
  let n = 0;
  for (const c of chunks) n += c.length;
  const out = new Int16Array(n);
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.length;
  }
  return out;
}

/** Float samples (-1..1) to 16-bit, clamped. */
export function floatToInt16(x: Float32Array): Int16Array {
  const out = new Int16Array(x.length);
  for (let i = 0; i < x.length; i++) {
    const v = Math.max(-1, Math.min(1, x[i]));
    out[i] = v < 0 ? Math.round(v * 32768) : Math.round(v * 32767);
  }
  return out;
}

/** A complete WAV file for two channels of 16-bit samples. */
export function encodeWav(left: ReadonlyArray<Int16Array>, right: ReadonlyArray<Int16Array>, sampleRate: number): Uint8Array {
  const l = joinChunks(left);
  const r = joinChunks(right);
  const frames = Math.min(l.length, r.length);
  const dataBytes = frames * 4;
  const out = new Uint8Array(44 + dataBytes);
  const v = new DataView(out.buffer);
  const tag = (o: number, s: string): void => {
    for (let i = 0; i < s.length; i++) out[o + i] = s.charCodeAt(i);
  };
  tag(0, 'RIFF');
  v.setUint32(4, 36 + dataBytes, true);
  tag(8, 'WAVE');
  tag(12, 'fmt ');
  v.setUint32(16, 16, true); // size of the format block
  v.setUint16(20, 1, true); // PCM
  v.setUint16(22, 2, true); // channels
  v.setUint32(24, sampleRate, true);
  v.setUint32(28, sampleRate * 4, true); // bytes per second
  v.setUint16(32, 4, true); // bytes per frame
  v.setUint16(34, 16, true); // bits per sample
  tag(36, 'data');
  v.setUint32(40, dataBytes, true);
  for (let i = 0; i < frames; i++) {
    v.setInt16(44 + i * 4, l[i], true);
    v.setInt16(46 + i * 4, r[i], true);
  }
  return out;
}
