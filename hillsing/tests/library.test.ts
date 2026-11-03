import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterAll, describe, expect, it } from 'vitest';

const { isInside, listBankFiles, scanLibrary } = createRequire(import.meta.url)('../electron/library.cjs') as {
  isInside(root: string, target: string): boolean;
  listBankFiles(dir: string): string[];
  scanLibrary(root: string): { ok: boolean; error?: string; entries: Array<{ kind: string; name: string; path: string; size?: number }> };
};

const root = mkdtempSync(join(tmpdir(), 'hillsing-lib-'));
afterAll(() => rmSync(root, { recursive: true, force: true }));
const put = (rel: string, text = 'x'): void => {
  const full = join(root, rel);
  mkdirSync(join(full, '..'), { recursive: true });
  writeFileSync(full, text);
};

describe('voicebank library folder', () => {
  it('finds banks as folders containing oto.ini (even nested) and as zips, sorted by name, ignoring junk', () => {
    put('Zed Bank/oto.ini');
    put('alpha/Voice/oto.ini'); // nested one level
    put('deep/a/b/c/d/oto.ini'); // four levels down: too deep to count as a bank
    put('not a bank/readme.txt');
    put('Packed.zip');
    put('__MACOSX/Zed Bank/oto.ini');
    put('.hidden/oto.ini');
    put('notes.txt');
    const r = scanLibrary(root);
    expect(r.ok).toBe(true);
    expect(r.entries.map((e) => `${e.kind}:${e.name}`)).toEqual(['folder:alpha', 'zip:Packed', 'folder:Zed Bank']);
    expect(r.entries[1].size).toBe(1);
  });

  it('follows a link to a voicebank kept elsewhere', () => {
    const elsewhere = mkdtempSync(join(tmpdir(), 'hillsing-elsewhere-'));
    writeFileSync(join(elsewhere, 'oto.ini'), '');
    const lib = mkdtempSync(join(tmpdir(), 'hillsing-linklib-'));
    symlinkSync(elsewhere, join(lib, 'Linked Bank'));
    const r = scanLibrary(lib);
    expect(r.entries.map((e) => e.name)).toEqual(['Linked Bank']);
    expect(listBankFiles(join(lib, 'Linked Bank'))).toEqual(['oto.ini']);
    rmSync(lib, { recursive: true, force: true });
    rmSync(elsewhere, { recursive: true, force: true });
  });

  it('treats the chosen folder itself as the bank when it holds an oto.ini', () => {
    const single = mkdtempSync(join(tmpdir(), 'hillsing-one-'));
    writeFileSync(join(single, 'oto.ini'), '');
    const r = scanLibrary(single);
    expect(r.entries).toHaveLength(1);
    expect(r.entries[0].kind).toBe('folder');
    rmSync(single, { recursive: true, force: true });
  });

  it('reports a missing folder (an unplugged drive) rather than throwing', () => {
    const r = scanLibrary(join(root, 'no such folder'));
    expect(r.ok).toBe(false);
    expect(r.error).toBe('not-found');
    expect(r.entries).toEqual([]);
  });

  it('lists every file of a bank with relative paths', () => {
    put('lister/oto.ini');
    put('lister/sub/a.wav');
    put('lister/.DS_Store');
    expect(listBankFiles(join(root, 'lister')).sort()).toEqual(['oto.ini', 'sub/a.wav']);
  });

  it('only allows paths inside the chosen folder', () => {
    expect(isInside('/Volumes/SSD/banks', '/Volumes/SSD/banks/Teto/oto.ini')).toBe(true);
    expect(isInside('/Volumes/SSD/banks', '/Volumes/SSD/banks')).toBe(true);
    expect(isInside('/Volumes/SSD/banks', '/Volumes/SSD/banks/../secrets/file')).toBe(false);
    expect(isInside('/Volumes/SSD/banks', '/Volumes/SSD/banks-other/x')).toBe(false);
    expect(isInside('/Volumes/SSD/banks', '/etc/passwd')).toBe(false);
  });
});
