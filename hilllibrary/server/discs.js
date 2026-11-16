// DVD / Blu-ray metadata lookup by retail barcode (UPC-A / EAN-13).
//
// Two stages, because no single free source does both jobs:
//   1. UPCitemdb (free "trial" endpoint, no key, ~100 lookups/day per IP) maps the barcode to a
//      product listing: a messy retail title such as "Spider-man DVD", a brand and a description.
//        https://www.upcitemdb.com/wp/docs/main/development/getting-started/
//   2. TMDB (optional; needs a free API key in TMDB_API_KEY) matches the cleaned title to a real
//      film/series and supplies genres, director, runtime, year, overview and a poster.
//        https://developer.themoviedb.org/docs/getting-started
// Without a TMDB key you still get the title/studio, but genre is a weak keyword guess flagged
// "needs review". Disc listings do not reliably say whether the product is a DVD or a Blu-ray,
// so the format is a guess the user can correct (`format_guess`).

import { classifyMovie } from './moviegenres.js';
import { LookupError } from './metadata.js';

const UA = 'HillLibrary/1.0 (personal home library app)';
const TIMEOUT_MS = 8000;
const str = (v) => (typeof v === 'string' ? v.trim() : '');

async function getJson(url, fetchImpl, headers = {}) {
  const res = await fetchImpl(url, { headers: { 'User-Agent': UA, Accept: 'application/json', ...headers }, signal: AbortSignal.timeout(TIMEOUT_MS) });
  let body = null;
  try { body = await res.json(); } catch { /* non-JSON error page */ }
  return { status: res.status, ok: res.ok, body };
}

// ------------------------------------------------------------- title cleanup

const FORMAT_WORDS = '(?:dvd|blu-?\\s?ray|bluray|bd|4k|uhd|ultra\\s?hd|hd|widescreen|full\\s?screen|fullscreen|ntsc|pal|region\\s?[a-z0-9]|' +
  'special edition|collector\'?s edition|anniversary edition|steelbook|combo pack|digital copy|digital hd|\\+ digital|' +
  '\\d-?\\s?disc(?: set)?|rated|unrated|director\'?s cut|extended edition|theatrical)';
const BRACKETED = new RegExp(`\\s*[\\[(][^\\])]*${FORMAT_WORDS}[^\\])]*[\\])]`, 'gi');
const LOOSE = new RegExp(`(?:^|\\s)[-–:,]?\\s*${FORMAT_WORDS}(?=$|[\\s,:;)\\]])`, 'gi');

/** "Alien (Blu-ray) [Widescreen]" -> "Alien". Falls back to the input if cleaning empties it. */
export function cleanDiscTitle(raw) {
  let t = String(raw ?? '').replace(/\s+/g, ' ').trim();
  t = t.replace(BRACKETED, '').replace(LOOSE, ' ').replace(LOOSE, ' ');
  t = t.replace(/\s*[-–:,/+&]\s*$/, '').replace(/\s+/g, ' ').trim();
  return t || String(raw ?? '').trim();
}

/**
 * Search strings to try on TMDB, most specific first. Retail titles carry edition noise that TMDB
 * doesn't know, e.g. "Memories of Murder (Criterion Collection)" or "Blade Runner: The Final Cut".
 */
export function titleCandidates(title) {
  const full = cleanDiscTitle(title);
  const noParens = full.replace(/\s*[\[(][^\])]*[\])]/g, '').replace(/\s+/g, ' ').trim();
  const head = noParens.split(/\s+[:\-–]\s+|:\s+/)[0].trim();
  return [...new Set([full, noParens, head].filter((t) => t.length >= 2))];
}

/** "Friends: The Complete First Season" -> "Friends" (series name for a TV search). */
export function seriesName(title) {
  const t = title.replace(/[:\-–,]?\s*(?:the\s+)?(?:complete\s+)?(?:\w+\s+)?(?:season|series|volume|vol\.?)\b.*$/i, '').trim();
  return t || title;
}
const looksLikeTv = (title) => /\b(season|series|complete (?:series|collection)|volume \d+|vol\.? \d+)\b/i.test(title);

/** @returns {{media:'dvd'|'bluray', fourK:boolean, certain:boolean}} */
export function guessFormat(...texts) {
  const s = texts.join(' ').toLowerCase();
  const fourK = /\b4k\b|\buhd\b|ultra\s?hd/.test(s);
  if (/blu-?\s?ray|\bbluray\b|\bbd\b/.test(s) || fourK) return { media: 'bluray', fourK, certain: true };
  if (/\bdvd\b/.test(s)) return { media: 'dvd', fourK: false, certain: true };
  return { media: 'dvd', fourK: false, certain: false };
}

// ---------------------------------------------------------------- providers

/** Returns a product record, null if the barcode isn't a recognisable disc, or throws on provider failure. */
export async function fetchUpcItemDb(gtin13, fetchImpl = fetch) {
  const { status, body } = await getJson(`https://api.upcitemdb.com/prod/trial/lookup?upc=${gtin13}`, fetchImpl);
  const code = str(body?.code);
  if (code === 'INVALID_UPC' || code === 'NOT_FOUND') return null;
  if (status === 429 || code === 'TOO_FAST' || code === 'EXCEED_LIMIT') throw new Error('rate limited (UPCitemdb allows about 100 free lookups a day)');
  if (status >= 400 || (code && code !== 'OK')) throw new Error(`HTTP ${status} ${code}`.trim());
  const item = Array.isArray(body?.items) ? body.items[0] : null;
  if (!item || typeof item !== 'object') return null;
  const rec = { title: str(item.title), description: str(item.description), brand: str(item.brand), category: str(item.category) };
  // The database holds every kind of product (a made-up code returned bread rolls), so insist this looks like a disc.
  const looksDisc = /dvd|blu-?\s?ray|\b4k\b|\buhd\b|video|movie|film|season|series/i.test(`${rec.title} ${rec.category}`);
  return rec.title && looksDisc ? rec : null;
}

function tmdbAuth(key) {
  // v4 "read access tokens" are long JWTs sent as a bearer header; v3 keys go in the query string.
  return key.length > 40 ? { headers: { Authorization: `Bearer ${key}` }, q: '' } : { headers: {}, q: `&api_key=${encodeURIComponent(key)}` };
}

const norm = (s) => String(s).toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();

/** Full TMDB record for one known film/series id. Returns null if TMDB has no such id. Throws on other HTTP failures. */
export async function fetchTmdbDetails(kind, id, { key, fetchImpl = fetch }) {
  if (kind !== 'movie' && kind !== 'tv') throw new Error('kind must be movie or tv');
  const auth = tmdbAuth(key);
  const d = await getJson(`https://api.themoviedb.org/3/${kind}/${id}?append_to_response=credits${auth.q}`, fetchImpl, auth.headers);
  if (d.status === 404) return null;
  if (!d.ok) throw new Error(`TMDB HTTP ${d.status}`);
  if (!d.body || typeof d.body !== 'object') return null;
  const b = d.body;
  const names = (v) => (Array.isArray(v) ? v.map((x) => str(x?.name)).filter(Boolean) : []);
  const directors = kind === 'movie'
    ? (Array.isArray(b.credits?.crew) ? b.credits.crew.filter((c) => c?.job === 'Director').map((c) => str(c.name)).filter(Boolean) : [])
    : names(b.created_by);
  const date = str(kind === 'movie' ? b.release_date : b.first_air_date);
  const runtime = kind === 'movie' ? b.runtime : Array.isArray(b.episode_run_time) ? b.episode_run_time[0] : null;
  return {
    id, kind,
    title: str(b.title ?? b.name),
    genres: names(b.genres),
    directors: [...new Set(directors)].slice(0, 3),
    studio: names(b.production_companies)[0] ?? names(b.networks)[0] ?? '',
    year: date.slice(0, 4),
    runtime: Number.isInteger(runtime) && runtime > 0 ? runtime : null,
    overview: str(b.overview),
    poster: str(b.poster_path).startsWith('/') ? `https://image.tmdb.org/t/p/w500${b.poster_path}` : '',
  };
}

/** Look a cleaned title up on TMDB. Returns a normalised record or null (no match). Throws on HTTP failure. */
export async function fetchTmdb(title, { tv = false, key, fetchImpl = fetch }) {
  const auth = tmdbAuth(key);
  const kinds = tv ? ['tv', 'movie'] : ['movie', 'tv'];
  for (const kind of kinds) {
    const name = kind === 'tv' ? seriesName(title) : title;
    const search = await getJson(`https://api.themoviedb.org/3/search/${kind}?query=${encodeURIComponent(name)}&include_adult=false${auth.q}`, fetchImpl, auth.headers);
    if (!search.ok) throw new Error(`TMDB HTTP ${search.status}`);
    const results = Array.isArray(search.body?.results) ? search.body.results : [];
    if (!results.length) continue;
    const label = (r) => str(r.title ?? r.name);
    const exact = results.find((r) => norm(label(r)) === norm(name));
    const pick = exact ?? results[0];
    if (!Number.isInteger(pick?.id)) continue;
    const details = await fetchTmdbDetails(kind, pick.id, { key, fetchImpl });
    if (details) return { ...details, exact: !!exact };
  }
  return null;
}

/**
 * Free-text TMDB search for the editor's "find this film" box: movies and TV together.
 * Optional `year` keeps only results released/first aired that year.
 * @returns {Promise<{id:number, kind:'movie'|'tv', title:string, year:string, overview:string, poster:string}[]>}
 */
export async function searchTmdb(query, { year = '', key, fetchImpl = fetch }) {
  const auth = tmdbAuth(key);
  const r = await getJson(`https://api.themoviedb.org/3/search/multi?query=${encodeURIComponent(query)}&include_adult=false${auth.q}`, fetchImpl, auth.headers);
  if (!r.ok) throw new Error(`TMDB HTTP ${r.status}`);
  const out = [];
  for (const x of Array.isArray(r.body?.results) ? r.body.results : []) {
    if ((x?.media_type !== 'movie' && x?.media_type !== 'tv') || !Number.isInteger(x.id)) continue;
    const date = str(x.media_type === 'movie' ? x.release_date : x.first_air_date);
    if (year && !date.startsWith(year)) continue;
    const overview = str(x.overview);
    out.push({
      id: x.id, kind: x.media_type, title: str(x.title ?? x.name), year: date.slice(0, 4),
      overview: overview.length > 180 ? `${overview.slice(0, 177)}...` : overview,
      poster: str(x.poster_path).startsWith('/') ? `https://image.tmdb.org/t/p/w185${x.poster_path}` : '',
    });
    if (out.length >= 10) break;
  }
  return out;
}

// ------------------------------------------------------------------- merge

export function buildDiscMeta(gtin13, upc, tmdb) {
  const cleaned = cleanDiscTitle(upc?.title ?? tmdb?.title ?? '');
  const title = tmdb?.exact ? tmdb.title : cleaned || tmdb?.title || '';
  if (!title) return null;
  const fmt = guessFormat(upc?.title, upc?.description);
  const description = tmdb?.overview || (upc?.description && upc.description.toLowerCase() !== cleaned.toLowerCase() ? upc.description : '');
  const tv = tmdb?.kind === 'tv' || (!tmdb && looksLikeTv(upc?.title ?? ''));
  const meta = {
    media: fmt.media, format_guess: fmt.certain ? 'sure' : 'guess', barcode: gtin13,
    title, subtitle: '', authors: tmdb?.directors ?? [], publisher: tmdb?.studio || upc?.brand || '',
    published: tmdb?.year ?? '', runtime: tmdb?.runtime ?? null, pages: null, description,
    cover_url: tmdb?.poster ?? '', genres: tmdb?.genres ?? [], tv, fourK: fmt.fourK, tmdb_match: tmdb ? (tmdb.exact ? 'exact' : 'closest') : 'none',
    sources: [upc && 'upcitemdb', tmdb && 'tmdb'].filter(Boolean),
    // Remembered only for an exact title match, so a wrong "closest" guess can never link two different films.
    ...(tmdb?.exact ? { tmdb: { id: tmdb.id, kind: tmdb.kind } } : {}),
  };
  return meta;
}

/** Attach classifier output to a disc record (same shape as books' `enrich`). */
export function enrichDisc(meta) {
  const c = classifyMovie({ title: meta.title, description: meta.description, genres: meta.genres, tv: meta.tv, year: meta.published, fourK: meta.fourK });
  // A TMDB title that was only the "closest" search hit may be the wrong film: ask the user to check.
  const needsReview = c.needsReview || meta.tmdb_match === 'closest';
  return { ...meta, genre: c.genre, subgenre: c.subgenre, tags: c.tags, confidence: meta.tmdb_match === 'closest' && c.confidence === 'high' ? 'medium' : c.confidence, needs_review: needsReview };
}

/**
 * @returns {(gtin13:string)=>Promise<object|null>} resolves to enriched metadata, null when unknown,
 *   and throws LookupError when providers failed (so the UI can say "couldn't reach" instead of "not found").
 */
export function createDiscLookup({ fetchImpl = fetch, tmdbKey = '', ttlMs = 10 * 60 * 1000 } = {}) {
  const cache = new Map();
  return async function lookupDisc(gtin13) {
    const hit = cache.get(gtin13);
    if (hit && Date.now() - hit.at < ttlMs) return hit.value;

    let upc = null;
    try { upc = await fetchUpcItemDb(gtin13, fetchImpl); }
    catch (e) { throw new LookupError('Could not look up that barcode', [{ provider: 'upcitemdb', error: String(e?.message ?? e) }]); }
    if (!upc) return null;

    const errors = [];
    let tmdb = null;
    if (tmdbKey) {
      try {
        for (const candidate of titleCandidates(upc.title)) {
          tmdb = await fetchTmdb(candidate, { tv: looksLikeTv(upc.title), key: tmdbKey, fetchImpl });
          if (tmdb) break;
        }
      } catch (e) { errors.push({ provider: 'tmdb', error: String(e?.message ?? e) }); }
    }
    const meta = buildDiscMeta(gtin13, upc, tmdb);
    if (!meta) return null;
    const value = { ...enrichDisc(meta), provider_errors: errors };
    cache.set(gtin13, { at: Date.now(), value });
    return value;
  };
}

/**
 * Client for the editor's search box. `details` returns the same enriched shape as a barcode lookup
 * (genre, subgenre, tags, ...) so the form can show what the classifier will pick.
 */
export function createTmdbClient({ fetchImpl = fetch, key = '' } = {}) {
  return {
    enabled: !!key,
    search: (query, year = '') => searchTmdb(query, { year, key, fetchImpl }),
    async details(kind, id) {
      const t = await fetchTmdbDetails(kind, id, { key, fetchImpl });
      if (!t) return null;
      const meta = buildDiscMeta(null, null, { ...t, exact: true });
      return meta && { ...enrichDisc(meta), tmdb: { id, kind } };
    },
  };
}
