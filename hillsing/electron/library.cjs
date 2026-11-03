// The voicebank library: a folder you pick once (an external drive works), where each voicebank is a sub-folder (the
// one with oto.ini in it) or a .zip. Plain Node, no Electron, so the scanning rules are testable.

const fs = require('node:fs');
const path = require('node:path');

// Names unzipped with the wrong encoding (Shift-JIS read as Windows-1252, "ˆÃ–Âƒjƒ…ƒC") are shown repaired.
const CP1252_HIGH = {
  0x20ac: 0x80, 0x201a: 0x82, 0x0192: 0x83, 0x201e: 0x84, 0x2026: 0x85, 0x2020: 0x86, 0x2021: 0x87, 0x02c6: 0x88,
  0x2030: 0x89, 0x0160: 0x8a, 0x2039: 0x8b, 0x0152: 0x8c, 0x017d: 0x8e, 0x2018: 0x91, 0x2019: 0x92, 0x201c: 0x93,
  0x201d: 0x94, 0x2022: 0x95, 0x2013: 0x96, 0x2014: 0x97, 0x02dc: 0x98, 0x2122: 0x99, 0x0161: 0x9a, 0x203a: 0x9b,
  0x0153: 0x9c, 0x017e: 0x9e, 0x0178: 0x9f,
};
const JAPANESE = /[\u3040-\u30ff\u3400-\u9fff\uff66-\uff9f]/;
function repairName(name) {
  if (!/[^\u0000-\u007f]/.test(name) || JAPANESE.test(name)) return name;
  const bytes = [];
  for (const ch of name) {
    const c = ch.codePointAt(0);
    if (c < 0x80 || (c >= 0xa0 && c <= 0xff)) bytes.push(c);
    else if (CP1252_HIGH[c] !== undefined) bytes.push(CP1252_HIGH[c]);
    else return name;
  }
  try {
    const fixed = new TextDecoder('shift_jis', { fatal: true }).decode(new Uint8Array(bytes));
    return JAPANESE.test(fixed) && !fixed.includes('\ufffd') ? fixed : name;
  } catch {
    return name;
  }
}

const MAX_DEPTH = 3; // how far below a candidate folder oto.ini may sit (banks often nest one or two levels)

const hidden = (name) => name.startsWith('.') || name === '__MACOSX';

/** 'dir', 'file' or null, following symlinks (a bank is often kept behind a link). */
function kindOf(full) {
  try {
    const st = fs.statSync(full);
    return st.isDirectory() ? 'dir' : st.isFile() ? 'file' : null;
  } catch {
    return null;
  }
}

/** True when `target` is `root` or something inside it (no escaping with ".."). */
function isInside(root, target) {
  const r = path.resolve(root);
  const t = path.resolve(target);
  return t === r || t.startsWith(r + path.sep);
}

/** Does this folder hold an oto.ini, at most `depth` levels down? */
function hasOto(dir, depth = MAX_DEPTH) {
  let items;
  try {
    items = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return false;
  }
  for (const it of items) if (it.name.toLowerCase() === 'oto.ini' && kindOf(path.join(dir, it.name)) === 'file') return true;
  if (depth <= 0) return false;
  for (const it of items) {
    if (!hidden(it.name) && kindOf(path.join(dir, it.name)) === 'dir' && hasOto(path.join(dir, it.name), depth - 1)) return true;
  }
  return false;
}

/**
 * The voicebanks in a library folder: each sub-folder that contains an oto.ini and each .zip, sorted by name. If the
 * folder itself is a voicebank, it is the one entry.
 */
function scanLibrary(root) {
  let st;
  try {
    st = fs.statSync(root);
  } catch {
    return { ok: false, error: 'not-found', entries: [] };
  }
  if (!st.isDirectory()) return { ok: false, error: 'not-a-folder', entries: [] };
  const entries = [];
  if (hasOto(root, 0)) {
    entries.push({ kind: 'folder', name: path.basename(root), path: root });
    return { ok: true, entries };
  }
  let items = [];
  try {
    items = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    return { ok: false, error: 'unreadable', entries: [] };
  }
  for (const it of items) {
    if (hidden(it.name)) continue;
    const full = path.join(root, it.name);
    const kind = kindOf(full);
    if (kind === 'dir' && hasOto(full)) entries.push({ kind: 'folder', name: repairName(it.name), path: full });
    else if (kind === 'dir') {
      // A folder of archives ("Fuu/Fuusenne V.2 CVVC JP.zip"): each zip inside is a voicebank, listed under the folder's name.
      let inner = [];
      try {
        inner = fs.readdirSync(full, { withFileTypes: true });
      } catch {
        inner = [];
      }
      for (const z of inner) {
        const zp = path.join(full, z.name);
        if (hidden(z.name) || kindOf(zp) !== 'file' || !/\.zip$/i.test(z.name)) continue;
        let size = 0;
        try {
          size = fs.statSync(zp).size;
        } catch {
          /* unreadable: list it with no size */
        }
        entries.push({ kind: 'zip', name: `${repairName(it.name)} › ${repairName(z.name.replace(/\.zip$/i, ''))}`, path: zp, size });
      }
    } else if (kind === 'file' && /\.zip$/i.test(it.name)) {
      let size = 0;
      try {
        size = fs.statSync(full).size;
      } catch {
        /* unreadable: list it with no size */
      }
      entries.push({ kind: 'zip', name: repairName(it.name.replace(/\.zip$/i, '')), path: full, size });
    }
  }
  entries.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
  return { ok: true, entries };
}

/** Every file under a voicebank folder, as paths relative to it ('/' separated), skipping hidden files. */
function listBankFiles(dir) {
  const out = [];
  const walk = (d, rel) => {
    let items;
    try {
      items = fs.readdirSync(d, { withFileTypes: true });
    } catch {
      return;
    }
    for (const it of items) {
      if (hidden(it.name)) continue;
      const r = rel ? `${rel}/${it.name}` : it.name;
      const kind = kindOf(path.join(d, it.name));
      if (kind === 'dir') walk(path.join(d, it.name), r);
      else if (kind === 'file') out.push(r);
    }
  };
  walk(dir, '');
  return out;
}

module.exports = { isInside, hasOto, scanLibrary, listBankFiles, repairName };
