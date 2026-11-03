// Repair file and folder names that were unzipped with the wrong text encoding. Japanese voicebank archives store
// names in Shift-JIS; an unzip tool that assumes Windows-1252 turns "暗鳴ニュイ" into "ˆÃ–Âƒjƒ…ƒC" on disk, and then
// the names in oto.ini ("息1.wav") no longer match the files. Reading each character back to the byte it came from
// and decoding those bytes as Shift-JIS undoes it.

const CP1252_HIGH: Record<number, number> = {
  0x20ac: 0x80, 0x201a: 0x82, 0x0192: 0x83, 0x201e: 0x84, 0x2026: 0x85, 0x2020: 0x86, 0x2021: 0x87, 0x02c6: 0x88,
  0x2030: 0x89, 0x0160: 0x8a, 0x2039: 0x8b, 0x0152: 0x8c, 0x017d: 0x8e, 0x2018: 0x91, 0x2019: 0x92, 0x201c: 0x93,
  0x201d: 0x94, 0x2022: 0x95, 0x2013: 0x96, 0x2014: 0x97, 0x02dc: 0x98, 0x2122: 0x99, 0x0161: 0x9a, 0x203a: 0x9b,
  0x0153: 0x9c, 0x017e: 0x9e, 0x0178: 0x9f,
};

const JAPANESE = /[぀-ヿ㐀-鿿ｦ-ﾟ]/;

/** The Shift-JIS reading of a mis-decoded name, or null when the name is fine or cannot be repaired. */
export function repairShiftJis(name: string): string | null {
  if (!/[^\u0000-\u007f]/.test(name) || JAPANESE.test(name)) return null; // plain ASCII, or already real Japanese
  const bytes: number[] = [];
  for (const ch of name) {
    const c = ch.codePointAt(0)!;
    if (c < 0x80 || (c >= 0xa0 && c <= 0xff)) bytes.push(c);
    else if (CP1252_HIGH[c] !== undefined) bytes.push(CP1252_HIGH[c]);
    else return null; // a character that no single byte produced (such as a lost one): leave the name alone
  }
  try {
    const fixed = new TextDecoder('shift_jis', { fatal: true }).decode(new Uint8Array(bytes));
    return JAPANESE.test(fixed) && !fixed.includes('�') ? fixed : null;
  } catch {
    return null;
  }
}

/** The name as it should read: repaired if it was mangled, otherwise unchanged. */
export const repairName = (name: string): string => repairShiftJis(name) ?? name;

/** `path` with only its file name repaired (folders keep the names they have on disk, so paths stay consistent). */
export function repairFileNameInPath(path: string): string | null {
  const slash = path.lastIndexOf('/');
  const base = path.slice(slash + 1);
  const fixed = repairShiftJis(base);
  return fixed === null ? null : path.slice(0, slash + 1) + fixed;
}
