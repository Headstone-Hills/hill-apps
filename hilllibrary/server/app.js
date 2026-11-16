// HTTP layer: JSON API + static files. Plain node:http, no framework.

import { createHmac, randomBytes, timingSafeEqual, createHash } from 'node:crypto';
import { readFile, writeFile, mkdir, unlink, stat } from 'node:fs/promises';
import { join, resolve, extname, sep } from 'node:path';
import { toIsbn13 } from './isbn.js';
import { isStatus, cleanTag } from './db.js';
import { classify, GENRE_NAMES, TAXONOMY, UNCATEGORIZED } from './genres.js';
import { downloadCover, enrich, LookupError } from './metadata.js';
import { toGtin13, isDisc } from './barcode.js';
import { classifyMovie, MOVIE_GENRES } from './moviegenres.js';
import { enrichDisc } from './discs.js';
import { combineMovies, JellyfinError } from './jellyfin.js';
import { scanCover } from './cover.js';

const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8',
  '.json': 'application/json', '.webmanifest': 'application/manifest+json', '.svg': 'image/svg+xml',
  '.png': 'image/png', '.jpg': 'image/jpeg', '.webp': 'image/webp', '.gif': 'image/gif', '.ico': 'image/x-icon',
};
const SESSION_DAYS = 90;
const MAX_BODY = 1024 * 1024;
const MAX_IMAGE = 8 * 1024 * 1024;

class HttpError extends Error {
  constructor(status, message, extra = {}) { super(message); this.status = status; this.extra = extra; }
}

/**
 * @param {object} o
 * @param {import('./db.js').LibraryDb} o.db
 * @param {(isbn13:string)=>Promise<object|null>} o.lookup
 * @param {(gtin13:string)=>Promise<object|null>} [o.lookupDisc]  DVD/Blu-ray barcode lookup
 * @param {{enabled:boolean, search?:Function, details?:Function}} [o.tmdb]  title search + details for the movie editor
 * @param {{enabled:boolean, publicUrl?:string, movies?:Function, image?:Function}} [o.jellyfin]  media-server connection
 * @param {object[]} [o.coverReaders]  cover text readers tried in order (see cover.js); empty disables cover scanning
 * @param {string} o.dataDir       where covers/ and .secret live
 * @param {string} o.publicDir
 * @param {string} [o.password]    empty/undefined = no auth (only safe on localhost)
 * @param {typeof fetch} [o.fetchImpl]  used for cover downloads
 * @param {string[]} [o.allowedOrigins]
 * @param {boolean} [o.trustProxy] take the client IP for login rate limiting from CF-Connecting-IP / X-Forwarded-For
 */
export function createApp({ db, lookup, lookupDisc = async () => null, tmdb = { enabled: false }, jellyfin = { enabled: false }, coverReaders = [], googleApiKey = '', dataDir, publicDir, password = '', fetchImpl = fetch, allowedOrigins = [], trustProxy = false }) {
  const coversDir = join(dataDir, 'covers');
  const root = resolve(publicDir);
  let secretPromise;
  const getSecret = () => (secretPromise ??= loadSecret(join(dataDir, '.secret')));
  const loginAttempts = new Map(); // ip -> {n, reset}

  // ------------------------------------------------------------- auth
  const pwDigest = (s) => createHash('sha256').update(String(s)).digest();
  const passwordOk = (s) => timingSafeEqual(pwDigest(s), pwDigest(password));

  async function sign(exp) {
    return `${exp}.${createHmac('sha256', await getSecret()).update(`session:${exp}:${pwDigest(password).toString('hex')}`).digest('hex')}`;
  }
  async function isAuthed(req) {
    if (!password) return true;
    const token = parseCookies(req.headers.cookie).hl_session;
    if (!token) return false;
    const [exp] = token.split('.');
    if (!(Number(exp) > Date.now())) return false;
    const expected = Buffer.from(await sign(exp));
    const given = Buffer.from(token);
    return expected.length === given.length && timingSafeEqual(expected, given);
  }
  // Only honour proxy headers when told to: otherwise anyone could spoof them to dodge the lockout.
  const clientIp = (req) => {
    if (trustProxy) {
      const h = req.headers['cf-connecting-ip'] || String(req.headers['x-forwarded-for'] ?? '').split(',')[0].trim();
      if (h) return h;
    }
    return req.socket.remoteAddress ?? '?';
  };
  const isHttps = (req) => !!req.socket.encrypted || req.headers['x-forwarded-proto'] === 'https';

  function originOk(req) {
    const origin = req.headers.origin;
    if (!origin) return true; // same-origin fetches from older browsers / curl send none
    try {
      const host = new URL(origin).host;
      return host === (req.headers['x-forwarded-host'] || req.headers.host) || allowedOrigins.includes(origin);
    } catch { return false; }
  }

  // ----------------------------------------------------------- helpers
  const send = (res, status, body, headers = {}) => {
    const isBuf = Buffer.isBuffer(body) || typeof body === 'string';
    res.writeHead(status, { ...(isBuf ? {} : { 'Content-Type': 'application/json; charset=utf-8' }), ...baseHeaders(), ...headers });
    res.end(isBuf ? body : JSON.stringify(body));
  };

  async function readJson(req) {
    if (!/^application\/json\b/i.test(req.headers['content-type'] ?? '')) throw new HttpError(415, 'Content-Type must be application/json');
    let size = 0;
    const chunks = [];
    for await (const c of req) {
      size += c.length;
      if (size > MAX_BODY) throw new HttpError(413, 'Request too large');
      chunks.push(c);
    }
    try { return JSON.parse(Buffer.concat(chunks).toString('utf8') || '{}'); }
    catch { throw new HttpError(400, 'Invalid JSON'); }
  }

  async function readImage(req) {
    const mime = (req.headers['content-type'] ?? '').split(';')[0].trim().toLowerCase();
    if (!['image/jpeg', 'image/png', 'image/webp'].includes(mime)) throw new HttpError(415, 'Send the photo as image/jpeg, image/png or image/webp.');
    let size = 0;
    const chunks = [];
    for await (const c of req) {
      size += c.length;
      if (size > MAX_IMAGE) throw new HttpError(413, 'That photo is too large (8 MB max).');
      chunks.push(c);
    }
    if (!size) throw new HttpError(400, 'Empty photo.');
    return { buffer: Buffer.concat(chunks), mime };
  }

  /** Validate and normalise user-editable fields. Throws HttpError(400). */
  function cleanFields(body, { requireTitle = false } = {}) {
    const out = {};
    const s = (k, max) => {
      if (body[k] === undefined) return;
      if (typeof body[k] !== 'string') throw new HttpError(400, `${k} must be a string`);
      out[k] = body[k].trim().slice(0, max);
    };
    s('title', 300); s('subtitle', 300); s('publisher', 200); s('published', 40);
    s('description', 10000); s('location', 100); s('notes', 5000);
    if (requireTitle && !out.title) throw new HttpError(400, 'title is required');
    if ('title' in out && !out.title) throw new HttpError(400, 'title cannot be empty');
    if (body.authors !== undefined) {
      if (!Array.isArray(body.authors) || body.authors.some((a) => typeof a !== 'string')) throw new HttpError(400, 'authors must be an array of strings');
      out.authors = body.authors.map((a) => a.trim()).filter(Boolean).slice(0, 20);
    }
    if (body.pages !== undefined) {
      if (body.pages === null || body.pages === '') out.pages = null;
      else {
        const n = Number(body.pages);
        if (!Number.isInteger(n) || n < 0 || n > 100000) throw new HttpError(400, 'pages must be a whole number');
        out.pages = n;
      }
    }
    if (body.runtime !== undefined) {
      if (body.runtime === null || body.runtime === '') out.runtime = null;
      else {
        const n = Number(body.runtime);
        if (!Number.isInteger(n) || n < 0 || n > 10000) throw new HttpError(400, 'runtime must be a whole number of minutes');
        out.runtime = n;
      }
    }
    if (body.media !== undefined) {
      if (body.media !== 'book' && !isDisc(body.media)) throw new HttpError(400, 'media must be book, dvd or bluray');
      out.media = body.media;
    }
    if (body.status !== undefined) {
      if (!isStatus(body.status)) throw new HttpError(400, 'status must be unread, reading or read');
      out.status = body.status;
    }
    if (body.tags !== undefined) {
      if (!Array.isArray(body.tags) || body.tags.some((t) => typeof t !== 'string')) throw new HttpError(400, 'tags must be an array of strings');
      out.tags = [...new Set(body.tags.map(cleanTag).filter(Boolean))].slice(0, 40);
    }
    if (body.genre !== undefined) {
      if (typeof body.genre !== 'string') throw new HttpError(400, 'genre must be a string');
      out.genre = body.genre.trim().slice(0, 60);
    }
    if (body.subgenre !== undefined) {
      if (typeof body.subgenre !== 'string') throw new HttpError(400, 'subgenre must be a string');
      out.subgenre = body.subgenre.trim().slice(0, 60);
    }
    return out;
  }

  async function saveCover(id, url) {
    if (!url) return null;
    const cover = await downloadCover(url, fetchImpl);
    if (!cover) return null;
    await mkdir(coversDir, { recursive: true });
    const file = `${id}.${cover.ext}`;
    await writeFile(join(coversDir, file), cover.buffer);
    return file;
  }

  /** Apply classifier output to a book unless the user locked it. */
  function reclassifyRow(book) {
    const c = isDisc(book.media)
      ? classifyMovie({ title: book.title, description: book.description, genres: book.meta_raw.genres, tv: book.meta_raw.tv, year: book.published, fourK: book.meta_raw.fourK })
      : classify({ title: book.title, subtitle: book.subtitle, description: book.description, ...book.meta_raw });
    const patch = { confidence: c.confidence };
    if (!book.genre_locked) Object.assign(patch, { genre: c.genre, subgenre: c.subgenre, needs_review: c.needsReview });
    if (!book.tags_locked) patch.tags = c.tags;
    return db.update(book.id, patch);
  }

  const lookupSafe = async (isbn13) => {
    try { return { meta: await lookup(isbn13), error: null }; }
    catch (e) {
      if (e instanceof LookupError) return { meta: null, error: e };
      throw e;
    }
  };

  const lookupDiscSafe = async (gtin13) => {
    try { return { meta: await lookupDisc(gtin13), error: null }; }
    catch (e) {
      if (e instanceof LookupError) return { meta: null, error: e };
      throw e;
    }
  };

  /** Validate a client-chosen {id, kind} and fetch the authoritative TMDB record ourselves (never trust client-sent metadata). */
  async function tmdbMatch(spec) {
    if (!tmdb.enabled) throw new HttpError(501, 'Movie search needs a TMDB API key. Set TMDB_API_KEY (see .env.example).');
    if (!spec || !Number.isInteger(spec.id) || spec.id <= 0 || (spec.kind !== 'movie' && spec.kind !== 'tv')) throw new HttpError(400, 'tmdb must be {id, kind: "movie"|"tv"}');
    let det;
    try { det = await tmdb.details(spec.kind, spec.id); }
    catch (e) { throw new HttpError(502, 'Could not reach TMDB.', { providers: [{ provider: 'tmdb', error: String(e?.message ?? e) }] }); }
    if (!det) throw new HttpError(404, 'TMDB has no such title.');
    return det;
  }

  /** A scanned/typed code is either a valid ISBN (book) or a valid UPC/EAN (DVD/Blu-ray). */
  function resolveCode(raw) {
    const isbn13 = toIsbn13(raw);
    if (isbn13) return { kind: 'book', code: isbn13 };
    const gtin = toGtin13(raw);
    return gtin ? { kind: 'disc', code: gtin } : null;
  }

  // ------------------------------------------------------------ routes
  const routes = [];
  const route = (method, pattern, handler, { auth = true } = {}) => routes.push({ method, re: new RegExp(`^${pattern}$`), handler, auth });

  route('GET', '/api/session', async (req, res) => send(res, 200, { authRequired: !!password, authed: await isAuthed(req) }), { auth: false });

  route('POST', '/api/login', async (req, res) => {
    if (!password) return send(res, 200, { ok: true });
    const ip = clientIp(req);
    const now = Date.now();
    const a = loginAttempts.get(ip);
    if (a && a.reset > now && a.n >= 10) throw new HttpError(429, 'Too many attempts. Try again in a few minutes.');
    const body = await readJson(req);
    if (typeof body.password !== 'string' || !passwordOk(body.password)) {
      loginAttempts.set(ip, a && a.reset > now ? { n: a.n + 1, reset: a.reset } : { n: 1, reset: now + 15 * 60 * 1000 });
      throw new HttpError(401, 'Wrong password');
    }
    loginAttempts.delete(ip);
    const exp = now + SESSION_DAYS * 86400 * 1000;
    send(res, 200, { ok: true }, {
      'Set-Cookie': `hl_session=${await sign(exp)}; HttpOnly; SameSite=Lax; Path=/; Max-Age=${SESSION_DAYS * 86400}${isHttps(req) ? '; Secure' : ''}`,
    });
  }, { auth: false });

  route('POST', '/api/logout', async (req, res) => send(res, 200, { ok: true }, {
    'Set-Cookie': `hl_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0${isHttps(req) ? '; Secure' : ''}`,
  }), { auth: false });

  route('GET', '/api/taxonomy', async (req, res) => send(res, 200, {
    genres: TAXONOMY.map((g) => ({ name: g.name, kind: g.kind, subgenres: [...new Set(g.rules.map((r) => r.sub).filter(Boolean))] })),
    uncategorized: UNCATEGORIZED,
    tmdb: !!tmdb.enabled,
    coverScan: coverReaders.length > 0,
    movieGenres: MOVIE_GENRES.map((name) => ({ name, subgenres: MOVIE_GENRES.filter((n) => n !== name) })),
  }));

  route('GET', '/api/tmdb/search', async (req, res) => {
    if (!tmdb.enabled) throw new HttpError(501, 'Movie search needs a TMDB API key. Set TMDB_API_KEY (see .env.example).');
    const q = (req.url.searchParams.get('q') ?? '').trim().slice(0, 200);
    const year = (req.url.searchParams.get('year') ?? '').trim();
    if (!q) throw new HttpError(400, 'Type a title to search for.');
    if (year && !/^\d{4}$/.test(year)) throw new HttpError(400, 'Year must be 4 digits.');
    try { send(res, 200, { results: await tmdb.search(q, year) }); }
    catch (e) { throw new HttpError(502, 'Could not reach TMDB.', { providers: [{ provider: 'tmdb', error: String(e?.message ?? e) }] }); }
  });

  route('GET', '/api/tmdb/(movie|tv)/(\\d+)', async (req, res, m) => {
    send(res, 200, await tmdbMatch({ kind: m[1], id: Number(m[2]) }));
  });

  // Movies tab: every disc you own plus every movie on Jellyfin, lined up as one list.
  route('GET', '/api/movies', async (req, res) => {
    const discs = db.list({ media: 'movie', limit: 2000 }).books;
    const info = { enabled: !!jellyfin.enabled, ok: false, error: '', count: 0, libraries: [], fetchedAt: null, publicUrl: jellyfin.publicUrl ?? '' };
    let server = [];
    if (jellyfin.enabled) {
      try {
        const r = await jellyfin.movies({ force: req.url.searchParams.get('refresh') === '1' });
        server = r.items; info.ok = true; info.count = server.length; info.libraries = r.libraries ?? []; info.fetchedAt = new Date(r.at).toISOString();
      } catch (e) {
        if (!(e instanceof JellyfinError)) throw e;
        info.error = e.message;   // still show the discs, with the reason the server side is missing
      }
    }
    send(res, 200, { items: combineMovies(discs, server), jellyfin: info });
  });

  route('GET', '/api/jellyfin/image/([0-9a-fA-F]{32})', async (req, res, m) => {
    if (!jellyfin.enabled) throw new HttpError(404, 'Jellyfin is not configured.');
    // Duplicate Jellyfin entries for one film may have a stale poster: try the main id, then `also` (id.tag,...) in order.
    const tries = [[m[1], req.url.searchParams.get('tag') ?? '']];
    for (const pair of (req.url.searchParams.get('also') ?? '').split(',').slice(0, 5)) {
      const [aid, atag = ''] = pair.split('.');
      if (/^[0-9a-fA-F]{32}$/.test(aid) && /^[0-9a-zA-Z]{0,64}$/.test(atag)) tries.push([aid, atag]);
    }
    let img = null;
    try { for (const [tid, ttag] of tries) { img = await jellyfin.image(tid, ttag); if (img) break; } }
    catch (e) { if (e instanceof JellyfinError) throw new HttpError(502, e.message); throw e; }
    if (!img) throw new HttpError(404, 'No poster');
    send(res, 200, img.buffer, { 'Content-Type': img.type, 'Cache-Control': 'private, max-age=86400' });
  });

  // Cover scanning: a photo of a cover with no barcode -> candidate books (the user picks; nothing is added here).
  route('POST', '/api/cover/scan', async (req, res) => {
    if (!coverReaders.length) throw new HttpError(501, 'Cover scanning is not set up on this server.');
    const { buffer, mime } = await readImage(req);
    let r;
    try { r = await scanCover(buffer, mime, { readers: coverReaders, fetchImpl, googleApiKey }); }
    catch (e) { if (e.noReader) throw new HttpError(503, e.message); throw e; }
    for (const c of r.candidates) c.existing = c.isbn13 ? (db.findByIsbn(c.isbn13)?.id ?? null) : null;
    if (r.isbn) r.isbnExisting = db.findByIsbn(r.isbn)?.id ?? null;
    send(res, 200, r);
  });

  route('GET', '/api/facets', async (req, res) => send(res, 200, db.facets()));

  route('GET', '/api/lookup/([0-9Xx-]+)', async (req, res, m) => {
    const r = resolveCode(m[1]);
    if (!r) throw new HttpError(400, 'That is not a valid ISBN or DVD/Blu-ray barcode (checksum failed).');
    if (r.kind === 'book') {
      const existing = db.findByIsbn(r.code);
      const { meta, error } = await lookupSafe(r.code);
      if (error) throw new HttpError(502, 'Could not reach the book databases. Check the server\'s internet connection.', { providers: error.providers, existing });
      return send(res, 200, { isbn13: r.code, code: r.code, media: 'book', found: !!meta, book: meta, existing });
    }
    const existing = db.findByBarcode(r.code);
    const { meta, error } = await lookupDiscSafe(r.code);
    if (error) throw new HttpError(502, 'Could not look up that barcode. Check the server\'s internet connection (the free barcode service also allows only about 100 lookups a day).', { providers: error.providers, existing });
    send(res, 200, { barcode: r.code, code: r.code, media: meta?.media ?? 'dvd', found: !!meta, book: meta, existing });
  });

  route('GET', '/api/books', async (req, res) => {
    const q = req.url.searchParams;
    send(res, 200, db.list({
      q: q.get('q') ?? '', genre: q.get('genre') ?? '', subgenre: q.get('subgenre') ?? '', tag: q.get('tag') ?? '',
      media: q.get('media') ?? '', status: q.get('status') ?? '', location: q.get('location') ?? '', review: q.get('review') === '1',
      sort: q.get('sort') || undefined, limit: q.get('limit') ?? 500, offset: q.get('offset') ?? 0,
    }));
  });

  route('GET', '/api/books/(\\d+)', async (req, res, m) => {
    const b = db.get(Number(m[1]));
    if (!b) throw new HttpError(404, 'Book not found');
    send(res, 200, b);
  });

  route('POST', '/api/books', async (req, res) => {
    const body = await readJson(req);
    const isbn13 = body.isbn ? toIsbn13(body.isbn) : null;
    if (body.isbn && !isbn13) throw new HttpError(400, 'That is not a valid ISBN (checksum failed).');
    const gtin13 = body.barcode ? toGtin13(body.barcode) : null;
    if (body.barcode && !gtin13) throw new HttpError(400, 'That is not a valid barcode (checksum failed).');
    const fields = cleanFields(body);
    const disc = !isbn13 && (!!gtin13 || isDisc(fields.media));
    if (disc && !isDisc(fields.media) && !gtin13) throw new HttpError(400, 'media must be dvd or bluray for a disc');
    if (isbn13) {
      const dup = db.findByIsbn(isbn13);
      if (dup) throw new HttpError(409, 'Already in your library', { existing: dup });
    }
    if (gtin13) {
      const dup = db.findByBarcode(gtin13);
      if (dup) throw new HttpError(409, 'Already in your library', { existing: dup });
    }

    let meta = null;
    if (isbn13) {
      const r = await lookupSafe(isbn13);
      meta = r.meta;
      if (!meta && !fields.title) {
        if (r.error) throw new HttpError(502, 'Could not reach the book databases and no title was given.', { providers: r.error.providers });
        throw new HttpError(404, 'No metadata found for that ISBN. Add the title manually.', { notFound: true });
      }
    } else if (gtin13) {
      const r = await lookupDiscSafe(gtin13);
      meta = r.meta;
      if (!meta && !fields.title) {
        if (r.error) throw new HttpError(502, 'Could not look up that barcode and no title was given.', { providers: r.error.providers });
        throw new HttpError(404, 'No movie found for that barcode. Add the title manually.', { notFound: true });
      }
    }
    let chosen = null;
    if (body.tmdb !== undefined) {
      if (!disc) throw new HttpError(400, 'A TMDB match only applies to DVDs/Blu-rays.');
      chosen = await tmdbMatch(body.tmdb);
      // The user's pick wins over the barcode lookup's guess, except the disc format and barcode, which TMDB doesn't know.
      const { media: _m, format_guess: _f, barcode: _b, sources, ...rest } = chosen;
      meta = { ...meta, ...rest, sources: [...new Set([...(meta?.sources ?? []), ...sources])] };
    }
    if (!meta && !fields.title) throw new HttpError(400, 'title is required');

    // Start from provider data, overlay the user's own fields, classify if the user didn't choose a genre.
    const media = disc ? (fields.media ?? meta?.media ?? 'dvd') : 'book';
    const base = meta ?? { subjects: [], categories: [], dewey: [], genres: [] };
    const merged = {
      media, isbn13, isbn10: meta?.isbn10 ?? null, barcode: gtin13, runtime: meta?.runtime ?? null,
      title: base.title, subtitle: base.subtitle, authors: base.authors, publisher: base.publisher,
      published: base.published, pages: base.pages, description: base.description,
      ...fields, media,
    };
    const c = disc
      ? enrichDisc({ ...merged, genres: base.genres, tv: base.tv, fourK: base.fourK, tmdb_match: base.tmdb_match ?? 'none' })
      : enrich({ ...merged, subjects: base.subjects, categories: base.categories, dewey: base.dewey });
    const userGenre = fields.genre && fields.genre !== 'auto';
    const row = {
      ...merged,
      genre: userGenre ? fields.genre : c.genre,
      subgenre: userGenre ? fields.subgenre ?? '' : fields.subgenre ?? c.subgenre,
      genre_locked: userGenre,
      tags_locked: fields.tags !== undefined,
      tags: fields.tags ?? c.tags,
      needs_review: userGenre ? false : c.needs_review,
      confidence: c.confidence,
      meta_raw: disc
        ? { genres: base.genres ?? [], tv: !!base.tv, fourK: !!base.fourK, sources: base.sources ?? [], ...(base.tmdb ? { tmdb: base.tmdb } : {}) }
        : { subjects: base.subjects, categories: base.categories, dewey: base.dewey, sources: base.sources ?? [] },
    };
    let book = db.insert(row);
    const file = await saveCover(book.id, base.cover_url);
    if (file) book = db.update(book.id, { cover_file: file });
    send(res, 201, book);
  });

  route('PATCH', '/api/books/(\\d+)', async (req, res, m) => {
    const id = Number(m[1]);
    const cur = db.get(id);
    if (!cur) throw new HttpError(404, 'Book not found');
    const body = await readJson(req);
    const patch = cleanFields(body);
    if ('media' in patch && patch.media !== cur.media && (patch.media === 'book' || cur.media === 'book')) {
      throw new HttpError(400, 'A book can\'t be changed into a DVD/Blu-ray (or the other way round). Only DVD <-> Blu-ray.');
    }
    let reclass = false;
    if ('genre' in patch) {
      if (patch.genre === '' || patch.genre === 'auto') {      // hand control back to the classifier
        delete patch.genre; delete patch.subgenre;
        patch.genre_locked = false;
        reclass = true;
      } else {
        patch.genre_locked = true;
        patch.needs_review = false;
        patch.subgenre ??= '';                                 // an old subgenre belongs to the old genre
      }
    }
    if (body.tmdb !== undefined) {
      if (!isDisc(cur.media)) throw new HttpError(400, 'A TMDB match only applies to DVDs/Blu-rays.');
      const det = await tmdbMatch(body.tmdb);
      patch.meta_raw = {
        ...cur.meta_raw, genres: det.genres, tv: det.tv, tmdb: det.tmdb,
        sources: [...new Set([...(cur.meta_raw.sources ?? []), 'tmdb'])],
      };
      const file = await saveCover(id, det.cover_url);   // the user chose this film, so its poster replaces the old cover
      if (file) {
        patch.cover_file = file;
        if (cur.cover_file && cur.cover_file !== file) await unlink(join(coversDir, cur.cover_file)).catch(() => {});
      }
      if (!('genre' in patch)) reclass = true;           // re-shelve from the new genres (a genre the user locked is kept)
    }
    if ('tags' in patch) patch.tags_locked = true;
    if (body.autoTags === true) { delete patch.tags; patch.tags_locked = false; reclass = true; }
    let book = db.update(id, patch);
    if (reclass) book = reclassifyRow(book);
    send(res, 200, book);
  });

  route('DELETE', '/api/books/(\\d+)', async (req, res, m) => {
    const id = Number(m[1]);
    const b = db.get(id);
    if (!b) throw new HttpError(404, 'Book not found');
    db.delete(id);
    if (b.cover_file) await unlink(join(coversDir, b.cover_file)).catch(() => {});
    send(res, 200, { ok: true });
  });

  route('POST', '/api/books/(\\d+)/refresh', async (req, res, m) => {
    const book = db.get(Number(m[1]));
    if (!book) throw new HttpError(404, 'Book not found');
    const code = book.isbn13 || book.barcode;
    if (!code) throw new HttpError(400, 'This item has no ISBN or barcode to look up.');
    const isDiscItem = !book.isbn13;
    const { meta, error } = isDiscItem ? await lookupDiscSafe(code) : await lookupSafe(code);
    if (error) throw new HttpError(502, 'Could not reach the metadata databases.', { providers: error.providers });
    if (!meta) throw new HttpError(404, 'No metadata found for this code.', { notFound: true });
    // Fill blanks only: never overwrite what the user may have edited.
    const patch = { meta_raw: isDiscItem
      ? { genres: meta.genres, tv: meta.tv, fourK: meta.fourK, sources: meta.sources, ...(meta.tmdb ? { tmdb: meta.tmdb } : {}) }
      : { subjects: meta.subjects, categories: meta.categories, dewey: meta.dewey, sources: meta.sources } };
    for (const k of ['subtitle', 'publisher', 'published', 'description']) if (!book[k] && meta[k]) patch[k] = meta[k];
    if (!book.pages && meta.pages) patch.pages = meta.pages;
    if (!book.runtime && meta.runtime) patch.runtime = meta.runtime;
    if (!book.authors.length && meta.authors.length) patch.authors = meta.authors;
    if (!book.has_cover) { const f = await saveCover(book.id, meta.cover_url); if (f) patch.cover_file = f; }
    send(res, 200, reclassifyRow(db.update(book.id, patch)));
  });

  route('POST', '/api/reclassify', async (req, res) => {
    let changed = 0;
    for (const b of db.all()) {
      const after = reclassifyRow(b);
      if (after.genre !== b.genre || after.subgenre !== b.subgenre || after.tags.join() !== b.tags.join()) changed++;
    }
    send(res, 200, { ok: true, changed, total: db.count() });
  });

  route('GET', '/api/export\\.(json|csv)', async (req, res, m) => {
    const books = db.all().map(({ meta_raw, cover_file, has_cover, ...b }) => b);
    const stamp = new Date().toISOString().slice(0, 10);
    if (m[1] === 'json') {
      return send(res, 200, JSON.stringify(books, null, 2), {
        'Content-Type': 'application/json', 'Content-Disposition': `attachment; filename="library-${stamp}.json"`,
      });
    }
    const cols = ['media', 'isbn13', 'barcode', 'title', 'subtitle', 'authors', 'publisher', 'published', 'pages', 'runtime', 'genre', 'subgenre', 'tags', 'status', 'location', 'notes', 'added_at'];
    const rows = [cols.join(',')].concat(books.map((b) => cols.map((c) => csv(Array.isArray(b[c]) ? b[c].join('; ') : b[c])).join(',')));
    send(res, 200, '﻿' + rows.join('\r\n') + '\r\n', {
      'Content-Type': 'text/csv; charset=utf-8', 'Content-Disposition': `attachment; filename="library-${stamp}.csv"`,
    });
  });

  route('GET', '/covers/(\\d+\\.(?:jpg|png|webp|gif))', async (req, res, m) => {
    const data = await readFile(join(coversDir, m[1])).catch(() => null);
    if (!data) throw new HttpError(404, 'No cover');
    send(res, 200, data, { 'Content-Type': MIME[extname(m[1])], 'Cache-Control': 'private, max-age=86400' });
  });

  // ------------------------------------------------------------ dispatch
  async function serveStatic(req, res, pathname) {
    const rel = pathname === '/' ? '/index.html' : pathname;
    const file = resolve(root, '.' + decodeURIComponent(rel));
    if (file !== root && !file.startsWith(root + sep)) throw new HttpError(404, 'Not found');
    const st = await stat(file).catch(() => null);
    if (!st?.isFile()) throw new HttpError(404, 'Not found');
    const type = MIME[extname(file)] ?? 'application/octet-stream';
    const cache = rel.startsWith('/vendor/') ? 'public, max-age=86400' : 'no-cache';
    send(res, 200, await readFile(file), { 'Content-Type': type, 'Cache-Control': cache });
  }

  return async function handler(req, res) {
    try {
      const url = new URL(req.url, 'http://localhost');
      req.url = url; // handlers read query params from req.url.searchParams
      const method = req.method === 'HEAD' ? 'GET' : req.method;

      if (method !== 'GET' && !originOk(req)) throw new HttpError(403, 'Cross-origin request blocked');

      for (const r of routes) {
        if (r.method !== method) continue;
        const m = url.pathname.match(r.re);
        if (!m) continue;
        if (r.auth && !(await isAuthed(req))) throw new HttpError(401, 'Not signed in', { authRequired: true });
        return await r.handler(req, res, m);
      }
      if (url.pathname.startsWith('/api/')) throw new HttpError(404, 'Unknown API route');
      if (method !== 'GET') throw new HttpError(405, 'Method not allowed');
      return await serveStatic(req, res, url.pathname);
    } catch (e) {
      if (e instanceof HttpError) return send(res, e.status, { error: e.message, ...e.extra });
      if (e?.code === 'ERR_INVALID_URL' || e instanceof URIError) return send(res, 400, { error: 'Bad request' });
      console.error('Unhandled error:', e);
      return send(res, 500, { error: 'Something went wrong on the server.' });
    }
  };
}

// ---------------------------------------------------------------- utilities

function baseHeaders() {
  return {
    'X-Content-Type-Options': 'nosniff',
    'Referrer-Policy': 'no-referrer',
    'X-Frame-Options': 'DENY',
    // Everything is self-hosted: covers are cached locally, the scanner library is vendored.
    'Content-Security-Policy': "default-src 'self'; img-src 'self' data: blob: https://covers.openlibrary.org https://*.archive.org https://books.google.com https://*.googleusercontent.com https://image.tmdb.org; media-src 'self' blob:; style-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
    'Permissions-Policy': 'camera=(self), microphone=(), geolocation=()',
  };
}

function parseCookies(header = '') {
  const out = {};
  for (const part of header.split(';')) {
    const i = part.indexOf('=');
    if (i > 0) out[part.slice(0, i).trim()] = part.slice(i + 1).trim();
  }
  return out;
}

async function loadSecret(path) {
  try { return await readFile(path); }
  catch {
    const s = randomBytes(32);
    await mkdir(resolve(path, '..'), { recursive: true });
    await writeFile(path, s, { mode: 0o600 });
    return s;
  }
}

/** CSV cell with quoting, plus a leading-quote guard against spreadsheet formula injection. */
function csv(v) {
  let s = v == null ? '' : String(v);
  if (/^[=+\-@\t\r]/.test(s)) s = "'" + s;
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

export { GENRE_NAMES };
