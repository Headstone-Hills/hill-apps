// HTTP layer: JSON API + static files. Plain node:http, no framework.

import { createHmac, randomBytes, timingSafeEqual, createHash } from 'node:crypto';
import { readFile, writeFile, mkdir, unlink, stat } from 'node:fs/promises';
import { join, resolve, extname, sep } from 'node:path';
import { toIsbn13 } from './isbn.js';
import { isStatus, cleanTag } from './db.js';
import { classify, GENRE_NAMES, TAXONOMY, UNCATEGORIZED } from './genres.js';
import { downloadCover, enrich, LookupError } from './metadata.js';

const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8',
  '.json': 'application/json', '.webmanifest': 'application/manifest+json', '.svg': 'image/svg+xml',
  '.png': 'image/png', '.jpg': 'image/jpeg', '.webp': 'image/webp', '.gif': 'image/gif', '.ico': 'image/x-icon',
};
const SESSION_DAYS = 90;
const MAX_BODY = 1024 * 1024;

class HttpError extends Error {
  constructor(status, message, extra = {}) { super(message); this.status = status; this.extra = extra; }
}

/**
 * @param {object} o
 * @param {import('./db.js').LibraryDb} o.db
 * @param {(isbn13:string)=>Promise<object|null>} o.lookup
 * @param {string} o.dataDir       where covers/ and .secret live
 * @param {string} o.publicDir
 * @param {string} [o.password]    empty/undefined = no auth (only safe on localhost)
 * @param {typeof fetch} [o.fetchImpl]  used for cover downloads
 * @param {string[]} [o.allowedOrigins]
 */
export function createApp({ db, lookup, dataDir, publicDir, password = '', fetchImpl = fetch, allowedOrigins = [] }) {
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
    const c = classify({ title: book.title, subtitle: book.subtitle, description: book.description, ...book.meta_raw });
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

  // ------------------------------------------------------------ routes
  const routes = [];
  const route = (method, pattern, handler, { auth = true } = {}) => routes.push({ method, re: new RegExp(`^${pattern}$`), handler, auth });

  route('GET', '/api/session', async (req, res) => send(res, 200, { authRequired: !!password, authed: await isAuthed(req) }), { auth: false });

  route('POST', '/api/login', async (req, res) => {
    if (!password) return send(res, 200, { ok: true });
    const ip = req.socket.remoteAddress ?? '?';
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
  }));

  route('GET', '/api/facets', async (req, res) => send(res, 200, db.facets()));

  route('GET', '/api/lookup/([0-9Xx-]+)', async (req, res, m) => {
    const isbn13 = toIsbn13(m[1]);
    if (!isbn13) throw new HttpError(400, 'That is not a valid ISBN (checksum failed).');
    const existing = db.findByIsbn(isbn13);
    const { meta, error } = await lookupSafe(isbn13);
    if (error) throw new HttpError(502, 'Could not reach the book databases. Check the server\'s internet connection.', { providers: error.providers, existing });
    send(res, 200, { isbn13, found: !!meta, book: meta, existing });
  });

  route('GET', '/api/books', async (req, res) => {
    const q = req.url.searchParams;
    send(res, 200, db.list({
      q: q.get('q') ?? '', genre: q.get('genre') ?? '', subgenre: q.get('subgenre') ?? '', tag: q.get('tag') ?? '',
      status: q.get('status') ?? '', location: q.get('location') ?? '', review: q.get('review') === '1',
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
    if (isbn13) {
      const dup = db.findByIsbn(isbn13);
      if (dup) throw new HttpError(409, 'Already in your library', { existing: dup });
    }
    const fields = cleanFields(body);
    let meta = null;
    if (isbn13) {
      const r = await lookupSafe(isbn13);
      meta = r.meta;
      if (!meta && !fields.title) {
        if (r.error) throw new HttpError(502, 'Could not reach the book databases and no title was given.', { providers: r.error.providers });
        throw new HttpError(404, 'No metadata found for that ISBN. Add the title manually.', { notFound: true });
      }
    }
    if (!meta && !fields.title) throw new HttpError(400, 'title is required');

    // Start from provider data, overlay the user's own fields, classify if the user didn't choose a genre.
    const base = meta ?? { subjects: [], categories: [], dewey: [] };
    const merged = {
      isbn13, isbn10: meta?.isbn10 ?? null,
      title: base.title, subtitle: base.subtitle, authors: base.authors, publisher: base.publisher,
      published: base.published, pages: base.pages, description: base.description,
      ...fields,
    };
    const c = enrich({ ...merged, subjects: base.subjects, categories: base.categories, dewey: base.dewey });
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
      meta_raw: { subjects: base.subjects, categories: base.categories, dewey: base.dewey, sources: base.sources ?? [] },
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
    if (!book.isbn13) throw new HttpError(400, 'This book has no ISBN to look up.');
    const { meta, error } = await lookupSafe(book.isbn13);
    if (error) throw new HttpError(502, 'Could not reach the book databases.', { providers: error.providers });
    if (!meta) throw new HttpError(404, 'No metadata found for this ISBN.', { notFound: true });
    // Fill blanks only: never overwrite what the user may have edited.
    const patch = { meta_raw: { subjects: meta.subjects, categories: meta.categories, dewey: meta.dewey, sources: meta.sources } };
    for (const k of ['subtitle', 'publisher', 'published', 'description']) if (!book[k] && meta[k]) patch[k] = meta[k];
    if (!book.pages && meta.pages) patch.pages = meta.pages;
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
    const cols = ['isbn13', 'title', 'subtitle', 'authors', 'publisher', 'published', 'pages', 'genre', 'subgenre', 'tags', 'status', 'location', 'notes', 'added_at'];
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
    'Content-Security-Policy': "default-src 'self'; img-src 'self' data: blob: https://covers.openlibrary.org https://*.archive.org https://books.google.com https://*.googleusercontent.com; media-src 'self' blob:; style-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
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
