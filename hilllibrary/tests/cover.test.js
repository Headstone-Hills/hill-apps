import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { LibraryDb } from '../server/db.js';
import { createApp } from '../server/app.js';
import { createLookup } from '../server/metadata.js';
import {
  tokens, parseTsv, summarizeWords, parseVisionLines, keywords, buildQueries, findIsbn, scoreCandidate,
  scanCover, createClaudeReader, createVisionReader, createTesseractReader,
} from '../server/cover.js';

const IMG = Buffer.from('/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////wgALCAABAAEBAREA/8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPxA=', 'base64');

// ---- fakes. Open Library search shape follows https://openlibrary.org/dev/docs/api/search ----
const DOCS = {
  hobbit: [{ key: '/works/OL1', title: 'The Hobbit', author_name: ['J.R.R. Tolkien'], first_publish_year: 1937, isbn: ['0547928221', '9780000000026', '9780547928227'], cover_i: 7, cover_edition_key: 'OL100M', edition_count: 500 },
           { key: '/works/OL2', title: 'The Hobbit Cookbook', author_name: ['Someone Else'], isbn: ['9780000000002'], edition_count: 2 }],
  dune: [{ key: '/works/OL3', title: 'Dune', author_name: ['Frank Herbert'], first_publish_year: 1965, isbn: ['9780441013593'], cover_i: 9, edition_count: 300 },
         { key: '/works/OL4', title: 'Dune Messiah', author_name: ['Frank Herbert'], isbn: ['9780593098233'], edition_count: 90 }],
  salinger: [{ key: '/works/OL5', title: 'Salinger', author_name: ['Pierre Salinger'], isbn: ['9780000000019'], edition_count: 3 },
             { key: '/works/OL6', title: 'The Catcher in the Rye', author_name: ['J. D. Salinger'], isbn: ['9780316769488'], edition_count: 400 }],
};
function fakeFetch({ olStatus = 200, gbStatus = 429 } = {}) {
  const calls = [];
  const f = async (url) => {
    const u = new URL(String(url));
    calls.push(String(url));
    if (u.hostname === 'openlibrary.org' && u.pathname === '/books/OL100M.json') return { ok: true, status: 200, json: async () => ({ isbn_13: ['9780547928227'] }) };
    if (u.hostname === 'openlibrary.org' && u.pathname.startsWith('/books/')) return { ok: false, status: 404, json: async () => ({}) };
    if (u.hostname === 'openlibrary.org') {
      if (olStatus !== 200) return { ok: false, status: olStatus, json: async () => ({}) };
      const q = `${u.searchParams.get('q') ?? ''} ${u.searchParams.get('title') ?? ''}`.toLowerCase();
      const docs = Object.entries(DOCS).filter(([k]) => q.includes(k)).flatMap(([, d]) => d);
      return { ok: true, status: 200, json: async () => ({ docs }) };
    }
    if (u.hostname === 'www.googleapis.com') return { ok: gbStatus < 400, status: gbStatus, json: async () => ({ items: [] }) };
    throw new Error('unexpected ' + url);
  };
  f.calls = calls;
  return f;
}
const fakeReader = (name, result) => ({ name, read: async () => { if (result instanceof Error) throw result; return result; } });
const read = (text, headline = text, extra = {}) => ({ text, headline, words: [], ...extra });

// ------------------------------------------------------------------ parsing

test('tokens: lowercase, accent-free words', () => {
  assert.deepEqual(tokens('Les Misérables — Victor Hugo!'), ['les', 'miserables', 'victor', 'hugo']);
});

test('parseTsv: keeps words with a confidence, drops layout rows', () => {
  const tsv = ['level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext',
    '1\t1\t0\t0\t0\t0\t0\t0\t800\t600\t-1\t',
    '5\t1\t1\t1\t1\t1\t100\t40\t300\t90\t93.5\tDUNE',
    '5\t1\t2\t1\t1\t1\t100\t400\t200\t30\t88\tFRANK'].join('\n');
  assert.deepEqual(parseTsv(tsv).map((w) => [w.text, w.height, w.conf]), [['DUNE', 90, 93.5], ['FRANK', 30, 88]]);
});

test('summarizeWords: the biggest type becomes the headline, noise is dropped', () => {
  const w = (text, top, height, conf = 90) => ({ text, conf, left: 10, top, height });
  const s = summarizeWords([w('THE', 40, 90), w('HOBBIT', 130, 100), w('J.R.R.', 600, 30), w('TOLKIEN', 640, 32), w('~', 5, 10), w('xx', 50, 90, 10)]);
  assert.equal(s.headline, 'THE HOBBIT');
  assert.match(s.text, /TOLKIEN/);
  assert.equal(summarizeWords([]).text, '');
});

test('parseVisionLines reads the helper output', () => {
  const w = parseVisionLines('0.1000\t0.2000\t0.0800\t1.000\tFRANK HERBERT\nnot a row\n0.5\t0.1\t0.2\t0.9\tDUNE');
  assert.deepEqual(w.map((x) => [x.text, x.top, x.height]), [['FRANK HERBERT', 0.1, 0.08], ['DUNE', 0.5, 0.2]]);
});

test('keywords/buildQueries: distinctive words, biggest first; one garbled word never forces a dead-end query', () => {
  const words = [{ text: 'HOBBIT', height: 100, conf: 90 }, { text: 'TOLKIEN', height: 40, conf: 90 }, { text: 'the', height: 90, conf: 99 }, { text: 'xq', height: 50, conf: 90 }, { text: '1937', height: 60, conf: 90 }];
  const kw = keywords(words);
  assert.deepEqual(kw, ['hobbit', 'tolkien']);
  const qs = buildQueries('HOBBIT', kw);
  assert.ok(qs.includes('hobbit') && qs.includes('tolkien') && qs.includes('hobbit tolkien'));
  assert.deepEqual(buildQueries('', []), []);
});

test('findIsbn: a printed ISBN counts only with a valid checksum', () => {
  assert.equal(findIsbn('ISBN 978-0-547-92822-7 Mariner'), '9780547928227');
  assert.equal(findIsbn('ISBN 0-547-92822-X'), '9780547928227');
  assert.equal(findIsbn('ISBN 0547928221'), null);              // wrong ISBN-10 check digit
  assert.equal(findIsbn('call no. 978-0-547-92822-9'), null);   // bad checksum
  assert.equal(findIsbn('copyright 1937 1938 1939'), null);
});

// ------------------------------------------------------------------ scoring

const cand = (title, author, popularity = 0) => ({ title, authors: author ? [author] : [], popularity });

test('scoreCandidate: title and author evidence in the cover text decide the rank', () => {
  const text = 'THE HOBBIT J.R.R. TOLKIEN';
  const good = scoreCandidate(cand('The Hobbit', 'J.R.R. Tolkien'), text, text);
  const wrong = scoreCandidate(cand('The Hobbit Cookbook', 'Someone Else'), text, text);
  const unrelated = scoreCandidate(cand('Moby Dick', 'Herman Melville'), text, text);
  assert.ok(good > wrong && wrong > unrelated, `${good} ${wrong} ${unrelated}`);
  assert.ok(unrelated < 0.3);
});

test('scoreCandidate: tolerates OCR slips, but a swapped letter in a 4-letter word is a different word', () => {
  assert.ok(scoreCandidate(cand('Gone Girl', 'Gillian Flynn'), 'GILLIAN FLYNN OGONE GIRL') > 1);   // dropped/added letter: ok
  assert.ok(scoreCandidate(cand('Gone Girl', 'Gillian Flynn'), 'GILLIAN FLYNN BONE GIRL') < 1);    // bone != gone
  assert.ok(scoreCandidate(cand('Pride and Prejudice', 'Jane Austen'), 'Ga bride Prejudice') > 0.5); // 5-letter word, one slip
});

test('scoreCandidate: big type outranks a blurb, a book named after the author is demoted, popularity breaks ties', () => {
  const text = 'GILLIAN FLYNN GONE GIRL also by the author of DARK PLACES';
  const head = 'GILLIAN FLYNN GONE GIRL';
  assert.ok(scoreCandidate(cand('Gone Girl', 'Gillian Flynn'), text, head) > scoreCandidate(cand('Dark Places', 'Gillian Flynn'), text, head));
  assert.ok(scoreCandidate(cand('The Catcher in the Rye', 'J. D. Salinger'), 'a novel by J.D. SALINGER THE CATCHER IN THE RYE', 'THE CATCHER IN THE RYE') >
            scoreCandidate(cand('Salinger', 'Pierre Salinger'), 'a novel by J.D. SALINGER', 'SALINGER'));
  assert.ok(scoreCandidate(cand('Dune', 'Frank Herbert', 300), 'DUNE', 'DUNE') > scoreCandidate(cand('Dune', 'Frank Herbert', 2), 'DUNE', 'DUNE'));
});

// ------------------------------------------------------------------ scanCover

test('scanCover: reads, searches, ranks, picks one ISBN-13 per candidate, and dedupes editions', async () => {
  const f = fakeFetch();
  const r = await scanCover(IMG, 'image/jpeg', { readers: [fakeReader('fake', read('THE HOBBIT J.R.R. TOLKIEN', 'THE HOBBIT', { words: [{ text: 'HOBBIT', height: 90, conf: 95 }, { text: 'TOLKIEN', height: 40, conf: 95 }] }))], fetchImpl: f });
  assert.equal(r.reader, 'fake');
  assert.equal(r.candidates[0].title, 'The Hobbit');
  assert.equal(r.candidates[0].isbn13, '9780547928227');       // from the cover edition, not the first ISBN in the list
  assert.equal(r.candidates[0].cover_url, 'https://covers.openlibrary.org/b/id/7-M.jpg');
  assert.equal(new Set(r.candidates.map((c) => c.title)).size, r.candidates.length);
  assert.ok(r.notes.some((n) => /Google Books/.test(n)));            // a failing second source is reported, not fatal
  assert.ok(f.calls.length <= 12);                                    // bounded fan-out
});

test('scanCover: a printed ISBN is returned; unreadable photos explain themselves; no match is reported', async () => {
  const withIsbn = await scanCover(IMG, 'image/jpeg', { readers: [fakeReader('f', read('ISBN 978-0-441-01359-3 DUNE', 'DUNE'))], fetchImpl: fakeFetch() });
  assert.equal(withIsbn.isbn, '9780441013593');
  const blank = await scanCover(IMG, 'image/jpeg', { readers: [fakeReader('f', read(''))], fetchImpl: fakeFetch() });
  assert.deepEqual(blank.candidates, []);
  assert.match(blank.notes[0], /No readable text/);
  const none = await scanCover(IMG, 'image/jpeg', { readers: [fakeReader('f', read('ZZZQQ WWWXX', 'ZZZQQ', { words: [{ text: 'ZZZQQ', height: 50, conf: 90 }] }))], fetchImpl: fakeFetch() });
  assert.deepEqual(none.candidates, []);
  assert.match(none.notes.join(' '), /no matching book/);
});

test('scanCover chain: stops at a confident reader, falls back when it finds nothing, and tolerates a missing reader', async () => {
  const good = read('DUNE FRANK HERBERT', 'DUNE FRANK HERBERT', { words: [{ text: 'DUNE', height: 90, conf: 95 }, { text: 'HERBERT', height: 50, conf: 95 }] });
  let secondCalled = false;
  const second = { name: 'second', read: async () => { secondCalled = true; return good; } };
  const a = await scanCover(IMG, 'image/jpeg', { readers: [fakeReader('first', good), second], fetchImpl: fakeFetch() });
  assert.equal(a.reader, 'first');
  assert.equal(secondCalled, false);                                  // confident: don't spend more effort

  const b = await scanCover(IMG, 'image/jpeg', { readers: [fakeReader('weak', read('')), fakeReader('strong', good)], fetchImpl: fakeFetch() });
  assert.equal(b.reader, 'strong');
  assert.equal(b.candidates[0].title, 'Dune');

  const c = await scanCover(IMG, 'image/jpeg', { readers: [fakeReader('broken', new Error('swiftc not found')), fakeReader('ok', good)], fetchImpl: fakeFetch() });
  assert.equal(c.reader, 'ok');
  assert.match(c.notes.join(' '), /broken reader unavailable/);

  await assert.rejects(() => scanCover(IMG, 'image/jpeg', { readers: [fakeReader('x', new Error('nope'))], fetchImpl: fakeFetch() }), /No cover reader is available/);
});

test('scanCover chain: a later reader never replaces a reasonable first result with its own fluke', async () => {
  const first = read('J.D. SALINGER THE CATCHER IN THE RYE', 'THE CATCHER IN THE RYE', { words: [{ text: 'CATCHER', height: 80, conf: 95 }, { text: 'SALINGER', height: 40, conf: 95 }] });
  const fluke = read('SALINGER', 'SALINGER', { words: [{ text: 'SALINGER', height: 80, conf: 95 }] });
  const r = await scanCover(IMG, 'image/jpeg', { readers: [fakeReader('vision', first), fakeReader('tess', fluke)], fetchImpl: fakeFetch() });
  assert.equal(r.reader, 'vision');
  assert.equal(r.candidates[0].title, 'The Catcher in the Rye');
});

test('scanCover: Open Library down is reported as such, not as "no book"', async () => {
  const r = await scanCover(IMG, 'image/jpeg', { readers: [fakeReader('f', read('DUNE', 'DUNE', { words: [{ text: 'DUNE', height: 90, conf: 90 }] }))], fetchImpl: fakeFetch({ olStatus: 500 }) });
  assert.deepEqual(r.candidates, []);
  assert.match(r.notes.join(' '), /Open Library/);
});

// ------------------------------------------------------------------ readers

test('Claude reader: sends the photo as a base64 image block and parses the JSON reply', async () => {
  let sent;
  const f = async (url, opts) => {
    sent = { url, headers: opts.headers, body: JSON.parse(opts.body) };
    return { ok: true, status: 200, json: async () => ({ content: [{ type: 'text', text: 'Here you go:\n```json\n{"title":"Dune","subtitle":"","authors":["Frank Herbert"],"series":"","publisher":"Ace","isbn":"","language":"en"}\n```' }] }) };
  };
  const r = await createClaudeReader({ apiKey: 'sk-test', fetchImpl: f }).read(IMG, 'image/jpeg');
  assert.equal(sent.url, 'https://api.anthropic.com/v1/messages');
  assert.equal(sent.headers['x-api-key'], 'sk-test');
  assert.equal(sent.headers['anthropic-version'], '2023-06-01');
  assert.equal(sent.body.messages[0].content[0].type, 'image');
  assert.equal(sent.body.messages[0].content[0].source.data, IMG.toString('base64'));
  assert.equal(sent.body.messages[0].content[0].source.media_type, 'image/jpeg');
  assert.deepEqual([r.title, r.authors, r.publisher], ['Dune', ['Frank Herbert'], 'Ace']);
  assert.match(r.headline, /Dune Frank Herbert/);
  const bad = createClaudeReader({ apiKey: 'k', fetchImpl: async () => ({ ok: true, status: 200, json: async () => ({ content: [{ type: 'text', text: 'I cannot read it' }] }) }) });
  assert.equal((await bad.read(IMG, 'image/jpeg')).title, '');               // garbage reply = nothing read, not a crash
  await assert.rejects(() => createClaudeReader({ apiKey: 'k', fetchImpl: async () => ({ ok: false, status: 401 }) }).read(IMG, 'image/jpeg'), /HTTP 401/);
});

test('Vision reader: refuses off macOS, builds the helper once, then runs it', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'hl-vis-'));
  await assert.rejects(() => createVisionReader({ binDir: dir, platform: 'linux' }).read(IMG), /only available on macOS/);

  const calls = [];
  const exec = async (cmd, args) => {
    calls.push(cmd.split('/').pop());
    if (cmd === 'swiftc') { writeFileSync(args[args.indexOf('-o') + 1], '#!/bin/sh'); return { stdout: '' }; }
    return { stdout: '0.1000\t0.1000\t0.2000\t1.000\tDUNE\n0.6\t0.1\t0.05\t1.000\tFRANK HERBERT\n' };
  };
  const reader = createVisionReader({ binDir: dir, platform: 'darwin', exec, source: new URL('../server/native/cover-ocr.swift', import.meta.url).pathname });
  const a = await reader.read(IMG);
  await reader.read(IMG);
  assert.equal(calls.filter((c) => c === 'swiftc').length, 1);               // compiled once, not per scan
  assert.equal(a.headline, 'DUNE');
  const failing = createVisionReader({ binDir: mkdtempSync(join(tmpdir(), 'hl-vis-')), platform: 'darwin', exec: async () => { throw Object.assign(new Error('x'), { code: 'ENOENT' }); }, source: new URL('../server/native/cover-ocr.swift', import.meta.url).pathname });
  await assert.rejects(() => failing.read(IMG), /Xcode command line tools/);
});

test('Tesseract reader: passes the image to the binary without a shell, tolerates a failing second pass', async () => {
  const tsv = 'level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t5\t5\t90\t80\t91\tDUNE';
  const seen = [];
  const exec = async (bin, args) => { seen.push([bin, args]); if (args.includes('thresholding_method=2')) throw new Error('unsupported'); return { stdout: tsv }; };
  const r = await createTesseractReader({ exec }).read(IMG);
  assert.equal(r.headline, 'DUNE');
  assert.equal(seen.length, 2);
  assert.ok(seen.every(([bin, args]) => bin === 'tesseract' && args.includes('--psm') && args.at(-1) === 'tsv'));
  await assert.rejects(() => createTesseractReader({ exec: async () => { throw new Error('ENOENT'); } }).read(IMG), /ENOENT/);
});

// ------------------------------------------------------------------ API

async function boot({ readers = [fakeReader('fake', read('DUNE FRANK HERBERT', 'DUNE', { words: [{ text: 'DUNE', height: 90, conf: 95 }, { text: 'HERBERT', height: 40, conf: 95 }] }))], fetchImpl = fakeFetch() } = {}) {
  const dataDir = mkdtempSync(join(tmpdir(), 'hl-cov-'));
  const publicDir = join(dataDir, 'public');
  mkdirSync(publicDir); writeFileSync(join(publicDir, 'index.html'), 'x');
  const db = new LibraryDb(':memory:');
  const server = createServer(createApp({ db, dataDir, publicDir, fetchImpl, lookup: createLookup({ fetchImpl }), coverReaders: readers }));
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  return {
    db,
    async post(body, type = 'image/jpeg') {
      const res = await fetch(`${base}/api/cover/scan`, { method: 'POST', headers: type ? { 'Content-Type': type } : {}, body });
      return { status: res.status, json: await res.json().catch(() => null) };
    },
    async json(path) { return (await fetch(base + path)).json(); },
    close: () => new Promise((r) => { server.close(r); db.close(); }),
  };
}

test('API /api/cover/scan: returns ranked candidates and flags books you already own', async () => {
  const t = await boot();
  try {
    t.db.insert({ title: 'Dune', isbn13: '9780441013593', authors: ['Frank Herbert'] });
    const r = await t.post(IMG);
    assert.equal(r.status, 200);
    assert.equal(r.json.candidates[0].title, 'Dune');
    assert.equal(r.json.candidates[0].isbn13, '9780441013593');
    assert.equal(typeof r.json.candidates[0].existing, 'number');            // "already in your library"
    assert.equal(r.json.reader, 'fake');
    assert.equal((await t.json('/api/taxonomy')).coverScan, true);
  } finally { await t.close(); }
});

test('API /api/cover/scan: rejects non-images, oversize and empty bodies; reports "no reader" and "feature off"', async () => {
  const t = await boot();
  try {
    assert.equal((await t.post('hello', 'text/plain')).status, 415);
    assert.equal((await t.post(JSON.stringify({ a: 1 }), 'application/json')).status, 415);
    assert.equal((await t.post(Buffer.alloc(0))).status, 400);
    assert.equal((await t.post(Buffer.alloc(9 * 1024 * 1024, 1))).status, 413);
    assert.equal((await t.post(IMG, null)).status, 415);
  } finally { await t.close(); }
  const broken = await boot({ readers: [fakeReader('x', new Error('no swift'))] });
  try {
    const r = await broken.post(IMG);
    assert.equal(r.status, 503);
    assert.match(r.json.error, /No cover reader is available/);
  } finally { await broken.close(); }
  const off = await boot({ readers: [] });
  try {
    assert.equal((await off.post(IMG)).status, 501);
    assert.equal((await off.json('/api/taxonomy')).coverScan, false);
  } finally { await off.close(); }
});

test('edition choice: the cover edition decides the ISBN; a failing edition lookup keeps the earlier pick', async () => {
  const hobbit = read('THE HOBBIT TOLKIEN', 'THE HOBBIT', { words: [{ text: 'HOBBIT', height: 90, conf: 95 }, { text: 'TOLKIEN', height: 40, conf: 95 }] });
  const ok = await scanCover(IMG, 'image/jpeg', { readers: [fakeReader('f', hobbit)], fetchImpl: fakeFetch() });
  assert.equal(ok.candidates[0].isbn13, '9780547928227');
  assert.equal('coverEdition' in ok.candidates[0], false);                          // internal detail is not leaked
  const base = fakeFetch();
  const flaky = async (url) => (String(url).includes('/books/OL100M') ? { ok: false, status: 500, json: async () => ({}) } : base(url));
  const fallback = await scanCover(IMG, 'image/jpeg', { readers: [fakeReader('f', hobbit)], fetchImpl: flaky });
  assert.equal(fallback.candidates[0].isbn13, '9780000000026');                      // first valid ISBN-13 in the work's list
});
