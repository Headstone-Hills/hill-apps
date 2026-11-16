import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { toGtin13 } from '../server/barcode.js';
import { classifyMovie } from '../server/moviegenres.js';
import { titleCandidates, cleanDiscTitle, guessFormat, seriesName, createDiscLookup, createTmdbClient, fetchUpcItemDb } from '../server/discs.js';
import { LibraryDb } from '../server/db.js';
import { createApp } from '../server/app.js';
import { createLookup, LookupError } from '../server/metadata.js';

const SPIDEY = '043396001718';   // real UPC-A (valid checksum); fixture content below is modelled on UPCitemdb's documented shape
const ALIEN = '0024543072447';

// ---- fake providers: UPCitemdb + TMDB shapes follow their public docs ----
const UPC = {
  [SPIDEY.padStart(13, '0')]: { title: 'Spider-man DVD', description: 'Spider-man', brand: 'Sony', category: 'Electronics > Video > DVD' },
  [ALIEN]: { title: 'Alien (Blu-ray) [Widescreen]', description: 'In space no one can hear you scream.', brand: '20th Century Fox', category: 'Media > DVDs & Movies' },
};
const TMDB = {
  'spider-man': { id: 557, title: 'Spider-Man', genres: [{ name: 'Action' }, { name: 'Adventure' }, { name: 'Science Fiction' }], runtime: 121, release_date: '2002-05-01',
    overview: 'After being bitten by a genetically altered spider...', poster_path: '/poster.jpg', production_companies: [{ name: 'Columbia Pictures' }],
    credits: { crew: [{ job: 'Producer', name: 'Laura Ziskin' }, { job: 'Director', name: 'Sam Raimi' }] } },
  alien: { id: 348, title: 'Alien', genres: [{ name: 'Horror' }, { name: 'Science Fiction' }], runtime: 117, release_date: '1979-05-25', overview: 'A crew meets something.',
    poster_path: '/alien.jpg', production_companies: [{ name: 'Brandywine' }], credits: { crew: [{ job: 'Director', name: 'Ridley Scott' }] } },
};

function fakeFetch({ upcStatus = 200, tmdb = true } = {}) {
  const calls = [];
  const f = async (url) => {
    const u = String(url);
    calls.push(u);
    const json = (status, body) => ({ ok: status < 400, status, json: async () => body });
    if (/openlibrary\.org\/api\/books|googleapis\.com\/books/.test(u)) return json(200, {}); // books: nothing known
    if (u.includes('api.upcitemdb.com')) {
      if (upcStatus !== 200) return json(upcStatus, { code: 'TOO_FAST', message: 'Please slow down!' });
      const item = UPC[new URL(u).searchParams.get('upc')];
      return json(200, item ? { code: 'OK', total: 1, items: [item] } : { code: 'OK', total: 0, items: [] });
    }
    if (u.includes('api.themoviedb.org/3/search/multi')) {
      const q = new URL(u).searchParams.get('query').toLowerCase();
      const hits = Object.values(TMDB).filter((t) => t.title.toLowerCase().includes(q)).map((t) => ({ media_type: 'movie', id: t.id, title: t.title, release_date: t.release_date, overview: t.overview, poster_path: t.poster_path }));
      return json(200, { results: [{ media_type: 'person', id: 1, name: 'Someone' }, ...hits] });
    }
    if (u.includes('api.themoviedb.org/3/search/')) {
      if (!tmdb) return json(401, {});
      const q = new URL(u).searchParams.get('query').toLowerCase();
      return json(200, { results: TMDB[q] ? [{ id: TMDB[q].id, title: TMDB[q].title }] : [] });
    }
    const m = u.match(/api\.themoviedb\.org\/3\/movie\/(\d+)/);
    if (m) return json(200, Object.values(TMDB).find((t) => t.id === Number(m[1])));
    if (u.includes('image.tmdb.org')) return { ok: true, status: 200, headers: new Map([['content-type', 'image/jpeg']]), arrayBuffer: async () => Buffer.alloc(2048, 7) };
    throw new Error('unexpected fetch ' + u);
  };
  f.calls = calls;
  return f;
}

async function boot({ tmdbKey = 'k', fetchImpl = fakeFetch(), db = new LibraryDb(':memory:') } = {}) {
  const dataDir = mkdtempSync(join(tmpdir(), 'hl-disc-'));
  const publicDir = join(dataDir, 'public');
  mkdirSync(publicDir);
  writeFileSync(join(publicDir, 'index.html'), 'x');
  const handler = createApp({
    db, dataDir, publicDir, fetchImpl,
    lookup: createLookup({ fetchImpl }), lookupDisc: createDiscLookup({ fetchImpl, tmdbKey }), tmdb: createTmdbClient({ fetchImpl, key: tmdbKey }),
  });
  const server = createServer(handler);
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  return {
    db,
    async req(method, path, body) {
      const res = await fetch(base + path, { method, headers: body !== undefined ? { 'Content-Type': 'application/json' } : {}, body: body !== undefined ? JSON.stringify(body) : undefined });
      const text = await res.text();
      let json = null; try { json = JSON.parse(text); } catch { /* not json */ }
      return { status: res.status, json, text };
    },
    close: () => new Promise((r) => { server.close(r); db.close(); }),
  };
}

// ------------------------------------------------------------------ units

test('toGtin13 accepts UPC-A and EAN-13 with a good checksum, and pads UPC-A', () => {
  assert.equal(toGtin13(SPIDEY), '0043396001718');
  assert.equal(toGtin13('0043396001718'), '0043396001718');
  assert.equal(toGtin13('0043-3960 01718'), '0043396001718');
  assert.equal(toGtin13('043396001719'), null);      // bad check digit
  assert.equal(toGtin13('12345'), null);
  assert.equal(toGtin13('abcdefghijkl'), null);
});

test('cleanDiscTitle strips format and edition noise but keeps the title', () => {
  const cases = {
    'Spider-man DVD': 'Spider-man',
    'Alien (Blu-ray) [Widescreen]': 'Alien',
    'The Matrix - 4K Ultra HD + Blu-ray + Digital': 'The Matrix',
    'Hotel Transylvania 2 (Blu-ray + DVD + Digital Copy)': 'Hotel Transylvania 2',
    'Blade Runner: The Final Cut [Blu-ray]': 'Blade Runner: The Final Cut',
    'Up': 'Up',
  };
  for (const [raw, want] of Object.entries(cases)) assert.equal(cleanDiscTitle(raw), want, raw);
  assert.equal(cleanDiscTitle('DVD'), 'DVD'); // never returns an empty title
});

test('titleCandidates: edition suffixes and subtitles fall back to simpler searches', () => {
  assert.deepEqual(titleCandidates('Memories of Murder (Criterion Collection)'), ['Memories of Murder (Criterion Collection)', 'Memories of Murder']);
  assert.deepEqual(titleCandidates('Blade Runner: The Final Cut [Blu-ray]'), ['Blade Runner: The Final Cut', 'Blade Runner']);
  assert.deepEqual(titleCandidates('Alien'), ['Alien']);
});

test('disc lookup retries simpler titles, so an edition suffix does not lose the metadata', async () => {
  const base = fakeFetch();
  const f = async (url, o) => {
    const u = String(url);
    if (u.includes('api.upcitemdb.com')) return { ok: true, status: 200, json: async () => ({ code: 'OK', items: [{ title: 'Alien (Criterion Collection)', description: '', brand: 'Criterion', category: 'Video > DVD' }] }) };
    return base(url, o);
  };
  const m = await createDiscLookup({ fetchImpl: f, tmdbKey: 'k' })(ALIEN);
  assert.equal(m.title, 'Alien');
  assert.equal(m.runtime, 117);
  assert.equal(m.genre, 'Horror');
  assert.equal(m.tmdb_match, 'exact');
});

test('guessFormat: explicit words win, 4K implies Blu-ray, silence is a flagged guess', () => {
  assert.deepEqual(guessFormat('Alien Blu-ray'), { media: 'bluray', fourK: false, certain: true });
  assert.deepEqual(guessFormat('Dune 4K UHD'), { media: 'bluray', fourK: true, certain: true });
  assert.deepEqual(guessFormat('Heat DVD'), { media: 'dvd', fourK: false, certain: true });
  assert.equal(guessFormat('Heat').certain, false);
});

test('seriesName drops season wording for TV searches', () => {
  assert.equal(seriesName('Friends: The Complete First Season'), 'Friends');
  assert.equal(seriesName('Breaking Bad - Season 2'), 'Breaking Bad');
});

test('classifyMovie maps TMDB genres to shelves, with animation/documentary taking priority', () => {
  const a = classifyMovie({ title: 'Spider-Man', genres: ['Action', 'Adventure', 'Science Fiction'], year: '2002' });
  assert.equal(a.genre, 'Action & Adventure');
  assert.equal(a.subgenre, 'Science Fiction');
  assert.equal(a.needsReview, false);
  assert.ok(a.tags.includes('2000s') && a.tags.includes('science fiction'));
  assert.equal(classifyMovie({ genres: ['Family', 'Animation', 'Comedy'] }).genre, 'Animation');
  assert.equal(classifyMovie({ genres: ['Documentary', 'History'] }).genre, 'Documentary');
  assert.equal(classifyMovie({ genres: ['Sci-Fi & Fantasy'], tv: true }).genre, 'Science Fiction');
});

test('classifyMovie without TMDB genres guesses from text and always asks for review', () => {
  const g = classifyMovie({ title: 'Zombie Apocalypse', description: '' });
  assert.equal(g.genre, 'Horror');
  assert.equal(g.needsReview, true);
  const u = classifyMovie({ title: 'Untitled', description: '' });
  assert.equal(u.genre, 'Uncategorized');
  assert.equal(u.needsReview, true);
});

test('fetchUpcItemDb rejects non-disc products and surfaces rate limiting', async () => {
  const bread = async () => ({ ok: true, status: 200, json: async () => ({ code: 'OK', items: [{ title: 'BRAT & SAUSAGE ROLLS', category: 'Food' }] }) });
  assert.equal(await fetchUpcItemDb('0123456789012', bread), null);
  await assert.rejects(() => fetchUpcItemDb(ALIEN, fakeFetch({ upcStatus: 429 })), /rate limited/);
});

test('disc lookup: UPC title + TMDB gives director, runtime, genres, poster; format comes from the listing', async () => {
  const look = createDiscLookup({ fetchImpl: fakeFetch(), tmdbKey: 'k' });
  const m = await look(ALIEN);
  assert.equal(m.media, 'bluray');
  assert.equal(m.title, 'Alien');
  assert.deepEqual(m.authors, ['Ridley Scott']);
  assert.equal(m.runtime, 117);
  assert.equal(m.published, '1979');
  assert.equal(m.genre, 'Horror');
  assert.equal(m.cover_url, 'https://image.tmdb.org/t/p/w500/alien.jpg');
  assert.equal(m.needs_review, false);
  assert.equal(await look('0123456789012'), null);
});

test('disc lookup without a TMDB key still returns title/studio, flagged for review', async () => {
  const f = fakeFetch();
  const m = await createDiscLookup({ fetchImpl: f, tmdbKey: '' })(ALIEN);
  assert.equal(m.title, 'Alien');
  assert.equal(m.publisher, '20th Century Fox');
  assert.equal(m.needs_review, true);
  assert.ok(!f.calls.some((u) => u.includes('themoviedb')));
});

test('a TMDB outage degrades to the UPC data instead of failing', async () => {
  const m = await createDiscLookup({ fetchImpl: fakeFetch({ tmdb: false }), tmdbKey: 'k' })(ALIEN);
  assert.equal(m.title, 'Alien');
  assert.equal(m.provider_errors[0].provider, 'tmdb');
});

test('UPCitemdb being down is a LookupError, not "not found"', async () => {
  await assert.rejects(() => createDiscLookup({ fetchImpl: fakeFetch({ upcStatus: 429 }), tmdbKey: 'k' })(ALIEN), LookupError);
});

// ------------------------------------------------------------------ API

test('API: scanning a disc barcode routes to the disc lookup; a bad checksum is a 400', async () => {
  const t = await boot();
  try {
    const r = await t.req('GET', `/api/lookup/${ALIEN}`);
    assert.equal(r.status, 200);
    assert.equal(r.json.media, 'bluray');
    assert.equal(r.json.book.title, 'Alien');
    assert.equal(r.json.existing, null);
    assert.equal((await t.req('GET', '/api/lookup/0024543072448')).status, 400);
    assert.equal((await t.req('GET', '/api/lookup/9780441013593')).json.media, 'book'); // ISBNs still go to the book path
  } finally { await t.close(); }
});

test('API: add a disc, cover saved, duplicate refused, filtering and facets keep books and movies apart', async () => {
  const t = await boot();
  try {
    const add = await t.req('POST', '/api/books', { barcode: ALIEN, location: 'Basement' });
    assert.equal(add.status, 201);
    assert.equal(add.json.media, 'bluray');
    assert.equal(add.json.barcode, ALIEN);
    assert.equal(add.json.runtime, 117);
    assert.deepEqual(add.json.authors, ['Ridley Scott']);
    assert.equal(add.json.has_cover, true);
    assert.equal(add.json.genre, 'Horror');
    assert.equal(add.json.isbn13, null);

    assert.equal((await t.req('POST', '/api/books', { barcode: ALIEN })).status, 409);
    await t.req('POST', '/api/books', { title: 'A Novel', genre: 'Horror' });   // a book in a same-named genre

    const movies = await t.req('GET', '/api/books?media=movie');
    assert.deepEqual(movies.json.books.map((b) => b.title), ['Alien']);
    assert.equal((await t.req('GET', '/api/books?media=book')).json.total, 1);
    assert.equal((await t.req('GET', '/api/books?media=dvd')).json.total, 0);
    assert.equal((await t.req('GET', '/api/books?media=bluray')).json.total, 1);
    assert.equal((await t.req('GET', '/api/books?q=blu-ray')).json.total, 1);     // searchable by format
    assert.equal((await t.req('GET', `/api/books?q=${ALIEN}`)).json.total, 1);    // and by barcode
    assert.equal((await t.req('GET', '/api/books?q=ridley')).json.total, 1);      // and by director

    const f = (await t.req('GET', '/api/facets')).json;
    assert.deepEqual(f.media, { book: 1, dvd: 0, bluray: 1 });
    assert.equal(f.genres.find((g) => g.genre === 'Horror').count, 1);
    assert.equal(f.movieGenres.find((g) => g.genre === 'Horror').count, 1);
  } finally { await t.close(); }
});

test('API: manual disc entry, DVD<->Blu-ray switch allowed, disc<->book switch refused', async () => {
  const t = await boot();
  try {
    const add = await t.req('POST', '/api/books', { media: 'dvd', title: 'Home Video 1998', runtime: 45, genre: 'Documentary' });
    assert.equal(add.status, 201);
    assert.equal(add.json.media, 'dvd');
    assert.equal(add.json.genre_locked, true);
    const sw = await t.req('PATCH', `/api/books/${add.json.id}`, { media: 'bluray' });
    assert.equal(sw.json.media, 'bluray');
    assert.equal((await t.req('PATCH', `/api/books/${add.json.id}`, { media: 'book' })).status, 400);
    assert.equal((await t.req('POST', '/api/books', { media: 'vhs', title: 'x' })).status, 400);
    const noTitle = await t.req('POST', '/api/books', { media: 'dvd' });
    assert.equal(noTitle.status, 400);
  } finally { await t.close(); }
});

test('API: unknown barcode asks for a manual title; refresh fills blanks and reclassifies movies by movie rules', async () => {
  const t = await boot({ tmdbKey: '' });
  try {
    assert.equal((await t.req('POST', '/api/books', { barcode: '0123456789012' })).status, 404);
    const add = await t.req('POST', '/api/books', { barcode: ALIEN });
    assert.equal(add.json.needs_review, true);       // no TMDB key -> weak guess
    assert.equal(add.json.runtime, null);
    const t2 = await boot({ tmdbKey: 'k' });
    try {
      const a2 = await t2.req('POST', '/api/books', { barcode: ALIEN, title: 'My Alien' });
      assert.equal(a2.json.title, 'My Alien');       // the user's own title wins
      const rf = await t2.req('POST', `/api/books/${a2.json.id}/refresh`, {});
      assert.equal(rf.status, 200);
      assert.equal(rf.json.title, 'My Alien');
      assert.equal(rf.json.genre, 'Horror');
      const rc = await t2.req('POST', '/api/reclassify', {});
      assert.equal(rc.json.changed, 0);              // movies are re-run with the movie classifier, not the book one
    } finally { await t2.close(); }
  } finally { await t.close(); }
});

test('API: CSV export includes media, barcode and runtime', async () => {
  const t = await boot();
  try {
    await t.req('POST', '/api/books', { barcode: ALIEN });
    const csv = (await t.req('GET', '/api/export.csv')).text;
    assert.match(csv, /^media,isbn13,barcode,title/);
    assert.match(csv, /bluray,,0024543072447,Alien/);
  } finally { await t.close(); }
});

test('migration: a library created before DVD support opens, keeps its books, and gains the new columns', () => {
  const dir = mkdtempSync(join(tmpdir(), 'hl-mig-'));
  const path = join(dir, 'old.db');
  const old = new DatabaseSync(path);
  old.exec(`CREATE TABLE books (
    id INTEGER PRIMARY KEY, isbn13 TEXT UNIQUE, isbn10 TEXT, title TEXT NOT NULL, subtitle TEXT NOT NULL DEFAULT '',
    authors TEXT NOT NULL DEFAULT '[]', author_sort TEXT NOT NULL DEFAULT '', title_sort TEXT NOT NULL DEFAULT '',
    publisher TEXT NOT NULL DEFAULT '', published TEXT NOT NULL DEFAULT '', pages INTEGER, description TEXT NOT NULL DEFAULT '',
    cover_file TEXT, genre TEXT NOT NULL DEFAULT 'Uncategorized', subgenre TEXT NOT NULL DEFAULT '', genre_locked INTEGER NOT NULL DEFAULT 0,
    tags_locked INTEGER NOT NULL DEFAULT 0, needs_review INTEGER NOT NULL DEFAULT 0, confidence TEXT NOT NULL DEFAULT 'low',
    status TEXT NOT NULL DEFAULT 'unread' CHECK (status IN ('unread','reading','read')), location TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '', meta_raw TEXT NOT NULL DEFAULT '{}',
    added_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')));
    INSERT INTO books (isbn13, title) VALUES ('9780441013593', 'Dune');`);
  old.close();
  const db = new LibraryDb(path);
  const b = db.findByIsbn('9780441013593');
  assert.equal(b.title, 'Dune');
  assert.equal(b.media, 'book');
  assert.equal(b.barcode, null);
  db.insert({ media: 'dvd', barcode: ALIEN, title: 'Alien' });
  assert.equal(db.findByBarcode(ALIEN).media, 'dvd');
  assert.throws(() => db.insert({ media: 'dvd', barcode: ALIEN, title: 'Alien again' }), /UNIQUE/);
  db.close();
  new LibraryDb(path).close(); // opening twice (re-running the migration) must be harmless
});

// ------------------------------------------------- editor title search (TMDB)

test('TMDB search: returns films only, honours the year filter, and is off without a key', async () => {
  const t = await boot();
  try {
    const r = await t.req('GET', '/api/tmdb/search?q=alien');
    assert.equal(r.status, 200);
    assert.deepEqual(r.json.results, [{ id: 348, kind: 'movie', title: 'Alien', year: '1979', overview: 'A crew meets something.', poster: 'https://image.tmdb.org/t/p/w185/alien.jpg' }]);
    assert.equal((await t.req('GET', '/api/tmdb/search?q=alien&year=1979')).json.results.length, 1);
    assert.equal((await t.req('GET', '/api/tmdb/search?q=alien&year=1980')).json.results.length, 0);
    assert.equal((await t.req('GET', '/api/tmdb/search?q=')).status, 400);
    assert.equal((await t.req('GET', '/api/tmdb/search?q=alien&year=79')).status, 400);
    assert.equal((await t.req('GET', '/api/taxonomy')).json.tmdb, true);
  } finally { await t.close(); }
  const off = await boot({ tmdbKey: '' });
  try {
    assert.equal((await off.req('GET', '/api/tmdb/search?q=alien')).status, 501);
    assert.equal((await off.req('GET', '/api/taxonomy')).json.tmdb, false);
  } finally { await off.close(); }
});

test('TMDB details: enriched with the genre the classifier will pick; bad ids are rejected', async () => {
  const t = await boot();
  try {
    const d = (await t.req('GET', '/api/tmdb/movie/348')).json;
    assert.equal(d.title, 'Alien');
    assert.deepEqual(d.authors, ['Ridley Scott']);
    assert.equal(d.runtime, 117);
    assert.equal(d.genre, 'Horror');
    assert.deepEqual(d.tmdb, { id: 348, kind: 'movie' });
    assert.equal((await t.req('GET', '/api/tmdb/movie/0')).status, 400);
  } finally { await t.close(); }
});

test('applying a TMDB match to a disc that needed review fills metadata, cover and genre from TMDB itself', async () => {
  const t = await boot({ tmdbKey: '' });          // no key at add time: weak data, flagged for review
  try {
    const add = (await t.req('POST', '/api/books', { barcode: ALIEN })).json;
    assert.equal(add.needs_review, true);
    assert.equal(add.has_cover, false);
  } finally { await t.close(); }

  const t2 = await boot();
  try {
    const add = (await t2.req('POST', '/api/books', { media: 'bluray', title: 'Alein (typo)', notes: 'mine' })).json;
    assert.equal(add.genre, 'Uncategorized');
    const r = await t2.req('PATCH', `/api/books/${add.id}`, { title: 'Alien', authors: ['Ridley Scott'], tmdb: { id: 348, kind: 'movie' } });
    assert.equal(r.status, 200);
    assert.equal(r.json.title, 'Alien');
    assert.equal(r.json.genre, 'Horror');
    assert.equal(r.json.needs_review, false);
    assert.equal(r.json.has_cover, true);
    assert.equal(r.json.media, 'bluray');           // the format is the user's, untouched
    assert.equal(r.json.notes, 'mine');
    assert.deepEqual(r.json.meta_raw.tmdb, { id: 348, kind: 'movie' });
    assert.ok(r.json.tags.includes('horror'));
    // the match survives a later "re-run genre detection"
    assert.equal((await t2.req('POST', '/api/reclassify', {})).json.changed, 0);
  } finally { await t2.close(); }
});

test('applying a match keeps a genre the user chose, and refuses books and bad specs', async () => {
  const t = await boot();
  try {
    const add = (await t.req('POST', '/api/books', { media: 'dvd', title: 'Alien', genre: 'Comedy' })).json;
    const r = await t.req('PATCH', `/api/books/${add.id}`, { tmdb: { id: 348, kind: 'movie' } });
    assert.equal(r.json.genre, 'Comedy');
    assert.equal(r.json.genre_locked, true);
    assert.equal((await t.req('PATCH', `/api/books/${add.id}`, { tmdb: { id: 'x', kind: 'movie' } })).status, 400);
    assert.equal((await t.req('PATCH', `/api/books/${add.id}`, { tmdb: { id: 348, kind: 'person' } })).status, 400);
    const book = (await t.req('POST', '/api/books', { title: 'A Novel' })).json;
    assert.equal((await t.req('PATCH', `/api/books/${book.id}`, { tmdb: { id: 348, kind: 'movie' } })).status, 400);
    assert.equal((await t.req('POST', '/api/books', { title: 'Another Novel', tmdb: { id: 348, kind: 'movie' } })).status, 400);
  } finally { await t.close(); }
});

test('creating a disc with a TMDB match keeps the barcode lookup\'s format but takes TMDB genres and poster', async () => {
  const t = await boot({ tmdbKey: 'k' });
  try {
    const r = await t.req('POST', '/api/books', { barcode: ALIEN, media: 'dvd', tmdb: { id: 348, kind: 'movie' } });
    assert.equal(r.status, 201);
    assert.equal(r.json.media, 'dvd');                  // user's explicit format wins over the listing's "Blu-ray"
    assert.equal(r.json.genre, 'Horror');
    assert.deepEqual(r.json.meta_raw.sources.sort(), ['tmdb', 'upcitemdb']);
    const manual = await t.req('POST', '/api/books', { media: 'dvd', title: 'Manual Alien', tmdb: { id: 348, kind: 'movie' } });
    assert.equal(manual.json.genre, 'Horror');
    assert.equal(manual.json.has_cover, true);
  } finally { await t.close(); }
});
