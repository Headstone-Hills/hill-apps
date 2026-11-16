import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtempSync, rmSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { LibraryDb } from '../server/db.js';
import { createApp } from '../server/app.js';
import { createLookup, mergeProviders, fetchOpenLibrary, fetchGoogleBooks, downloadCover, LookupError } from '../server/metadata.js';

// ---- fake providers (shapes follow the documented Open Library / Google Books responses) ----
const DUNE = '9780441013593';
const OL = { [`ISBN:${DUNE}`]: {
  title: 'Dune', authors: [{ name: 'Frank Herbert' }], publishers: [{ name: 'Ace' }], publish_date: 'August 2005', number_of_pages: 535,
  subjects: [{ name: 'Science fiction' }, { name: 'Fiction' }, { name: 'Large type books' }, { name: 'Ecology' }],
  classifications: { dewey_decimal_class: ['813/.54'] }, cover: { large: 'https://covers.openlibrary.org/b/id/1-L.jpg' } } };
const GB = { items: [{ volumeInfo: {
  title: 'Dune', authors: ['Frank Herbert'], description: '<p>Set on the desert planet <b>Arrakis</b>.</p>',
  categories: ['Fiction / Science Fiction / General'], pageCount: 612, imageLinks: { thumbnail: 'http://books.google.com/t.jpg&edge=curl' } } }] };
const JPEG = Buffer.alloc(2048, 7);

function fakeFetch({ ol = OL, gb = GB, olStatus = 200, gbStatus = 200 } = {}) {
  const calls = [];
  const f = async (url) => {
    const u = String(url);
    calls.push(u);
    const json = (status, body) => ({ ok: status < 400, status, json: async () => body });
    if (u.includes('openlibrary.org/api/books')) return json(olStatus, ol);
    if (u.includes('googleapis.com/books')) return json(gbStatus, gb);
    if (/openlibrary\.org|books\.google\.com/.test(u)) {
      return { ok: true, status: 200, headers: new Map([['content-type', 'image/jpeg']]), arrayBuffer: async () => JPEG };
    }
    throw new Error('unexpected fetch ' + u);
  };
  f.calls = calls;
  return f;
}

async function boot({ password = '', fetchImpl = fakeFetch(), lookup } = {}) {
  const dataDir = mkdtempSync(join(tmpdir(), 'hl-test-'));
  const db = new LibraryDb(':memory:');
  const publicDir = join(dataDir, 'public');
  mkdirSync(publicDir);
  writeFileSync(join(publicDir, 'index.html'), '<h1>hi</h1>');
  const handler = createApp({ db, lookup: lookup ?? createLookup({ fetchImpl }), dataDir, publicDir, password, fetchImpl });
  const server = createServer(handler);
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  const ctx = {
    db, base, dataDir, fetchImpl,
    async req(method, path, body, headers = {}) {
      const res = await fetch(base + path, {
        method, redirect: 'manual',
        headers: { ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}), ...headers },
        body: body !== undefined ? JSON.stringify(body) : undefined,
      });
      const text = await res.text();
      let json; try { json = JSON.parse(text); } catch { json = null; }
      return { status: res.status, json, text, headers: res.headers };
    },
    async close() { server.close(); server.closeAllConnections(); db.close(); rmSync(dataDir, { recursive: true, force: true }); },
  };
  return ctx;
}

// ------------------------------------------------------------------ metadata
test('mergeProviders: OL for bibliographic data, Google for description, both feed the classifier', async () => {
  const lookup = createLookup({ fetchImpl: fakeFetch() });
  const m = await lookup(DUNE);
  assert.equal(m.title, 'Dune');
  assert.equal(m.publisher, 'Ace');
  assert.equal(m.pages, 535);                         // OL preferred
  assert.equal(m.description, 'Set on the desert planet Arrakis.'); // HTML stripped, from Google
  assert.equal(m.cover_url, 'https://covers.openlibrary.org/b/id/1-L.jpg');
  assert.equal(m.isbn10, '0441013597');
  assert.equal(m.genre, 'Science Fiction');
  assert.deepEqual(m.sources, ['openlibrary', 'googlebooks']);
  assert.ok(!m.tags.includes('large type books'));
});

test('lookup survives one provider failing, and uses the other', async () => {
  const m = await createLookup({ fetchImpl: fakeFetch({ olStatus: 500 }) })(DUNE);
  assert.equal(m.title, 'Dune');
  assert.deepEqual(m.sources, ['googlebooks']);
  assert.equal(m.provider_errors[0].provider, 'openlibrary');
  assert.equal(m.pages, 612);
});

test('lookup: both down -> LookupError; both answer-but-unknown -> null', async () => {
  await assert.rejects(createLookup({ fetchImpl: fakeFetch({ olStatus: 503, gbStatus: 429 }) })(DUNE), LookupError);
  assert.equal(await createLookup({ fetchImpl: fakeFetch({ ol: {}, gb: { totalItems: 0 } }) })(DUNE), null);
});

test('one provider unknown + the other erroring is "not found", not an outage', async () => {
  assert.equal(await createLookup({ fetchImpl: fakeFetch({ ol: {}, gbStatus: 429 }) })(DUNE), null);
});

test('providers tolerate malformed payloads', async () => {
  const weird = fakeFetch({ ol: { [`ISBN:${DUNE}`]: { title: 42, authors: 'x', subjects: [null, 5, { name: 'ok' }], number_of_pages: '300' } }, gb: { items: [{}] } });
  const ol = await fetchOpenLibrary(DUNE, weird);
  assert.equal(ol.title, ''); assert.deepEqual(ol.authors, []); assert.deepEqual(ol.subjects, ['ok']); assert.equal(ol.pages, null);
  assert.equal(await fetchGoogleBooks(DUNE, weird), null);
  assert.equal(mergeProviders(DUNE, ol, null), null); // no usable title -> treated as not found
});

test('lookup results are cached', async () => {
  const f = fakeFetch();
  const lookup = createLookup({ fetchImpl: f });
  await lookup(DUNE); await lookup(DUNE);
  assert.equal(f.calls.filter((u) => u.includes('googleapis')).length, 1);
});

test('downloadCover: host allowlist, https only, content-type, size', async () => {
  const f = fakeFetch();
  assert.ok(await downloadCover('https://covers.openlibrary.org/b/id/1-L.jpg', f));
  assert.match(f.calls.at(-1), /default=false/);
  assert.equal(await downloadCover('http://covers.openlibrary.org/x.jpg', f), null);
  assert.equal(await downloadCover('https://evil.example.com/x.jpg', f), null);
  assert.equal(await downloadCover('https://openlibrary.org.evil.com/x.jpg', f), null);
  assert.equal(await downloadCover('file:///etc/passwd', f), null);
  assert.equal(await downloadCover('not a url', f), null);
});

// ----------------------------------------------------------------------- API
test('lookup endpoint: validates ISBN, returns preview and existing copy', async () => {
  const t = await boot();
  try {
    assert.equal((await t.req('GET', '/api/lookup/9780441013594')).status, 400);          // bad checksum
    const r = await t.req('GET', '/api/lookup/0441013597');                                 // ISBN-10 accepted
    assert.equal(r.status, 200);
    assert.equal(r.json.found, true);
    assert.equal(r.json.isbn13, DUNE);
    assert.equal(r.json.existing, null);
  } finally { await t.close(); }
});

test('add by ISBN: stores metadata, genre, tags, cover; then duplicate -> 409', async () => {
  const t = await boot();
  try {
    const add = await t.req('POST', '/api/books', { isbn: '978-0-441-01359-3', location: 'Study' });
    assert.equal(add.status, 201, add.text);
    const b = add.json;
    assert.equal(b.title, 'Dune');
    assert.equal(b.genre, 'Science Fiction');
    assert.equal(b.genre_locked, false);
    assert.equal(b.location, 'Study');
    assert.equal(b.has_cover, true);
    assert.ok(existsSync(join(t.dataDir, 'covers', b.cover_file)));
    assert.ok(b.tags.includes('ecology'));

    const cover = await fetch(`${t.base}/covers/${b.cover_file}`);
    assert.equal(cover.status, 200);
    assert.equal(cover.headers.get('content-type'), 'image/jpeg');

    const dup = await t.req('POST', '/api/books', { isbn: DUNE });
    assert.equal(dup.status, 409);
    assert.equal(dup.json.existing.id, b.id);
    assert.equal((await t.req('GET', '/api/lookup/' + DUNE)).json.existing.id, b.id);
  } finally { await t.close(); }
});

test('add: unknown ISBN needs a title; provider outage is reported distinctly', async () => {
  const t = await boot({ fetchImpl: fakeFetch({ ol: {}, gb: { totalItems: 0 } }) });
  try {
    const nf = await t.req('POST', '/api/books', { isbn: DUNE });
    assert.equal(nf.status, 404);
    assert.equal(nf.json.notFound, true);
    const manual = await t.req('POST', '/api/books', { isbn: DUNE, title: 'My Copy of Dune', authors: ['Frank Herbert'] });
    assert.equal(manual.status, 201);
    assert.equal(manual.json.isbn13, DUNE);
  } finally { await t.close(); }
  const down = await boot({ fetchImpl: fakeFetch({ olStatus: 500, gbStatus: 500 }) });
  try {
    const r = await down.req('POST', '/api/books', { isbn: DUNE });
    assert.equal(r.status, 502);
    assert.equal((await down.req('GET', '/api/lookup/' + DUNE)).status, 502);
  } finally { await down.close(); }
});

test('manual add without ISBN; validation errors', async () => {
  const t = await boot();
  try {
    const r = await t.req('POST', '/api/books', { title: 'Family Recipes', authors: ['Grandma'], genre: 'Cooking & Food', tags: ['Family', ' heirloom '] });
    assert.equal(r.status, 201);
    assert.equal(r.json.genre, 'Cooking & Food');
    assert.equal(r.json.genre_locked, true);
    assert.deepEqual(r.json.tags, ['family', 'heirloom']);
    assert.equal(r.json.isbn13, null);
    assert.equal((await t.req('POST', '/api/books', {})).status, 400);
    assert.equal((await t.req('POST', '/api/books', { title: 'x', status: 'lost' })).status, 400);
    assert.equal((await t.req('POST', '/api/books', { title: 'x', pages: -3 })).status, 400);
    assert.equal((await t.req('POST', '/api/books', { title: 'x', authors: 'bob' })).status, 400);
    assert.equal((await t.req('POST', '/api/books', { isbn: '1234567890123' })).status, 400);
  } finally { await t.close(); }
});

test('search, filters and facets over HTTP', async () => {
  const t = await boot();
  try {
    await t.req('POST', '/api/books', { isbn: DUNE });
    await t.req('POST', '/api/books', { title: 'Sourdough Basics', genre: 'Cooking & Food', authors: ['Pat Baker'] });
    assert.equal((await t.req('GET', '/api/books?q=arrakis')).json.books[0].title, 'Dune');
    assert.equal((await t.req('GET', '/api/books?q=sourd')).json.total, 1);
    assert.equal((await t.req('GET', '/api/books?genre=Cooking%20%26%20Food')).json.total, 1);
    assert.equal((await t.req('GET', '/api/books?q=%22%28%28')).status, 200);
    const f = (await t.req('GET', '/api/facets')).json;
    assert.equal(f.total, 2);
    assert.deepEqual(f.genres.map((g) => g.genre).sort(), ['Cooking & Food', 'Science Fiction']);
    const tax = (await t.req('GET', '/api/taxonomy')).json;
    assert.ok(tax.genres.some((g) => g.name === 'Fantasy'));
  } finally { await t.close(); }
});

test('PATCH: editing genre locks it; "auto" unlocks and reclassifies; tags lock/unlock', async () => {
  const t = await boot();
  try {
    const { json: b } = await t.req('POST', '/api/books', { isbn: DUNE });
    let r = await t.req('PATCH', `/api/books/${b.id}`, { genre: 'Fantasy' });
    assert.equal(r.json.genre, 'Fantasy'); assert.equal(r.json.genre_locked, true); assert.equal(r.json.subgenre, '');

    await t.req('POST', '/api/reclassify');                          // must not touch a locked genre
    assert.equal(t.db.get(b.id).genre, 'Fantasy');

    r = await t.req('PATCH', `/api/books/${b.id}`, { genre: 'auto' });
    assert.equal(r.json.genre, 'Science Fiction'); assert.equal(r.json.genre_locked, false);

    r = await t.req('PATCH', `/api/books/${b.id}`, { tags: ['favourite'] });
    assert.deepEqual(r.json.tags, ['favourite']); assert.equal(r.json.tags_locked, true);
    await t.req('POST', '/api/reclassify');
    assert.deepEqual(t.db.get(b.id).tags, ['favourite']);            // locked tags survive reclassify

    r = await t.req('PATCH', `/api/books/${b.id}`, { autoTags: true });
    assert.equal(r.json.tags_locked, false); assert.ok(r.json.tags.includes('ecology'));

    r = await t.req('PATCH', `/api/books/${b.id}`, { status: 'read', notes: 'lent to Sam', title: 'Dune (1965)' });
    assert.equal(r.json.status, 'read');
    assert.equal((await t.req('GET', '/api/books?q=lent')).json.total, 1);
    assert.equal((await t.req('PATCH', '/api/books/999', { title: 'x' })).status, 404);
    assert.equal((await t.req('PATCH', `/api/books/${b.id}`, { title: '' })).status, 400);
  } finally { await t.close(); }
});

test('reclassify picks up taxonomy changes offline using stored raw metadata', async () => {
  const t = await boot({ fetchImpl: fakeFetch() });
  try {
    const { json: b } = await t.req('POST', '/api/books', { isbn: DUNE });
    t.db.update(b.id, { genre: 'Uncategorized', subgenre: '', needs_review: true });
    t.fetchImpl.calls.length = 0;
    const r = await t.req('POST', '/api/reclassify');
    assert.equal(r.json.changed, 1);
    assert.equal(t.db.get(b.id).genre, 'Science Fiction');
    assert.equal(t.fetchImpl.calls.length, 0);                       // no network used
  } finally { await t.close(); }
});

test('refresh fills blanks but never overwrites user edits', async () => {
  const t = await boot();
  try {
    const { json: b } = await t.req('POST', '/api/books', { isbn: DUNE, title: 'My Title', publisher: 'My Press' });
    assert.equal(b.title, 'My Title');
    t.db.update(b.id, { description: '' });
    const r = await t.req('POST', `/api/books/${b.id}/refresh`);
    assert.equal(r.status, 200);
    assert.equal(r.json.title, 'My Title'); assert.equal(r.json.publisher, 'My Press');
    assert.match(r.json.description, /Arrakis/);
    const noIsbn = await t.req('POST', '/api/books', { title: 'No ISBN' });
    assert.equal((await t.req('POST', `/api/books/${noIsbn.json.id}/refresh`)).status, 400);
  } finally { await t.close(); }
});

test('delete removes book, search entry and cover file', async () => {
  const t = await boot();
  try {
    const { json: b } = await t.req('POST', '/api/books', { isbn: DUNE });
    const file = join(t.dataDir, 'covers', b.cover_file);
    assert.ok(existsSync(file));
    assert.equal((await t.req('DELETE', `/api/books/${b.id}`)).status, 200);
    assert.equal(existsSync(file), false);
    assert.equal((await t.req('GET', '/api/books?q=dune')).json.total, 0);
    assert.equal((await t.req('DELETE', `/api/books/${b.id}`)).status, 404);
  } finally { await t.close(); }
});

test('export: JSON and CSV (formula injection neutralised)', async () => {
  const t = await boot();
  try {
    await t.req('POST', '/api/books', { title: '=HYPERLINK("http://x")', authors: ['A, B'] });
    const j = await t.req('GET', '/api/export.json');
    assert.equal(JSON.parse(j.text).length, 1);
    assert.match(j.headers.get('content-disposition'), /attachment/);
    const c = await t.req('GET', '/api/export.csv');
    assert.ok(c.text.includes(`"'=HYPERLINK(""http://x"")"`), c.text);
    assert.ok(c.text.includes('"A, B"'));
  } finally { await t.close(); }
});

// ------------------------------------------------------------------ security
test('static files are served, traversal is blocked, security headers set', async () => {
  const t = await boot();
  try {
    const home = await t.req('GET', '/');
    assert.equal(home.status, 200);
    assert.match(home.headers.get('content-security-policy'), /script-src 'self'/);
    assert.equal(home.headers.get('x-content-type-options'), 'nosniff');
    for (const p of ['/../package.json', '/%2e%2e/package.json', '/..%2fpackage.json', '/vendor/../../x']) {
      assert.notEqual((await t.req('GET', p)).status, 200, p);
    }
    assert.equal((await t.req('GET', '/nope.txt')).status, 404);
    assert.equal((await t.req('GET', '/api/nope')).status, 404);
  } finally { await t.close(); }
});

test('mutating requests require JSON and reject cross-origin', async () => {
  const t = await boot();
  try {
    const form = await fetch(`${t.base}/api/books`, { method: 'POST', headers: { 'Content-Type': 'text/plain' }, body: '{"title":"x"}' });
    assert.equal(form.status, 415);
    const evil = await t.req('POST', '/api/books', { title: 'x' }, { Origin: 'https://evil.example' });
    assert.equal(evil.status, 403);
    const same = await t.req('POST', '/api/books', { title: 'x' }, { Origin: t.base });
    assert.equal(same.status, 201);
    const big = await fetch(`${t.base}/api/books`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ title: 'x'.repeat(2e6) }) });
    assert.equal(big.status, 413);
  } finally { await t.close(); }
});

test('password protection: API locked, login sets HttpOnly cookie, wrong password refused, logout clears', async () => {
  const t = await boot({ password: 'correct horse battery' });
  try {
    assert.equal((await t.req('GET', '/api/books')).status, 401);
    assert.equal((await t.req('GET', '/api/facets')).status, 401);
    assert.equal((await t.req('GET', '/covers/1.jpg')).status, 401);
    assert.equal((await t.req('POST', '/api/books', { title: 'x' })).status, 401);
    assert.equal((await t.req('GET', '/')).status, 200);                       // app shell is public
    let s = (await t.req('GET', '/api/session')).json;
    assert.deepEqual(s, { authRequired: true, authed: false });

    assert.equal((await t.req('POST', '/api/login', { password: 'nope' })).status, 401);
    const ok = await t.req('POST', '/api/login', { password: 'correct horse battery' });
    assert.equal(ok.status, 200);
    const cookie = ok.headers.get('set-cookie');
    assert.match(cookie, /HttpOnly/); assert.match(cookie, /SameSite=Lax/);
    const hdr = { Cookie: cookie.split(';')[0] };
    assert.equal((await t.req('GET', '/api/books', undefined, hdr)).status, 200);
    assert.equal((await t.req('GET', '/api/session', undefined, hdr)).json.authed, true);

    const forged = { Cookie: `hl_session=${Date.now() + 1e9}.${'0'.repeat(64)}` };
    assert.equal((await t.req('GET', '/api/books', undefined, forged)).status, 401);
    const expired = { Cookie: `hl_session=${Date.now() - 1000}.${cookie.split('.').pop().split(';')[0]}` };
    assert.equal((await t.req('GET', '/api/books', undefined, expired)).status, 401);

    const out = await t.req('POST', '/api/logout', {});
    assert.match(out.headers.get('set-cookie'), /Max-Age=0/);
  } finally { await t.close(); }
});

test('login is rate limited after repeated failures', async () => {
  const t = await boot({ password: 'correct horse battery' });
  try {
    for (let i = 0; i < 10; i++) assert.equal((await t.req('POST', '/api/login', { password: 'bad' + i })).status, 401);
    assert.equal((await t.req('POST', '/api/login', { password: 'bad' })).status, 429);
    assert.equal((await t.req('POST', '/api/login', { password: 'correct horse battery' })).status, 429); // locked out even with the right one
  } finally { await t.close(); }
});

test('session cookie is Secure behind an https proxy', async () => {
  const t = await boot({ password: 'correct horse battery' });
  try {
    const r = await t.req('POST', '/api/login', { password: 'correct horse battery' }, { 'X-Forwarded-Proto': 'https' });
    assert.match(r.headers.get('set-cookie'), /; Secure/);
  } finally { await t.close(); }
});
