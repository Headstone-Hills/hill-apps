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

import { toIsbn10 } from './isbn.js';
import { classify } from './genres.js';
import { stripHtml } from './text.js';

const UA = 'HillLibrary/1.0 (personal home library app)';
const TIMEOUT_MS = 8000;

export class LookupError extends Error {
  constructor(message, providers) { super(message); this.name = 'LookupError'; this.providers = providers; }
}

const str = (v) => (typeof v === 'string' ? v.trim() : '');
const strList = (v) => (Array.isArray(v) ? v.map((x) => (typeof x === 'string' ? x : x?.name)).map(str).filter(Boolean) : []);

async function getJson(url, fetchImpl) {
  const res = await fetchImpl(url, {
    headers: { 'User-Agent': UA, Accept: 'application/json' },
    signal: AbortSignal.timeout(TIMEOUT_MS),
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
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

/**
 * Look an ISBN up. Resolves to enriched metadata, or null when no provider knows
 * anything about the book. A record with `incomplete: true` has no title but carries
 * whatever else was found (publisher, subjects, ...). Throws LookupError only when providers *failed* (network/HTTP), so the
 * UI can tell "not found" from "couldn't reach the internet".
 */
export function createLookup({ fetchImpl = fetch, googleApiKey = '', ttlMs = 10 * 60 * 1000 } = {}) {
  const cache = new Map();
  return async function lookup(isbn13) {
    const hit = cache.get(isbn13);
    if (hit && Date.now() - hit.at < ttlMs) return hit.value;

    const [ol, gb] = await Promise.allSettled([
      fetchOpenLibrary(isbn13, fetchImpl),
      fetchGoogleBooks(isbn13, fetchImpl, googleApiKey),
    ]);
    const errors = [
      ol.status === 'rejected' && { provider: 'openlibrary', error: String(ol.reason?.message ?? ol.reason) },
      gb.status === 'rejected' && { provider: 'googlebooks', error: String(gb.reason?.message ?? gb.reason) },
    ].filter(Boolean);

    const merged = mergeProviders(isbn13, ol.value ?? null, gb.value ?? null);
    if (!merged) {
      if (errors.length === 2) throw new LookupError('Could not reach any metadata provider', errors);
      return null; // at least one provider answered and neither knows this ISBN
    }
    const value = { ...enrich(merged), provider_errors: errors };
    cache.set(isbn13, { at: Date.now(), value });
    return value;
  };
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
