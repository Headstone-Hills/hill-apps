import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtempSync, rmSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { LibraryDb } from '../server/db.js';
import { createApp } from '../server/app.js';
import { toIsbn10 } from '../server/isbn.js';
import { createLookup, createSearch, createBreaker, mergeSearchResults, searchOpenLibrary, searchGoogleBooks, mergeProviders, fetchOpenLibrary, fetchGoogleBooks, downloadCover, LookupError } from '../server/metadata.js';

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

async function boot({ password = '', fetchImpl = fakeFetch(), lookup, search } = {}) {
  const dataDir = mkdtempSync(join(tmpdir(), 'hl-test-'));
  const db = new LibraryDb(':memory:');
  const publicDir = join(dataDir, 'public');
  mkdirSync(publicDir);
  writeFileSync(join(publicDir, 'index.html'), '<h1>hi</h1>');
  const handler = createApp({ db, lookup: lookup ?? createLookup({ fetchImpl }), search, dataDir, publicDir, password, fetchImpl });
  const server = createServer(handler);
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  const ctx = {
    db, base, dataDir, fetchImpl, idle: () => handler.idle(),
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
  const partial = mergeProviders(DUNE, ol, null);       // no title, but subjects survived -> partial, not "found"
  assert.equal(partial.incomplete, true);
  assert.deepEqual(partial.subjects, ['ok']);
  assert.equal(mergeProviders(DUNE, { title: '', authors: [], subjects: [], dewey: [], pages: null }, null), null); // truly nothing -> not found
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
    assert.equal(b.has_cover, false);                       // the response does not wait for the cover download
    await t.idle();
    const stored = t.db.get(b.id);
    assert.equal(stored.has_cover, true);
    assert.ok(existsSync(join(t.dataDir, 'covers', stored.cover_file)));
    assert.ok(b.tags.includes('ecology'));

    const cover = await fetch(`${t.base}/covers/${stored.cover_file}`);
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
    await t.idle();
    const file = join(t.dataDir, 'covers', t.db.get(b.id).cover_file);
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


// ------------------------------------------------ partial lookups + cover photos
const PARTIAL_ISBN = '9780743273565';
const partialFetch = () => async (url) => {
  const u = String(url);
  const json = (body) => ({ ok: true, status: 200, json: async () => body });
  if (u.includes('openlibrary.org/api/books')) {
    return json({ [`ISBN:${PARTIAL_ISBN}`]: { publishers: [{ name: 'Scribner' }], number_of_pages: 180, subjects: [{ name: 'Rich people' }, { name: 'Long Island (N.Y.)' }, { name: 'Fiction' }] } });
  }
  if (u.includes('googleapis.com/books')) return json({ totalItems: 0 });
  return { ok: false, status: 404 };
};
const JPG = Buffer.concat([Buffer.from([0xff, 0xd8, 0xff, 0xe0]), Buffer.alloc(3000, 1)]);
const PNG = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.alloc(3000, 2)]);

async function putCover(t, id, body, headers = {}) {
  const res = await fetch(`${t.base}/api/books/${id}/cover`, { method: 'PUT', headers: { 'Content-Type': 'image/jpeg', ...headers }, body });
  const text = await res.text();
  let json; try { json = JSON.parse(text); } catch { json = null; }
  return { status: res.status, json, headers: res.headers };
}

test('partial lookup: no title is "not found" but the other fields are returned for pre-filling', async () => {
  const t = await boot({ fetchImpl: partialFetch() });
  try {
    const r = await t.req('GET', `/api/lookup/${PARTIAL_ISBN}`);
    assert.equal(r.status, 200);
    assert.equal(r.json.found, false);
    assert.equal(r.json.book, null);
    assert.equal(r.json.partial.publisher, 'Scribner');
    assert.equal(r.json.partial.pages, 180);
    assert.ok(r.json.partial.subjects.includes('Rich people'));
    // Without a title the server still refuses to invent one...
    assert.equal((await t.req('POST', '/api/books', { isbn: PARTIAL_ISBN })).status, 404);
    // ...but with the user's title it keeps the partial data and classifies from it.
    const add = await t.req('POST', '/api/books', { isbn: PARTIAL_ISBN, title: 'The Great Gatsby' });
    assert.equal(add.status, 201, add.text);
    assert.equal(add.json.publisher, 'Scribner');
    assert.equal(add.json.pages, 180);
    assert.ok(add.json.tags.includes('rich people'));
    assert.deepEqual(add.json.meta_raw.subjects.slice(0, 1), ['Rich people']);
  } finally { await t.close(); }
});

test('nothing known at all: lookup says not found with no partial', async () => {
  const t = await boot({ fetchImpl: fakeFetch({ ol: {}, gb: { totalItems: 0 } }) });
  try {
    const r = await t.req('GET', `/api/lookup/${DUNE}`);
    assert.equal(r.json.found, false); assert.equal(r.json.partial, null);
  } finally { await t.close(); }
});

test('cover photo: upload attaches and serves it; re-upload replaces and deletes the old file', async () => {
  const t = await boot({ fetchImpl: fakeFetch({ ol: {}, gb: { totalItems: 0 } }) });
  try {
    const { json: b } = await t.req('POST', '/api/books', { title: 'Home Made Zine' });
    assert.equal(b.has_cover, false);

    const up = await putCover(t, b.id, JPG);
    assert.equal(up.status, 200);
    assert.equal(up.json.has_cover, true);
    assert.match(up.json.cover_file, new RegExp(`^${b.id}-[a-z0-9]+\\.jpg$`));
    const f1 = join(t.dataDir, 'covers', up.json.cover_file);
    assert.ok(existsSync(f1));
    const served = await fetch(`${t.base}/covers/${up.json.cover_file}`);
    assert.equal(served.status, 200);
    assert.equal(served.headers.get('content-type'), 'image/jpeg');
    assert.equal(Buffer.from(await served.arrayBuffer()).length, JPG.length);

    await new Promise((r) => setTimeout(r, 5));          // ensure a different timestamp in the filename
    const up2 = await putCover(t, b.id, PNG, { 'Content-Type': 'application/octet-stream' }); // type comes from the bytes
    assert.equal(up2.status, 200);
    assert.match(up2.json.cover_file, /\.png$/);
    assert.notEqual(up2.json.cover_file, up.json.cover_file);   // new URL => no stale browser/service-worker cache
    assert.equal(existsSync(f1), false);                          // old file cleaned up
    assert.ok(existsSync(join(t.dataDir, 'covers', up2.json.cover_file)));
  } finally { await t.close(); }
});

test('cover photo: replaces a provider cover too, and survives book deletion cleanup', async () => {
  const t = await boot();
  try {
    const { json: b } = await t.req('POST', '/api/books', { isbn: DUNE });
    await t.idle();
    assert.equal(t.db.get(b.id).cover_file, `${b.id}.jpg`);       // provider-supplied cover, fetched in the background
    const up = await putCover(t, b.id, JPG);
    assert.equal(existsSync(join(t.dataDir, 'covers', `${b.id}.jpg`)), false);
    await t.req('DELETE', `/api/books/${b.id}`);
    assert.equal(existsSync(join(t.dataDir, 'covers', up.json.cover_file)), false);
  } finally { await t.close(); }
});

test('cover photo: rejects non-images (even if labelled image/jpeg), SVG, tiny and oversized bodies, unknown books', async () => {
  const t = await boot();
  try {
    const { json: b } = await t.req('POST', '/api/books', { title: 'X' });
    assert.equal((await putCover(t, b.id, Buffer.from('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>'))).status, 415);
    assert.equal((await putCover(t, b.id, Buffer.from('GIF89a' + 'x'.repeat(100)))).status, 415);
    assert.equal((await putCover(t, b.id, Buffer.from('not an image at all, just text'))).status, 415);
    assert.equal((await putCover(t, b.id, Buffer.alloc(3))).status, 415);
    assert.equal((await putCover(t, b.id, Buffer.concat([JPG.subarray(0, 4), Buffer.alloc(6 * 1024 * 1024)]))).status, 413);
    assert.equal((await putCover(t, 9999, JPG)).status, 404);
    assert.equal(t.db.get(b.id).has_cover, false);                 // nothing got attached by the failures
  } finally { await t.close(); }
});

test('cover photo: requires login and rejects cross-origin writes', async () => {
  const t = await boot({ password: 'correct horse battery' });
  try {
    const login = await t.req('POST', '/api/login', { password: 'correct horse battery' });
    const cookie = login.headers.get('set-cookie').split(';')[0];
    const { json: b } = await t.req('POST', '/api/books', { title: 'X' }, { Cookie: cookie });
    assert.equal((await putCover(t, b.id, JPG)).status, 401);                                   // no cookie
    assert.equal((await putCover(t, b.id, JPG, { Cookie: cookie, Origin: 'https://evil.example' })).status, 403);
    assert.equal((await putCover(t, b.id, JPG, { Cookie: cookie })).status, 200);
  } finally { await t.close(); }
});


// ------------------------------------------------ scan-speed behaviour
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Fetch fake with per-host delays and a call log, for timing/ordering tests. */
function timedFetch({ olDelay = 0, gbDelay = 0, coverDelay = 0, gbStatus = 200, gbHeaders = null } = {}) {
  const calls = { ol: 0, gb: 0, cover: 0 };
  const f = async (url) => {
    const u = String(url);
    if (u.includes('openlibrary.org/api/books')) {                     // answers for whichever ISBN was asked, with Dune's record
      calls.ol++; await sleep(olDelay);
      const key = decodeURIComponent(u.match(/bibkeys=([^&]+)/)[1]);
      return { ok: true, status: 200, json: async () => ({ [key]: OL[`ISBN:${DUNE}`] }) };
    }
    if (u.includes('googleapis.com/books')) {
      calls.gb++; await sleep(gbDelay);
      return { ok: gbStatus < 400, status: gbStatus, headers: gbHeaders ? new Map(Object.entries(gbHeaders)) : undefined, json: async () => GB };
    }
    calls.cover++; await sleep(coverDelay);
    return { ok: true, status: 200, headers: new Map([['content-type', 'image/jpeg']]), arrayBuffer: async () => JPEG };
  };
  f.calls = calls;
  return f;
}

test('speed: a slow cover host does not delay the add; the cover arrives afterwards', async () => {
  const f = timedFetch({ coverDelay: 400 });
  const t = await boot({ fetchImpl: f });
  try {
    const t0 = Date.now();
    const add = await t.req('POST', '/api/books', { isbn: DUNE });
    const took = Date.now() - t0;
    assert.equal(add.status, 201);
    assert.ok(took < 300, `add took ${took} ms; it should not wait for a 400 ms cover download`);
    assert.equal(add.json.has_cover, false);
    await t.idle();
    assert.equal(t.db.get(add.json.id).has_cover, true);
  } finally { await t.close(); }
});

test('speed: a cover that arrives after the book was deleted leaves no file or error behind', async () => {
  const t = await boot({ fetchImpl: timedFetch({ coverDelay: 150 }) });
  try {
    const { json: b } = await t.req('POST', '/api/books', { isbn: DUNE });
    await t.req('DELETE', `/api/books/${b.id}`);
    await t.idle();
    assert.equal(existsSync(join(t.dataDir, 'covers', `${b.id}.jpg`)), false);
  } finally { await t.close(); }
});

test('speed: a late provider cover never replaces a photo the user uploaded meanwhile', async () => {
  const t = await boot({ fetchImpl: timedFetch({ coverDelay: 200 }) });
  try {
    const { json: b } = await t.req('POST', '/api/books', { isbn: DUNE });
    const up = await putCover(t, b.id, JPG);                       // user photo lands first
    assert.equal(up.status, 200);
    await t.idle();
    const cur = t.db.get(b.id);
    assert.equal(cur.cover_file, up.json.cover_file);
    assert.equal(existsSync(join(t.dataDir, 'covers', `${b.id}.jpg`)), false); // the provider's copy was discarded
  } finally { await t.close(); }
});

test('speed: a reused book id never inherits the previous book\'s late cover', async () => {
  const t = await boot({ fetchImpl: timedFetch({ coverDelay: 250 }) });
  try {
    const { json: first } = await t.req('POST', '/api/books', { isbn: DUNE });          // cover still downloading...
    await t.req('DELETE', `/api/books/${first.id}`);                                      // ...book deleted; SQLite may reuse the id
    const { json: second } = await t.req('POST', '/api/books', { title: 'Different book, no ISBN' });
    assert.equal(second.id, first.id, 'precondition: the id was reused');
    await t.idle();
    assert.equal(t.db.get(second.id).has_cover, false);
  } finally { await t.close(); }
});

test('speed: lookup answers on the first complete record instead of waiting for the slow provider', async () => {
  const f = timedFetch({ olDelay: 20, gbDelay: 600 });
  const lookup = createLookup({ fetchImpl: f, graceMs: 80 });
  const t0 = Date.now();
  const m = await lookup(DUNE);
  const took = Date.now() - t0;
  assert.ok(took < 350, `lookup took ${took} ms; it should not wait 600 ms for Google`);
  assert.equal(m.title, 'Dune');
  assert.deepEqual(m.sources, ['openlibrary']);
  assert.equal(m.description, '');                                // Google's description hasn't landed yet
  await sleep(700);                                               // ...but when it does, the cached record is upgraded
  const later = await lookup(DUNE);
  assert.deepEqual(later.sources, ['openlibrary', 'googlebooks']);
  assert.match(later.description, /Arrakis/);
  assert.equal(f.calls.gb, 1);                                    // still just one request to each provider
});

test('speed: concurrent lookups of the same ISBN share one provider request', async () => {
  const f = timedFetch({ olDelay: 50, gbDelay: 50 });
  const lookup = createLookup({ fetchImpl: f });
  const [a, b] = await Promise.all([lookup(DUNE), lookup(DUNE)]);
  assert.equal(a, b);
  assert.deepEqual(f.calls, { ol: 1, gb: 1, cover: 0 });
});

test('breaker: a throttling provider (429) is skipped on later scans, then probed again after Retry-After', async () => {
  let clock = 1_000_000;
  const f = timedFetch({ gbStatus: 429, gbHeaders: { 'retry-after': '30' } });
  const lookup = createLookup({ fetchImpl: f, now: () => clock, graceMs: 50 });
  const isbns = ['9780441013593', '9780547928227', '9780140449136', '9780375704024'];
  const first = await lookup(isbns[0]);
  assert.equal(first.title, 'Dune');                              // Open Library still answers
  await sleep(10);
  assert.equal(f.calls.gb, 1);
  assert.equal(lookup.breaker.isOpen('googlebooks'), true);
  assert.equal(lookup.breaker.remainingMs('googlebooks'), 30_000); // Retry-After honoured

  const second = await lookup(isbns[1]);                          // different ISBN, so not a cache hit
  assert.equal(f.calls.gb, 1, 'Google must not be called while it is paused');
  assert.match(second.provider_errors.find((e) => e.provider === 'googlebooks').error, /paused/);

  clock += 31_000;                                                // Retry-After elapsed
  await lookup(isbns[2]);
  await sleep(10);
  assert.equal(f.calls.gb, 2, 'after the pause, Google is probed again');
});

test('breaker: ordinary failures only pause a provider after two in a row; a success resets the count', () => {
  let clock = 0;
  const b = createBreaker({ now: () => clock });
  b.failure('x', new Error('timeout'));
  assert.equal(b.isOpen('x'), false);                             // one blip is tolerated
  b.success('x');
  b.failure('x', new Error('timeout'));
  assert.equal(b.isOpen('x'), false);                             // the success reset the streak
  b.failure('x', new Error('timeout'));
  assert.equal(b.isOpen('x'), true);                              // two in a row
  assert.equal(b.remainingMs('x'), 15_000);
  clock += 15_001;
  assert.equal(b.isOpen('x'), false);
  b.failure('x', new Error('timeout'));                           // the probe fails too: back off for longer
  assert.equal(b.remainingMs('x'), 30_000);
});

test('breaker: when every provider is paused the lookup reports an outage, not "not found"', async () => {
  const throttled = async () => ({ ok: false, status: 429, json: async () => ({}) });
  const lookup = createLookup({ fetchImpl: throttled });
  await assert.rejects(lookup(DUNE), LookupError);                // both answer 429, so both get paused
  await assert.rejects(lookup('9780547928227'), (e) => e instanceof LookupError && e.providers.every((p) => /paused/.test(p.error)));
});

test('cache: bounded, oldest entries evicted first', async () => {
  const isbnOf = (i) => { const core = '978' + String(i).padStart(9, '0'); let s = 0; for (let k = 0; k < 12; k++) s += +core[k] * (k % 2 ? 3 : 1); return core + ((10 - (s % 10)) % 10); };
  const fetchImpl = async (url) => {
    const m = String(url).match(/ISBN:(\d+)/);
    const body = m ? { [`ISBN:${m[1]}`]: { title: `T ${m[1]}`, authors: [{ name: 'A' }] } } : { totalItems: 0 };
    return { ok: true, status: 200, json: async () => body };
  };
  const lookup = createLookup({ fetchImpl, maxCache: 2, graceMs: 10 });
  for (const i of [1, 2, 3]) await lookup(isbnOf(i));
  assert.equal(lookup.cacheSize(), 2);                            // 3 looked up, only the newest 2 kept
});


// ------------------------------------------------ title search
const GATSBY = '9780743273565';
const OL_SEARCH = { docs: [
  { key: '/works/OL468431W', title: 'The Great Gatsby', author_name: ['F. Scott Fitzgerald'], first_publish_year: 1925,
    isbn: ['1234567890123', 'not-an-isbn', GATSBY, '9780141182636'], cover_i: 8410459, publisher: ['Scribner', 'Penguin'] },
  { key: '/works/OL1W', title: "Gatsby's Girl", author_name: ['Caroline Preston'], first_publish_year: 2007 },            // no ISBN at all
  { key: '/works/OL2W', title: 'Something Else Entirely', author_name: ['Nobody'], first_publish_year: 1999, isbn: ['9780000001009'] },
] };
const GB_SEARCH = { items: [
  { volumeInfo: { title: 'The Great Gatsby', authors: ['F. Scott Fitzgerald'], publishedDate: '2004-09-30', publisher: 'Scribner',
    industryIdentifiers: [{ type: 'ISBN_10', identifier: toIsbn10(GATSBY) }, { type: 'ISBN_13', identifier: GATSBY }], imageLinks: { thumbnail: 'http://books.google.com/g.jpg&edge=curl' } } },
  { volumeInfo: { title: 'Great Gatsby Study Guide', authors: ['Some Teacher'], publishedDate: '2012',
    industryIdentifiers: [{ type: 'ISBN_10', identifier: '0140449132' }] } },                                              // ISBN-10 only
  { volumeInfo: { title: 'The Great Gatsby', authors: ['F. Scott Fitzgerald'], publishedDate: '1999' } },                      // same work, no ISBN
] };

function searchFetch({ ol = OL_SEARCH, gb = GB_SEARCH, olStatus = 200, gbStatus = 200, olDelay = 0, gbDelay = 0 } = {}) {
  const calls = { ol: 0, gb: 0, urls: [] };
  const f = async (url) => {
    const u = String(url); calls.urls.push(u);
    if (u.includes('openlibrary.org/search.json')) { calls.ol++; await sleep(olDelay); return { ok: olStatus < 400, status: olStatus, json: async () => ol }; }
    if (u.includes('googleapis.com/books/v1/volumes')) { calls.gb++; await sleep(gbDelay); return { ok: gbStatus < 400, status: gbStatus, json: async () => gb }; }
    return { ok: true, status: 200, headers: new Map([['content-type', 'image/jpeg']]), arrayBuffer: async () => JPEG };
  };
  f.calls = calls;
  return f;
}

test('search: merges both providers, dedupes, fills gaps, converts ISBN-10, ignores junk ISBNs, ranks by title match', async () => {
  const search = createSearch({ fetchImpl: searchFetch(), graceMs: 50 });
  const { results } = await search('great gatsby');
  const titles = results.map((r) => r.title);
  assert.equal(titles[0], 'The Great Gatsby');                                  // exact-ish title match first
  assert.equal(titles.filter((t) => t === 'The Great Gatsby').length, 1);       // 3 copies across providers -> 1 row
  const g = results[0];
  assert.equal(g.isbn13, GATSBY);                                               // the valid one; the junk entries were skipped
  assert.deepEqual(g.sources.sort(), ['googlebooks', 'openlibrary']);
  assert.equal(g.publisher, 'Scribner');
  assert.equal(g.cover_url, 'https://covers.openlibrary.org/b/id/8410459-M.jpg');  // first provider's cover wins, gaps still filled
  const guide = results.find((r) => r.title === 'Great Gatsby Study Guide');
  assert.equal(guide.isbn13, '9780140449136');                                  // ISBN-10 -> ISBN-13
  assert.equal(results.find((r) => r.title === "Gatsby's Girl").isbn13, null);  // kept, just without an ISBN
  assert.equal(titles.at(-1), 'Something Else Entirely');                       // unrelated result ranks last
});

test('search: unknown-ISBN Google https-ifies thumbnails and drops the curl edge param', async () => {
  const gb = await searchGoogleBooks('x', searchFetch());
  assert.equal(gb[0].cover_url, 'https://books.google.com/g.jpg');
});

test('search endpoint: validates input, marks books already in the library, annotates by ISBN', async () => {
  const f = searchFetch();
  const t = await boot({ fetchImpl: f });
  try {
    assert.equal((await t.req('GET', '/api/search')).status, 400);
    assert.equal((await t.req('GET', '/api/search?q=a')).status, 400);
    assert.equal((await t.req('GET', `/api/search?q=${'x'.repeat(201)}`)).status, 400);
    const before = await t.req('GET', '/api/search?q=great%20gatsby');
    assert.equal(before.status, 200);
    assert.equal(before.json.query, 'great gatsby');
    assert.equal(before.json.results[0].in_library, null);

    await t.req('POST', '/api/books', { isbn: GATSBY, title: 'The Great Gatsby' });   // lookup finds nothing in this fake, so the title is supplied
    const after = await t.req('GET', '/api/search?q=great%20gatsby%20%20');            // extra spaces are normalised, cache is per normalised query
    assert.ok(after.json.results[0].in_library > 0);
  } finally { await t.close(); }
});

test('search: one provider failing still returns the other\'s results; both failing is a 502', async () => {
  const half = createSearch({ fetchImpl: searchFetch({ gbStatus: 500 }), graceMs: 20 });
  const r = await half('great gatsby');
  assert.ok(r.results.length > 0);
  assert.equal(r.provider_errors[0].provider, 'googlebooks');

  const t = await boot({ fetchImpl: searchFetch({ olStatus: 503, gbStatus: 500 }) });
  try {
    const res = await t.req('GET', '/api/search?q=anything');
    assert.equal(res.status, 502);
    assert.equal(res.json.providers.length, 2);
  } finally { await t.close(); }
});

test('search: no results is a normal empty answer, not an error', async () => {
  const t = await boot({ fetchImpl: searchFetch({ ol: { docs: [] }, gb: { totalItems: 0 } }) });
  try {
    const res = await t.req('GET', '/api/search?q=zzzzzz');
    assert.equal(res.status, 200);
    assert.deepEqual(res.json.results, []);
  } finally { await t.close(); }
});

test('search: malformed provider payloads are tolerated', async () => {
  const f = searchFetch({ ol: { docs: [null, 5, { title: 42 }, { title: 'Ok', isbn: 'x', cover_i: 'nope', author_name: 'str' }] }, gb: { items: [null, {}, { volumeInfo: { title: ['no'] } }, { volumeInfo: { title: 'Fine', industryIdentifiers: 'bad' } }] } });
  const { results } = await createSearch({ fetchImpl: f, graceMs: 10 })('ok fine');
  assert.deepEqual(results.map((r) => r.title).sort(), ['Fine', 'Ok']);
  assert.ok(results.every((r) => r.isbn13 === null && r.cover_url === ''));
  assert.deepEqual(await searchOpenLibrary('x', searchFetch({ ol: { docs: 'nope' } })), []);
});

test('search: needs login', async () => {
  const t = await boot({ password: 'correct horse battery', fetchImpl: searchFetch() });
  try { assert.equal((await t.req('GET', '/api/search?q=gatsby')).status, 401); } finally { await t.close(); }
});

test('search: same work with different edition ISBNs from the two providers is one row', () => {
  const merged = mergeSearchResults('dune', [
    [{ title: 'Dune', authors: ['Frank Herbert'], isbn13: '9780441013593', cover_url: '', published: '1965', publisher: '', source: 'openlibrary' }],
    [{ title: 'Dune', authors: ['Frank Herbert'], isbn13: '9780340960196', cover_url: 'https://x/y.jpg', published: '', publisher: 'Hodder', source: 'googlebooks' }],
  ]);
  assert.equal(merged.length, 1);
  assert.equal(merged[0].isbn13, '9780441013593');                  // first provider's edition kept
  assert.equal(merged[0].cover_url, 'https://x/y.jpg');             // gaps filled from the duplicate
  assert.equal(merged[0].publisher, 'Hodder');
});

test('search speed: answers on the first provider with results, caches, shares in-flight queries', async () => {
  const f = searchFetch({ olDelay: 10, gbDelay: 500 });
  const search = createSearch({ fetchImpl: f, graceMs: 60 });
  const t0 = Date.now();
  const [a, b] = await Promise.all([search('great gatsby'), search('Great  Gatsby')]);     // same query after normalising
  assert.ok(Date.now() - t0 < 300, 'should not wait 500 ms for the slow provider');
  assert.equal(a, b);
  assert.deepEqual(a.results[0].sources, ['openlibrary']);
  assert.equal(f.calls.ol, 1);
  await sleep(600);                                                 // the slow provider lands and upgrades the cached entry
  const later = await search('great gatsby');
  assert.deepEqual(later.results[0].sources.sort(), ['googlebooks', 'openlibrary']);
  assert.equal(f.calls.gb, 1);
  assert.equal(search.cacheSize(), 1);
});

test('search shares the circuit breaker with lookup: a provider throttling lookups is skipped by search', async () => {
  const f = timedFetch({ gbStatus: 429 });
  const lookup = createLookup({ fetchImpl: f, graceMs: 20 });
  await lookup(DUNE);                                               // Google answers 429 -> breaker opens
  await sleep(10);
  assert.equal(lookup.breaker.isOpen('googlebooks'), true);
  const searchF = searchFetch();
  const search = createSearch({ fetchImpl: searchF, breaker: lookup.breaker, graceMs: 20 });
  const r = await search('great gatsby');
  assert.equal(searchF.calls.gb, 0, 'search must not call a paused provider');
  assert.ok(r.results.length > 0);
});

test('add from a search result without an ISBN: cover_url is fetched only from allow-listed hosts', async () => {
  const f = searchFetch();
  const t = await boot({ fetchImpl: f });
  try {
    const ok = await t.req('POST', '/api/books', { title: "Gatsby's Girl", authors: ['Caroline Preston'], cover_url: 'https://covers.openlibrary.org/b/id/1-M.jpg' });
    assert.equal(ok.status, 201);
    const evil = await t.req('POST', '/api/books', { title: 'Evil', cover_url: 'https://evil.example.com/steal.jpg' });
    const internal = await t.req('POST', '/api/books', { title: 'Internal', cover_url: 'https://127.0.0.1:8080/secret' });
    assert.equal(evil.status, 201); assert.equal(internal.status, 201);
    await t.idle();
    assert.equal(t.db.get(ok.json.id).has_cover, true);
    assert.equal(t.db.get(evil.json.id).has_cover, false);
    assert.equal(t.db.get(internal.json.id).has_cover, false);
    assert.ok(!f.calls.urls.some((u) => /evil\.example|127\.0\.0\.1/.test(u)), 'the server must never request a non-allow-listed host');
  } finally { await t.close(); }
});

test('search ranking ignores a leading article, and prefers an exact title over a longer one that merely starts with it', () => {
  const mk = (title) => ({ title, authors: ['A'], isbn13: null, cover_url: '', published: '', publisher: '', source: 'openlibrary' });
  const r = mergeSearchResults('great gatsby', [[mk('Great Gatsby Study Guide'), mk('The Great Gatsby'), mk('Gatsby')]]);
  assert.deepEqual(r.map((x) => x.title), ['The Great Gatsby', 'Great Gatsby Study Guide', 'Gatsby']);
  const r2 = mergeSearchResults('the hobbit', [[mk('The Hobbit: There and Back Again'), mk('Hobbit')]]);
  assert.equal(r2[0].title, 'Hobbit');                                // exact once articles are ignored
});
