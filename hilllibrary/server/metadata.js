// Book metadata lookup by ISBN.
//
// Two free providers are queried in parallel and merged field by field:
//   * Open Library  - best for subjects, Dewey numbers, and covers
//   * Google Books  - best for descriptions and category paths
//
// ASSUMPTION NOTICE: the response shapes below are written from the providers'
// public documentation as I know it; they could not be verified against the live
// APIs from the build sandbox. Every field is therefore shape-checked and optional,
// and a malformed response from one provider degrades to "no data from that
// provider" rather than an exception.
//
//   Open Library Books API:  https://openlibrary.org/dev/docs/api/books
//   Open Library Covers API: https://openlibrary.org/dev/docs/api/covers
//   Google Books API:        https://developers.google.com/books/docs/v1/using

import { toIsbn10, toIsbn13 } from './isbn.js';
import { classify } from './genres.js';
import { stripHtml } from './text.js';

const UA = 'HillLibrary/1.0 (personal home library app)';
const TIMEOUT_MS = 6000;

export class LookupError extends Error {
  constructor(message, providers) { super(message); this.name = 'LookupError'; this.providers = providers; }
}

const str = (v) => (typeof v === 'string' ? v.trim() : '');
const strList = (v) => (Array.isArray(v) ? v.map((x) => (typeof x === 'string' ? x : x?.name)).map(str).filter(Boolean) : []);

/** Retry-After is either seconds or an HTTP date. Capped at 10 minutes. */
function parseRetryAfter(v) {
  if (!v) return null;
  const secs = Number(v);
  if (Number.isFinite(secs)) return Math.min(Math.max(secs, 0), 600) * 1000;
  const at = Date.parse(v);
  return Number.isNaN(at) ? null : Math.min(Math.max(at - Date.now(), 0), 600_000);
}

async function getJson(url, fetchImpl) {
  const res = await fetchImpl(url, {
    headers: { 'User-Agent': UA, Accept: 'application/json' },
    signal: AbortSignal.timeout(TIMEOUT_MS),
  });
  if (!res.ok) {
    const err = new Error(`HTTP ${res.status}`);
    err.status = res.status;
    err.retryAfterMs = parseRetryAfter(res.headers?.get?.('retry-after'));
    throw err;
  }
  return res.json();
}

export async function fetchOpenLibrary(isbn13, fetchImpl = fetch) {
  const key = `ISBN:${isbn13}`;
  const data = (await getJson(`https://openlibrary.org/api/books?bibkeys=${key}&format=json&jscmd=data`, fetchImpl))?.[key];
  if (!data || typeof data !== 'object') return null;
  const cover = data.cover && typeof data.cover === 'object' ? data.cover.large || data.cover.medium || data.cover.small : '';
  const dewey = data.classifications?.dewey_decimal_class;
  return {
    title: str(data.title),
    subtitle: str(data.subtitle),
    authors: strList(data.authors),
    publisher: strList(data.publishers)[0] ?? '',
    published: str(data.publish_date),
    pages: Number.isInteger(data.number_of_pages) ? data.number_of_pages : null,
    subjects: strList(data.subjects),
    dewey: Array.isArray(dewey) ? dewey.map(str).filter(Boolean) : [],
    cover: str(cover),
    description: '', // not provided by this endpoint; Google Books fills it in
  };
}

export async function fetchGoogleBooks(isbn13, fetchImpl = fetch, apiKey = '') {
  const url = `https://www.googleapis.com/books/v1/volumes?q=isbn:${isbn13}&maxResults=1${apiKey ? `&key=${encodeURIComponent(apiKey)}` : ''}`;
  const v = (await getJson(url, fetchImpl))?.items?.[0]?.volumeInfo;
  if (!v || typeof v !== 'object') return null;
  const thumb = str(v.imageLinks?.thumbnail || v.imageLinks?.smallThumbnail)
    .replace(/^http:/, 'https:').replace(/&edge=curl/, '');
  return {
    title: str(v.title),
    subtitle: str(v.subtitle),
    authors: strList(v.authors),
    publisher: str(v.publisher),
    published: str(v.publishedDate),
    pages: Number.isInteger(v.pageCount) && v.pageCount > 0 ? v.pageCount : null,
    categories: strList(v.categories),
    description: stripHtml(str(v.description)),
    cover: thumb,
  };
}

/** Field-by-field merge; Open Library wins for bibliographic data, Google for description. */
export function mergeProviders(isbn13, ol, gb) {
  if (!ol && !gb) return null;
  const pick = (...vals) => vals.find((v) => v) ?? '';
  const authors = (ol?.authors?.length ?? 0) >= (gb?.authors?.length ?? 0) ? ol?.authors ?? [] : gb?.authors ?? [];
  const title = pick(ol?.title, gb?.title);
  const merged = {
    isbn13,
    isbn10: toIsbn10(isbn13),
    title,
    subtitle: pick(ol?.subtitle, gb?.subtitle),
    authors,
    publisher: pick(ol?.publisher, gb?.publisher),
    published: pick(ol?.published, gb?.published),
    pages: ol?.pages ?? gb?.pages ?? null,
    description: pick(gb?.description, ol?.description),
    cover_url: pick(ol?.cover, gb?.cover),
    subjects: ol?.subjects ?? [],
    categories: gb?.categories ?? [],
    dewey: ol?.dewey ?? [],
    sources: [ol && 'openlibrary', gb && 'googlebooks'].filter(Boolean),
    incomplete: !title, // a record with no title is "partial": useful to pre-fill a manual add, not a match
  };
  if (!title) {
    // Keep whatever else the providers knew (publisher, subjects, ...). If there is truly nothing, it's not found.
    const hasAnything = merged.authors.length || merged.publisher || merged.published || merged.pages || merged.description
      || merged.cover_url || merged.subjects.length || merged.categories.length || merged.dewey.length || merged.subtitle;
    if (!hasAnything) return null;
  }
  return merged;
}

/** Attach classifier output (genre, subgenre, tags, ...) to a merged metadata record. */
export function enrich(meta) {
  const c = classify(meta);
  return { ...meta, genre: c.genre, subgenre: c.subgenre, tags: c.tags, confidence: c.confidence, needs_review: c.needsReview };
}

const PROVIDERS = {
  openlibrary: (isbn, fetchImpl) => fetchOpenLibrary(isbn, fetchImpl),
  googlebooks: (isbn, fetchImpl, apiKey) => fetchGoogleBooks(isbn, fetchImpl, apiKey),
};

/**
 * Remembers which providers are failing so we stop asking them for a while. Anonymous book APIs throttle
 * rapid use (HTTP 429); retrying on every scan makes each scan wait for the failure again, and keeps the
 * throttling going. A 429 pauses the provider at once (honouring Retry-After); other failures pause it after
 * two in a row, with exponential backoff. One success clears it. After the pause a single request probes again.
 */
export function createBreaker({ now = Date.now, failuresToOpen = 2, baseMs = 15_000, maxMs = 5 * 60_000, throttleMs = 60_000 } = {}) {
  const state = new Map();
  return {
    isOpen: (name) => (state.get(name)?.until ?? 0) > now(),
    remainingMs: (name) => Math.max(0, (state.get(name)?.until ?? 0) - now()),
    success: (name) => { state.delete(name); },
    failure(name, err) {
      const s = state.get(name) ?? { fails: 0, until: 0 };
      s.fails++;
      if (err?.status === 429) s.until = now() + (err.retryAfterMs ?? throttleMs);
      else if (s.fails >= failuresToOpen) s.until = now() + Math.min(maxMs, baseMs * 2 ** (s.fails - failuresToOpen));
      state.set(name, s);
    },
  };
}

/**
 * Run one provider call under the circuit breaker. Never rejects: resolves {name, value} or {name, error}.
 * (Lookup and search share one breaker, so a provider that is throttling us is skipped by both.)
 */
async function guarded(breaker, name, fn) {
  if (breaker.isOpen(name)) return { name, error: `paused for ${Math.ceil(breaker.remainingMs(name) / 1000)}s after repeated failures` };
  try {
    const value = await fn();
    breaker.success(name); // "no results" is still a healthy answer
    return { name, value };
  } catch (e) {
    breaker.failure(name, e);
    return { name, error: String(e?.message ?? e) };
  }
}

/**
 * Look an ISBN up. Resolves to enriched metadata, or null when no provider knows
 * anything about the book. A record with `incomplete: true` has no title but carries
 * whatever else was found (publisher, subjects, ...). Throws LookupError only when providers
 * *failed* (network/HTTP/paused), so the UI can tell "not found" from "couldn't reach the internet".
 *
 * Speed: both providers are asked at once, but we answer as soon as one has a complete record
 * (a title), waiting at most `graceMs` for the other to add detail. If the other lands later it
 * upgrades the cached entry, so the add that follows a scan still gets the richer record.
 */
export function createLookup({
  fetchImpl = fetch, googleApiKey = '', ttlMs = 10 * 60 * 1000, graceMs = 800, maxCache = 500, now = Date.now,
  breaker = createBreaker({ now }),
} = {}) {
  const cache = new Map(); // isbn13 -> {at, value}; Map keeps insertion order, so the first key is the oldest
  const inflight = new Map(); // isbn13 -> Promise, so a double read of one barcode costs one lookup
  const names = Object.keys(PROVIDERS);

  const remember = (isbn13, value) => {
    cache.delete(isbn13);
    cache.set(isbn13, { at: now(), value });
    while (cache.size > maxCache) cache.delete(cache.keys().next().value);
  };

  const ask = (name, isbn13) => guarded(breaker, name, () => PROVIDERS[name](isbn13, fetchImpl, googleApiKey));

  const build = (isbn13, results) => ({
    merged: mergeProviders(isbn13, results.openlibrary?.value ?? null, results.googlebooks?.value ?? null),
    errors: Object.values(results).filter((r) => r.error).map((r) => ({ provider: r.name, error: r.error })),
  });

  async function lookupUncached(isbn13) {
    const results = {};
    const tasks = names.map((n) => ask(n, isbn13).then((r) => { results[n] = r; }));
    const everyone = Promise.all(tasks);
    await Promise.race(tasks); // tasks never reject: ask() catches

    if (Object.values(results).some((r) => r.value?.title)) {
      let timer;
      await Promise.race([everyone, new Promise((res) => { timer = setTimeout(res, graceMs); timer.unref?.(); })]);
      clearTimeout(timer);
    } else {
      await everyone; // nothing usable yet: we have to hear from the rest
    }

    const { merged, errors } = build(isbn13, results);
    if (!merged) {
      if (errors.length === names.length) throw new LookupError('Could not reach any metadata provider', errors);
      return null; // at least one provider answered and none knows this ISBN
    }
    const value = { ...enrich(merged), provider_errors: errors };
    remember(isbn13, value);

    if (names.some((n) => !results[n])) { // someone missed the grace period: fold their answer in when it arrives
      everyone.then(() => {
        const late = build(isbn13, results);
        if (late.merged && cache.get(isbn13)?.value === value) remember(isbn13, { ...enrich(late.merged), provider_errors: late.errors });
      });
    }
    return value;
  }

  async function lookup(isbn13) {
    const hit = cache.get(isbn13);
    if (hit && now() - hit.at < ttlMs) return hit.value;
    if (hit) cache.delete(isbn13);
    if (inflight.has(isbn13)) return inflight.get(isbn13);
    const p = lookupUncached(isbn13).finally(() => inflight.delete(isbn13));
    inflight.set(isbn13, p);
    return p;
  }
  lookup.breaker = breaker;
  lookup.cacheSize = () => cache.size;
  return lookup;
}

// ------------------------------------------------------------------- title search
//
// Search both providers by free text (title, author, or both) and merge into one list of candidates.
// Same caveat as above: response shapes follow the providers' public docs from memory and could not be
// checked against the live APIs from the build sandbox, so every field is optional and shape-checked.
//   Open Library Search API: https://openlibrary.org/dev/docs/api/search
//   Google Books volumes:    https://developers.google.com/books/docs/v1/using#PerformingSearch

/** Lowercase, accent-free, punctuation-free text for comparing titles. */
export const normText = (s) => String(s ?? '').normalize('NFKD').replace(/[\u0300-\u036f]/g, '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();

/** First entry of a list that is a valid ISBN, as ISBN-13. */
const firstIsbn13 = (list) => {
  for (const x of Array.isArray(list) ? list : []) {
    const v = toIsbn13(typeof x === 'string' ? x : x?.identifier);
    if (v) return v;
  }
  return null;
};

export async function searchOpenLibrary(q, fetchImpl = fetch) {
  const url = `https://openlibrary.org/search.json?q=${encodeURIComponent(q)}&limit=15&fields=key,title,author_name,first_publish_year,isbn,cover_i,publisher`;
  const docs = (await getJson(url, fetchImpl))?.docs;
  if (!Array.isArray(docs)) return [];
  return docs.map((d) => ({
    title: str(d?.title),
    authors: strList(d?.author_name).slice(0, 4),
    published: Number.isInteger(d?.first_publish_year) ? String(d.first_publish_year) : '',
    publisher: strList(d?.publisher)[0] ?? '',
    isbn13: firstIsbn13(d?.isbn),
    cover_url: Number.isInteger(d?.cover_i) && d.cover_i > 0 ? `https://covers.openlibrary.org/b/id/${d.cover_i}-M.jpg` : '',
    source: 'openlibrary',
  })).filter((r) => r.title);
}

export async function searchGoogleBooks(q, fetchImpl = fetch, apiKey = '') {
  const url = `https://www.googleapis.com/books/v1/volumes?q=${encodeURIComponent(q)}&maxResults=15&printType=books${apiKey ? `&key=${encodeURIComponent(apiKey)}` : ''}`;
  const items = (await getJson(url, fetchImpl))?.items;
  if (!Array.isArray(items)) return [];
  return items.map((it) => {
    const v = it?.volumeInfo ?? {};
    const ids = Array.isArray(v.industryIdentifiers) ? v.industryIdentifiers : [];
    const thumb = str(v.imageLinks?.thumbnail || v.imageLinks?.smallThumbnail).replace(/^http:/, 'https:').replace(/&edge=curl/, '');
    return {
      title: str(v.title),
      subtitle: str(v.subtitle),
      authors: strList(v.authors).slice(0, 4),
      published: str(v.publishedDate).slice(0, 10),
      publisher: str(v.publisher),
      isbn13: firstIsbn13([...ids.filter((x) => x?.type === 'ISBN_13'), ...ids]),
      cover_url: thumb,
      source: 'googlebooks',
    };
  }).filter((r) => r.title);
}

/** How well a candidate matches what was typed: 4 exact title ... 0 only loosely related. */
function matchScore(r, qNorm, qTokens) {
  // Compare without a leading "The/A/An": people type "great gatsby", the catalogue says "The Great Gatsby".
  const core = (x) => x.replace(/^(the|a|an) /, '');
  const t = normText(r.title);
  const qc = core(qNorm);
  if (core(t) === qc) return 4;
  if (core(t).startsWith(qc)) return 3;
  if (qTokens.every((tok) => t.includes(tok))) return 2;
  const hay = normText(`${r.title} ${r.authors.join(' ')}`);
  return qTokens.every((tok) => hay.includes(tok)) ? 1 : 0;
}

/**
 * Merge candidate lists from several providers: interleave (so neither provider dominates), drop duplicates
 * (same ISBN, or same title + first author), fill gaps in a duplicate from the other copy, then rank by how well
 * the title matches the query (stable, so provider order breaks ties).
 */
export function mergeSearchResults(query, lists, limit = 20) {
  const byIsbn = new Map();
  const byWork = new Map();
  const out = [];
  const longest = Math.max(0, ...lists.map((l) => l.length));
  for (let i = 0; i < longest; i++) {
    for (const list of lists) {
      const r = list[i];
      if (!r) continue;
      const workKey = `${normText(r.title)}|${normText(r.authors[0] ?? '')}`;
      const dup = (r.isbn13 && byIsbn.get(r.isbn13)) || byWork.get(workKey);
      if (dup) {
        for (const k of ['isbn13', 'cover_url', 'publisher', 'published', 'subtitle']) if (!dup[k] && r[k]) dup[k] = r[k];
        if (!dup.authors.length && r.authors.length) dup.authors = r.authors;
        if (!dup.sources.includes(r.source)) dup.sources.push(r.source);
        if (dup.isbn13) byIsbn.set(dup.isbn13, dup);
        continue;
      }
      const { source, ...rest } = r;
      const entry = { subtitle: '', ...rest, sources: [source] };
      out.push(entry);
      byWork.set(workKey, entry);
      if (entry.isbn13) byIsbn.set(entry.isbn13, entry);
    }
  }
  const qNorm = normText(query);
  const qTokens = qNorm.split(' ').filter(Boolean);
  return out
    .map((r, i) => ({ r, i, s: matchScore(r, qNorm, qTokens) }))
    .sort((a, b) => b.s - a.s || a.i - b.i)
    .slice(0, limit)
    .map(({ r }) => r);
}

const SEARCHERS = {
  openlibrary: (q, fetchImpl) => searchOpenLibrary(q, fetchImpl),
  googlebooks: (q, fetchImpl, apiKey) => searchGoogleBooks(q, fetchImpl, apiKey),
};

/**
 * Title/author search across both providers. Resolves {results, provider_errors}; throws LookupError only when
 * every provider failed or is paused. Same speed rules as lookup: answer once one provider has results (0.8 s grace
 * for the other), share the circuit breaker, de-duplicate in-flight queries, small bounded cache.
 */
export function createSearch({
  fetchImpl = fetch, googleApiKey = '', breaker = createBreaker(), now = Date.now,
  graceMs = 800, ttlMs = 2 * 60 * 1000, maxCache = 100, limit = 20,
} = {}) {
  const cache = new Map();
  const inflight = new Map();
  const names = Object.keys(SEARCHERS);

  const remember = (key, value) => {
    cache.delete(key);
    cache.set(key, { at: now(), value });
    while (cache.size > maxCache) cache.delete(cache.keys().next().value);
  };
  const build = (q, results) => ({
    results: mergeSearchResults(q, names.map((n) => results[n]?.value ?? []), limit),
    provider_errors: Object.values(results).filter((r) => r.error).map((r) => ({ provider: r.name, error: r.error })),
  });

  async function searchUncached(q, key) {
    const results = {};
    const tasks = names.map((n) => guarded(breaker, n, () => SEARCHERS[n](q, fetchImpl, googleApiKey)).then((r) => { results[n] = r; }));
    const everyone = Promise.all(tasks);
    await Promise.race(tasks);
    if (Object.values(results).some((r) => r.value?.length)) {
      let timer;
      await Promise.race([everyone, new Promise((res) => { timer = setTimeout(res, graceMs); timer.unref?.(); })]);
      clearTimeout(timer);
    } else {
      await everyone;
    }
    const value = build(q, results);
    if (!value.results.length && value.provider_errors.length === names.length) throw new LookupError('Could not reach any book search provider', value.provider_errors);
    remember(key, value);
    if (names.some((n) => !results[n])) {
      everyone.then(() => { if (cache.get(key)?.value === value) remember(key, build(q, results)); });
    }
    return value;
  }

  async function search(q) {
    const key = normText(q);
    const hit = cache.get(key);
    if (hit && now() - hit.at < ttlMs) return hit.value;
    if (hit) cache.delete(key);
    if (inflight.has(key)) return inflight.get(key);
    const p = searchUncached(q, key).finally(() => inflight.delete(key));
    inflight.set(key, p);
    return p;
  }
  search.breaker = breaker;
  search.cacheSize = () => cache.size;
  return search;
}

// ------------------------------------------------------------------- covers

const COVER_HOSTS = [/(^|\.)openlibrary\.org$/, /(^|\.)archive\.org$/, /(^|\.)books\.google\.com$/, /(^|\.)googleusercontent\.com$/, /(^|\.)gstatic\.com$/];
const MAX_COVER_BYTES = 3 * 1024 * 1024;
const EXT = { 'image/jpeg': 'jpg', 'image/png': 'png', 'image/webp': 'webp', 'image/gif': 'gif' };

/** Download a cover image. Returns {buffer, ext} or null. Never throws. */
export async function downloadCover(url, fetchImpl = fetch) {
  try {
    const u = new URL(url);
    if (u.protocol !== 'https:' || !COVER_HOSTS.some((re) => re.test(u.hostname))) return null;
    // `default=false` makes Open Library answer 404 instead of a 1x1 placeholder.
    if (/openlibrary\.org$/.test(u.hostname)) u.searchParams.set('default', 'false');
    const res = await fetchImpl(u, { headers: { 'User-Agent': UA }, signal: AbortSignal.timeout(TIMEOUT_MS) });
    if (!res.ok) return null;
    const ext = EXT[(res.headers.get('content-type') ?? '').split(';')[0].trim().toLowerCase()];
    if (!ext) return null;
    const buffer = Buffer.from(await res.arrayBuffer());
    if (buffer.length < 1024 || buffer.length > MAX_COVER_BYTES) return null; // tiny = placeholder
    return { buffer, ext };
  } catch {
    return null;
  }
}
