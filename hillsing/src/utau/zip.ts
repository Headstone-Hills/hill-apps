// Read a voicebank straight from a .zip, in the browser or in Node. Nothing is extracted to disk and nothing
// is uploaded: the archive's index is parsed, and each file is sliced out and inflated only when it is used.
// This is how a voicebank is loaded on a phone, where there is no folder picker: zip the bank folder once,
// keep the zip on the phone, and pick it in the page.

import type { BankFile } from './voicebank';

const EOCD_SIG = 0x06054b50;
const CEN_SIG = 0x02014b50;
const LOC_SIG = 0x04034b50;

/** Names are UTF-8 when flagged (or when they happen to decode as UTF-8); older Japanese tools wrote Shift-JIS. */
function decodeName(bytes: Uint8Array, flags: number): string {
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    if (flags & 0x800) return new TextDecoder().decode(bytes);
    return new TextDecoder('shift_jis').decode(bytes);
  }
}

async function inflateRaw(data: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([data as BlobPart]).stream().pipeThrough(new DecompressionStream('deflate-raw'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

/** List the files in a zip as BankFiles that read (and inflate) on demand. */
export async function openZip(zip: Blob): Promise<BankFile[]> {
  // End of central directory record: within the last 64 KiB + 22 bytes.
  const tailLen = Math.min(zip.size, 65557);
  const tail = new Uint8Array(await zip.slice(zip.size - tailLen).arrayBuffer());
  const tv = new DataView(tail.buffer);
  let eocd = -1;
  for (let i = tail.length - 22; i >= 0; i--) {
    if (tv.getUint32(i, true) === EOCD_SIG) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw new Error('This does not look like a .zip file.');
  const entries = tv.getUint16(eocd + 10, true);
  const cdSize = tv.getUint32(eocd + 12, true);
  const cdOffset = tv.getUint32(eocd + 16, true);
  if (cdOffset === 0xffffffff || cdSize === 0xffffffff || entries === 0xffff) {
    throw new Error('This zip uses the Zip64 format, which is not supported. Re-zip the folder with the standard Archive Utility.');
  }

  const cd = new Uint8Array(await zip.slice(cdOffset, cdOffset + cdSize).arrayBuffer());
  const v = new DataView(cd.buffer);
  const files: BankFile[] = [];
  for (let p = 0; p + 46 <= cd.length && v.getUint32(p, true) === CEN_SIG; ) {
    const flags = v.getUint16(p + 8, true);
    const method = v.getUint16(p + 10, true);
    const compSize = v.getUint32(p + 20, true);
    const nameLen = v.getUint16(p + 28, true);
    const extraLen = v.getUint16(p + 30, true);
    const commentLen = v.getUint16(p + 32, true);
    const localOffset = v.getUint32(p + 42, true);
    const name = decodeName(cd.subarray(p + 46, p + 46 + nameLen), flags).replace(/\\/g, '/');
    p += 46 + nameLen + extraLen + commentLen;
    if (name.endsWith('/') || name.startsWith('__MACOSX/') || /(^|\/)\._/.test(name)) continue; // folders and macOS resource-fork junk
    if (flags & 1) throw new Error('This zip is password-protected.');
    if (method !== 0 && method !== 8) throw new Error(`"${name}" uses an unsupported zip compression method (${method}).`);
    files.push({
      path: name,
      read: async () => {
        const head = new DataView(await zip.slice(localOffset, localOffset + 30).arrayBuffer());
        if (head.getUint32(0, true) !== LOC_SIG) throw new Error(`Damaged zip entry: ${name}`);
        const start = localOffset + 30 + head.getUint16(26, true) + head.getUint16(28, true);
        const raw = new Uint8Array(await zip.slice(start, start + compSize).arrayBuffer());
        return method === 0 ? raw : inflateRaw(raw);
      },
    });
  }
  if (files.length === 0) throw new Error('That zip has no files in it.');
  return files;
}
