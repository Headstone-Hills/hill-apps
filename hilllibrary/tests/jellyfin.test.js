import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { LibraryDb } from '../server/db.js';
import { createApp } from '../server/app.js';
import { createLookup } from '../server/metadata.js';
import { createJellyfin, normalizeItem, combineMovies, matchTitle } from '../server/jellyfin.js';

const id = (n) => n.toString(16).padStart(32, '0');
// Shapes follow Jellyfin's documented BaseItemDto (https://api.jellyfin.org/).
const JF = [
  { Id: id(1), Name: 'Seven Samurai', ProductionYear: 1954, ProviderIds: { Tmdb: '346' }, Genres: ['Action', 'Drama'], Overview: 'Bandits.', RunTimeTicks: 12420000000 * 10 / 10 * 1, People: [{ Name: 'Akira Kurosawa', Type: 'Director' }, { Name: 'Toshiro Mifune', Type: 'Actor' }], Studios: [{ Name: 'Toho' }], ImageTags: { Primary: 'abc' } },
  { Id: id(2), Name: 'Alien', ProductionYear: 1979, ProviderIds: { Tmdb: '348' }, Genres: ['Horror', 'Science Fiction'], Overview: 'In space.', ImageTags: {} },
  { Id: id(3), Name: 'The Matrix', ProductionYear: 1999, ProviderIds: {}, Genres: ['Science Fiction', 'Action'], ImageTags: { Primary: 'm' } },
  { Id: 'not-an-id', Name: 'Broken' },
];

const MOVIES_LIB = id(900), LYNCH_LIB = id(901), HOME_LIB = id(902);
const LIBRARIES = [
  { Name: 'Movies', CollectionType: 'movies', ItemId: MOVIES_LIB },
  { Name: 'David Lynch Ephemera', ItemId: LYNCH_LIB },                 // no type set, like on a real server
  { Name: 'Concerts', CollectionType: 'homevideos', ItemId: HOME_LIB },
  { Name: 'Shows', CollectionType: 'tvshows', ItemId: id(903) },
];

function fakeJellyfin({ status = 200, items = JF, libraries = LIBRARIES, onCall } = {}) {
  const calls = [];
  const f = async (url, opts = {}) => {
    const u = new URL(String(url));
    calls.push({ path: u.pathname, search: u.searchParams, auth: opts.headers?.Authorization });
    onCall?.(u);
    if (status !== 200) return { ok: false, status, json: async () => ({}) };
    if (u.pathname === '/Library/VirtualFolders') {
      return { ok: true, status: 200, json: async () => libraries };
    }
    if (u.pathname === '/Items') {
      const start = Number(u.searchParams.get('StartIndex')), limit = Number(u.searchParams.get('Limit'));
      const mine = items.filter((x) => (x._lib ?? MOVIES_LIB) === u.searchParams.get('ParentId'));   // only the requested library
      return { ok: true, status: 200, json: async () => ({ Items: mine.slice(start, start + limit), TotalRecordCount: mine.length }) };
    }
    if (u.pathname.endsWith('/Images/Primary')) {
      if (u.pathname.includes(id(2))) return { ok: false, status: 404 };
      return { ok: true, status: 200, headers: new Map([['content-type', 'image/jpeg']]), arrayBuffer: async () => Buffer.alloc(300, 9) };
    }
    throw new Error('unexpected ' + u);
  };
  f.calls = calls;
  return f;
}

async function boot({ jellyfin, db = new LibraryDb(':memory:') } = {}) {
  const dataDir = mkdtempSync(join(tmpdir(), 'hl-jf-'));
  const publicDir = join(dataDir, 'public');
  mkdirSync(publicDir); writeFileSync(join(publicDir, 'index.html'), 'x');
  const server = createServer(createApp({ db, dataDir, publicDir, lookup: createLookup({ fetchImpl: async () => { throw new Error('offline'); } }), jellyfin }));
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  return {
    db,
    async req(method, path, body) {
      const res = await fetch(base + path, { method, headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined });
      const buf = Buffer.from(await res.arrayBuffer());
      let json = null; try { json = JSON.parse(buf.toString()); } catch { /* binary */ }
      return { status: res.status, json, buf, headers: res.headers };
    },
    close: () => new Promise((r) => { server.close(r); db.close(); }),
  };
}
const client = (o = {}) => createJellyfin({ url: 'http://jf.local:8096/', apiKey: 'SECRET', fetchImpl: fakeJellyfin(o), ...o });

// ------------------------------------------------------------------ units

test('normalizeItem: reads ids, year, genres, director, runtime in minutes; rejects malformed items', () => {
  const m = normalizeItem({ ...JF[0], RunTimeTicks: 124 * 60 * 10_000_000 });
  assert.deepEqual(m, { id: id(1), title: 'Seven Samurai', year: '1954', overview: 'Bandits.', genres: ['Action', 'Drama'], studio: 'Toho', directors: ['Akira Kurosawa'], runtime: 124, tmdbId: 346, imageTag: 'abc' });
  assert.equal(normalizeItem(JF[3]), null);
  assert.equal(normalizeItem({ Id: id(9), Name: 'No Extras' }).tmdbId, null);
  assert.equal(normalizeItem(null), null);
});

test('matchTitle ignores edition suffixes, articles, case and punctuation', () => {
  assert.equal(matchTitle('Seven Samurai (Criterion Collection)'), matchTitle('seven samurai'));
  assert.equal(matchTitle('The Matrix'), matchTitle('Matrix'));
  assert.equal(matchTitle("Ocean's Eleven"), matchTitle('Oceans  Eleven'));
});

test('combineMovies: TMDB id first, then title+year; unmatched on either side stays visible', () => {
  const server = [JF[0], JF[1], JF[2]].map(normalizeItem);
  const discs = [
    { id: 1, title: 'Seven Samurai (Criterion Collection)', published: '1954', media: 'dvd', meta_raw: { tmdb: { id: 346, kind: 'movie' } }, genre: 'Action & Adventure', subgenre: 'Drama', authors: ['Akira Kurosawa'], has_cover: true, cover_file: '1.jpg', updated_at: 'T1' },
    { id: 2, title: 'Matrix', published: '1998', media: 'bluray', meta_raw: {}, genre: 'Uncategorized', authors: [], has_cover: false },  // year off by one: still the same film
    { id: 3, title: 'Heat', published: '1995', media: 'dvd', meta_raw: {}, genre: 'Crime', authors: [] },
    { id: 4, title: 'Seven Samurai', published: '1954', media: 'bluray', meta_raw: { tmdb: { id: 346, kind: 'movie' } }, genre: 'Drama', authors: [] }, // 2nd copy of the same film
  ];
  const out = combineMovies(discs, server);
  const by = Object.fromEntries(out.map((e) => [e.title, e]));
  assert.equal(out.length, 4);
  assert.equal(by['Seven Samurai (Criterion Collection)'].availability, 'both');
  assert.deepEqual(by['Seven Samurai (Criterion Collection)'].copies.map((c) => c.media), ['dvd', 'bluray']);
  assert.equal(by['Seven Samurai (Criterion Collection)'].poster, '/covers/1.jpg?v=T1');          // your own cover wins
  assert.equal(by.Matrix.availability, 'both');
  assert.equal(by.Matrix.poster, `/api/jellyfin/image/${id(3)}?tag=m`);                              // else the server's poster
  assert.equal(by.Heat.availability, 'disc');
  assert.equal(by.Alien.availability, 'server');
  assert.equal(by.Alien.genre, 'Horror');                                                            // server-only films get movie shelves too
  assert.equal(by.Alien.poster, '');                                                                // no poster on the server: the app draws a cover
});

test('combineMovies: a different film with the same title and a far-off year is not merged; conflicting TMDB ids are never merged', () => {
  const server = [{ Id: id(5), Name: 'Alien', ProductionYear: 2021, ProviderIds: { Tmdb: '999' } }].map(normalizeItem);
  const discs = [{ id: 1, title: 'Alien', published: '1979', media: 'dvd', meta_raw: { tmdb: { id: 348, kind: 'movie' } }, genre: 'Horror', authors: [] }];
  assert.deepEqual(combineMovies(discs, server).map((e) => e.availability).sort(), ['disc', 'server']);
  const sameYear = [{ Id: id(6), Name: 'Alien', ProductionYear: 1979, ProviderIds: { Tmdb: '999' } }].map(normalizeItem);
  assert.equal(combineMovies(discs, sameYear).length, 2);
});

test('TV series TMDB ids are not used to match movies', () => {
  const discs = [{ id: 1, title: 'Friends', published: '1994', media: 'dvd', meta_raw: { tmdb: { id: 1668, kind: 'tv' } }, genre: 'Comedy', authors: [] }];
  const server = [{ Id: id(7), Name: 'Other', ProductionYear: 2000, ProviderIds: { Tmdb: '1668' } }].map(normalizeItem);
  assert.equal(combineMovies(discs, server).length, 2);
});

// ------------------------------------------------------------------ client

test('client: sends the API key as a MediaBrowser token, pages through the library, skips bad items', async () => {
  const many = Array.from({ length: 1200 }, (_, i) => ({ Id: id(100 + i), Name: `Film ${i}`, ProductionYear: 2000 }));
  const f = fakeJellyfin({ items: many });
  const jf = createJellyfin({ url: 'http://jf.local:8096/', apiKey: 'SECRET', fetchImpl: f });
  const { items } = await jf.movies();
  assert.equal(items.length, 1200);
  const itemCalls = () => f.calls.filter((c) => c.path === '/Items');
  assert.equal(itemCalls().length, 3);                                               // 500 + 500 + 200
  assert.equal(f.calls[0].auth, 'MediaBrowser Token="SECRET"');
  assert.equal(itemCalls()[0].search.get('IncludeItemTypes'), 'Movie');
  assert.match(itemCalls()[0].search.get('Fields'), /ProviderIds/);
  await jf.movies();
  assert.equal(itemCalls().length, 3);                                               // cached
  await jf.movies({ force: true });
  assert.equal(itemCalls().length, 6);
});

test('client: not configured means disabled and empty; auth and network failures are described', async () => {
  const off = createJellyfin({});
  assert.equal(off.enabled, false);
  assert.deepEqual((await off.movies()).items, []);
  await assert.rejects(() => createJellyfin({ url: 'http://x', apiKey: 'bad', fetchImpl: fakeJellyfin({ status: 401 }) }).movies(), /rejected the API key/);
  await assert.rejects(() => createJellyfin({ url: 'http://x', apiKey: 'k', fetchImpl: fakeJellyfin({ status: 500 }) }).movies(), /HTTP 500/);
  await assert.rejects(() => createJellyfin({ url: 'http://x', apiKey: 'k', fetchImpl: async () => { throw Object.assign(new Error('fetch failed'), { cause: { code: 'ECONNREFUSED' } }); } }).movies(), /ECONNREFUSED/);
});

// ------------------------------------------------------------------ API

test('API /api/movies: discs plus server movies, with connection status', async () => {
  const t = await boot({ jellyfin: client() });
  try {
    await t.req('POST', '/api/books', { media: 'dvd', title: 'Alien', published: '1979', genre: 'Horror' });
    await t.req('POST', '/api/books', { media: 'bluray', title: 'Heat', published: '1995' });
    await t.req('POST', '/api/books', { title: 'A Novel' });                       // books never appear
    const r = (await t.req('GET', '/api/movies')).json;
    assert.equal(r.jellyfin.ok, true);
    assert.equal(r.jellyfin.count, 3);
    const by = Object.fromEntries(r.items.map((e) => [e.title, e.availability]));
    assert.deepEqual(by, { Alien: 'both', Heat: 'disc', 'Seven Samurai': 'server', 'The Matrix': 'server' });
    assert.equal(r.items.some((e) => e.title === 'A Novel'), false);
  } finally { await t.close(); }
});

test('API /api/movies: with Jellyfin off or failing, discs still show and the reason is reported', async () => {
  const off = await boot({});
  try {
    await off.req('POST', '/api/books', { media: 'dvd', title: 'Heat' });
    const r = (await off.req('GET', '/api/movies')).json;
    assert.equal(r.jellyfin.enabled, false);
    assert.deepEqual(r.items.map((e) => e.availability), ['disc']);
  } finally { await off.close(); }
  const bad = await boot({ jellyfin: client({ status: 401 }) });
  try {
    await bad.req('POST', '/api/books', { media: 'dvd', title: 'Heat' });
    const r = (await bad.req('GET', '/api/movies')).json;
    assert.equal(r.jellyfin.enabled, true);
    assert.equal(r.jellyfin.ok, false);
    assert.match(r.jellyfin.error, /rejected the API key/);
    assert.equal(r.items.length, 1);
    assert.equal(JSON.stringify(r).includes('SECRET'), false);                      // the key never reaches the browser
  } finally { await bad.close(); }
});

test('API poster proxy: serves images, 404s missing ones, rejects malformed ids, and needs no key in the URL', async () => {
  const t = await boot({ jellyfin: client() });
  try {
    const ok = await t.req('GET', `/api/jellyfin/image/${id(1)}?tag=abc`);
    assert.equal(ok.status, 200);
    assert.equal(ok.headers.get('content-type'), 'image/jpeg');
    assert.equal(ok.buf.length, 300);
    assert.equal((await t.req('GET', `/api/jellyfin/image/${id(2)}`)).status, 404);
    assert.equal((await t.req('GET', '/api/jellyfin/image/..%2f..%2fetc')).status, 404);
    assert.equal((await t.req('GET', '/api/jellyfin/image/xyz')).status, 404);
  } finally { await t.close(); }
  const off = await boot({});
  try { assert.equal((await off.req('GET', `/api/jellyfin/image/${id(1)}`)).status, 404); } finally { await off.close(); }
});

test('duplicate Jellyfin entries: all poster candidates are kept, and the proxy falls through a stale one', async () => {
  // id(2) has a tagged poster Jellyfin cannot serve (404); its twin id(8) works.
  const twins = [
    { Id: id(2), Name: 'Alien', ProductionYear: 1979, ImageTags: { Primary: 'stale' } },
    { Id: id(8), Name: 'Alien', ProductionYear: 1979, ImageTags: { Primary: 'good' } },
  ];
  const out = combineMovies([], twins.map(normalizeItem));
  assert.equal(out.length, 1);                                                      // one film, not two
  assert.equal(out[0].poster, `/api/jellyfin/image/${id(2)}?tag=stale&also=${encodeURIComponent(`${id(8)}.good`)}`);
  const t = await boot({ jellyfin: client({ items: twins }) });
  try {
    const r = await t.req('GET', out[0].poster);
    assert.equal(r.status, 200);
    assert.equal(r.buf.length, 300);
    assert.equal((await t.req('GET', `/api/jellyfin/image/${id(2)}?tag=stale&also=zzz.bad,../../x`)).status, 404); // junk candidates are ignored
  } finally { await t.close(); }
});

test('API /api/movies?refresh=1 refetches from Jellyfin', async () => {
  const f = fakeJellyfin();
  const t = await boot({ jellyfin: createJellyfin({ url: 'http://jf', apiKey: 'k', fetchImpl: f }) });
  try {
    const items = () => f.calls.filter((c) => c.path === '/Items').length;
    await t.req('GET', '/api/movies'); await t.req('GET', '/api/movies');
    assert.equal(items(), 1);
    await t.req('GET', '/api/movies?refresh=1');
    assert.equal(items(), 2);
  } finally { await t.close(); }
});

test('a disc added by barcode remembers its exact TMDB id, which makes the Jellyfin link exact', async () => {
  const { createDiscLookup } = await import('../server/discs.js');
  const f = async (url) => {
    const u = String(url);
    const json = (b) => ({ ok: true, status: 200, json: async () => b });
    if (u.includes('upcitemdb')) return json({ code: 'OK', items: [{ title: 'Alien DVD', description: '', brand: 'Fox', category: 'Video > DVD' }] });
    if (u.includes('/search/')) return json({ results: [{ id: 348, title: 'Alien' }] });
    if (u.includes('/movie/348')) return json({ title: 'Alien', genres: [{ name: 'Horror' }], release_date: '1979-05-25', credits: { crew: [] } });
    throw new Error('unexpected ' + u);
  };
  const m = await createDiscLookup({ fetchImpl: f, tmdbKey: 'k' })('0024543072447');
  assert.deepEqual(m.tmdb, { id: 348, kind: 'movie' });
  const t = await boot({ jellyfin: client({ items: [{ Id: id(2), Name: 'Alien: Director’s Cut', ProductionYear: 1979, ProviderIds: { Tmdb: '348' } }] }) });
  try {
    const disc = (await t.req('POST', '/api/books', { media: 'dvd', title: 'Alien', published: '1979' })).json;
    t.db.update(disc.id, { meta_raw: { tmdb: { id: 348, kind: 'movie' } } });
    const r = (await t.req('GET', '/api/movies')).json;
    assert.deepEqual(r.items.map((e) => e.availability), ['both']);            // different titles, same TMDB id: still one film
  } finally { await t.close(); }
});

// ------------------------------------------------------------------ library selection

test('only the Movies library is read: ephemera, home-video and other libraries never appear', async () => {
  const items = [
    { Id: id(1), Name: 'Alien', ProductionYear: 1979 },
    { Id: id(2), Name: 'Heat', ProductionYear: 1995 },
    { Id: id(20), Name: 'David Lynch Daily Weather Report', ProductionYear: 2009, _lib: LYNCH_LIB },
    { Id: id(21), Name: 'Concert Film', ProductionYear: 2001, _lib: HOME_LIB },
  ];
  const f = fakeJellyfin({ items });
  const jf = createJellyfin({ url: 'http://jf', apiKey: 'k', fetchImpl: f });
  const r = await jf.movies();
  assert.deepEqual(r.items.map((i) => i.title), ['Alien', 'Heat']);
  assert.deepEqual(r.libraries, ['Movies']);
  assert.deepEqual([...new Set(f.calls.filter((c) => c.path === '/Items').map((c) => c.search.get('ParentId')))], [MOVIES_LIB]);
});

test('JELLYFIN_MOVIE_LIBRARY picks a library by name (case-insensitive); an unknown name is a clear error', async () => {
  const items = [{ Id: id(1), Name: 'Alien' }, { Id: id(20), Name: 'Lynch Short', _lib: LYNCH_LIB }];
  const named = createJellyfin({ url: 'http://jf', apiKey: 'k', library: 'david lynch ephemera', fetchImpl: fakeJellyfin({ items }) });
  assert.deepEqual((await named.movies()).items.map((i) => i.title), ['Lynch Short']);
  const wrong = createJellyfin({ url: 'http://jf', apiKey: 'k', library: 'Films', fetchImpl: fakeJellyfin({ items }) });
  await assert.rejects(() => wrong.movies(), /no library named "Films".*Movies, David Lynch Ephemera/);
});

test('several Movies-type libraries are combined without duplicates; none at all is an explanatory error', async () => {
  const libs = [{ Name: 'Movies', CollectionType: 'movies', ItemId: MOVIES_LIB }, { Name: 'Kids Movies', CollectionType: 'Movies', ItemId: LYNCH_LIB }];
  const items = [{ Id: id(1), Name: 'Alien' }, { Id: id(30), Name: 'Up', _lib: LYNCH_LIB }];
  const both = await createJellyfin({ url: 'http://jf', apiKey: 'k', fetchImpl: fakeJellyfin({ items, libraries: libs }) }).movies();
  assert.deepEqual(both.items.map((i) => i.title).sort(), ['Alien', 'Up']);
  assert.deepEqual(both.libraries, ['Movies', 'Kids Movies']);
  const none = createJellyfin({ url: 'http://jf', apiKey: 'k', fetchImpl: fakeJellyfin({ libraries: [{ Name: 'Shows', CollectionType: 'tvshows', ItemId: id(903) }] }) });
  await assert.rejects(() => none.movies(), /no library of type Movies.*JELLYFIN_MOVIE_LIBRARY/);
});

test('API /api/movies reports which library it read, and a missing library shows as a Jellyfin error with discs intact', async () => {
  const t = await boot({ jellyfin: client({ items: [{ Id: id(1), Name: 'Alien' }, { Id: id(20), Name: 'Lynch Short', _lib: LYNCH_LIB }] }) });
  try {
    const r = (await t.req('GET', '/api/movies')).json;
    assert.deepEqual(r.jellyfin.libraries, ['Movies']);
    assert.deepEqual(r.items.map((e) => e.title), ['Alien']);
  } finally { await t.close(); }
  const bad = await boot({ jellyfin: client({ libraries: [] }) });
  try {
    await bad.req('POST', '/api/books', { media: 'dvd', title: 'Heat' });
    const r = (await bad.req('GET', '/api/movies')).json;
    assert.equal(r.jellyfin.ok, false);
    assert.match(r.jellyfin.error, /no library of type Movies/);
    assert.equal(r.items.length, 1);
  } finally { await bad.close(); }
});
