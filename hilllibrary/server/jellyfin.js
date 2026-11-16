// Jellyfin connection: read-only view of the movies on your media server, plus the logic that
// lines them up with the discs you own so the Movies tab can show "disc", "server" or "both".
//
// Auth is a Jellyfin API key (Dashboard -> Advanced -> API Keys), sent as
//   Authorization: MediaBrowser Token="<key>"
// API reference: https://api.jellyfin.org/  (Items: GET /Items, images: GET /Items/{id}/Images/Primary)
// The key stays on the server: posters are proxied through /api/jellyfin/image/:id.

import { classifyMovie } from './moviegenres.js';

const UA = 'HillLibrary/1.0 (personal home library app)';
const TIMEOUT_MS = 15000;
const PAGE = 500;
const str = (v) => (typeof v === 'string' ? v.trim() : '');

export class JellyfinError extends Error {
  constructor(message, status) { super(message); this.name = 'JellyfinError'; this.status = status; }
}

/**
 * @param {{url?:string, apiKey?:string, publicUrl?:string, library?:string, fetchImpl?:typeof fetch, ttlMs?:number}} o
 *   `library`: name of the Jellyfin library to read. Default: every library of type "Movies", so home videos,
 *   concert/extras folders and the like never show up as films.
 */
export function createJellyfin({ url = '', apiKey = '', publicUrl = '', library = '', fetchImpl = fetch, ttlMs = 5 * 60 * 1000 } = {}) {
  const base = url.replace(/\/+$/, '');
  const enabled = !!(base && apiKey);
  let cache = null; // {at, items}
  let inflight = null;

  async function call(path) {
    let res;
    try {
      res = await fetchImpl(`${base}${path}`, {
        headers: { 'User-Agent': UA, Accept: 'application/json', Authorization: `MediaBrowser Token="${apiKey}"` },
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
    } catch (e) {
      throw new JellyfinError(`Could not reach Jellyfin at ${base} (${e?.cause?.code ?? e?.message ?? 'network error'}).`, 0);
    }
    if (res.status === 401 || res.status === 403) throw new JellyfinError('Jellyfin rejected the API key. Create a new one under Dashboard > Advanced > API Keys.', res.status);
    if (!res.ok) throw new JellyfinError(`Jellyfin answered HTTP ${res.status}.`, res.status);
    return res;
  }

  /** The libraries to read: the one named `library`, else every library whose type is "movies". */
  async function movieLibraries() {
    const body = await (await call('/Library/VirtualFolders')).json().catch(() => null);
    const all = Array.isArray(body) ? body : [];
    const want = library.trim().toLowerCase();
    const picked = all.filter((l) => (want ? str(l?.Name).toLowerCase() === want : str(l?.CollectionType).toLowerCase() === 'movies'))
      .filter((l) => /^[0-9a-f]{32}$/i.test(str(l?.ItemId)));
    if (!picked.length) {
      const names = all.map((l) => str(l?.Name)).filter(Boolean).join(', ') || 'none';
      throw new JellyfinError(want
        ? `Jellyfin has no library named "${library}" (libraries: ${names}).`
        : `Jellyfin has no library of type Movies (libraries: ${names}). Set JELLYFIN_MOVIE_LIBRARY to the library's name.`, 0);
    }
    return picked.map((l) => ({ id: str(l.ItemId), name: str(l.Name) }));
  }

  async function loadMovies() {
    const libs = await movieLibraries();
    const items = [];
    const seen = new Set();
    for (const lib of libs) await loadLibrary(lib.id, items, seen);
    return { items, libraries: libs.map((l) => l.name) };
  }

  async function loadLibrary(parentId, items, seen) {
    for (let start = 0; ; start += PAGE) {
      const q = new URLSearchParams({
        ParentId: parentId, IncludeItemTypes: 'Movie', Recursive: 'true', SortBy: 'SortName', SortOrder: 'Ascending',
        Fields: 'ProviderIds,Genres,Overview,Studios,People,RunTimeTicks,ProductionYear',
        StartIndex: String(start), Limit: String(PAGE),
      });
      const body = await (await call(`/Items?${q}`)).json().catch(() => null);
      const page = Array.isArray(body?.Items) ? body.Items : [];
      for (const x of page) {
        const m = normalizeItem(x);
        if (m && !seen.has(m.id)) { seen.add(m.id); items.push(m); }
      }
      if (page.length < PAGE || start + page.length >= (body?.TotalRecordCount ?? 0)) break;
    }
  }

  return {
    enabled, publicUrl: publicUrl.replace(/\/+$/, ''),

    /** `{at, items, libraries}` for the movies on the server (cached a few minutes; {force:true} refetches). Throws JellyfinError. */
    async movies({ force = false } = {}) {
      if (!enabled) return { at: 0, items: [], libraries: [] };
      if (!force && cache && Date.now() - cache.at < ttlMs) return cache;
      inflight ??= loadMovies().then((r) => (cache = { at: Date.now(), ...r })).finally(() => { inflight = null; });
      return inflight;
    },

    /** Poster bytes for one item, or null. */
    async image(id, tag = '') {
      if (!enabled || !/^[0-9a-f]{32}$/i.test(id)) return null;
      const q = new URLSearchParams({ maxHeight: '480', quality: '85', ...(tag ? { tag } : {}) });
      let res;
      try { res = await call(`/Items/${id}/Images/Primary?${q}`); } catch (e) { if (e.status === 404) return null; throw e; }
      const type = (res.headers.get('content-type') ?? '').split(';')[0].trim().toLowerCase();
      if (!/^image\/(jpeg|png|webp|gif)$/.test(type)) return null;
      return { buffer: Buffer.from(await res.arrayBuffer()), type };
    },
  };
}

/** One Jellyfin movie -> the small shape the app uses. Tolerates missing/odd fields. */
export function normalizeItem(x) {
  const id = str(x?.Id);
  const title = str(x?.Name);
  if (!/^[0-9a-f]{32}$/i.test(id) || !title) return null;
  const names = (v) => (Array.isArray(v) ? v.map((s) => str(typeof s === 'string' ? s : s?.Name)).filter(Boolean) : []);
  const ticks = Number(x.RunTimeTicks);
  const tmdb = Number.parseInt(x.ProviderIds?.Tmdb ?? x.ProviderIds?.tmdb, 10);
  return {
    id, title,
    year: Number.isInteger(x.ProductionYear) ? String(x.ProductionYear) : '',
    overview: str(x.Overview),
    genres: names(x.Genres),
    studio: names(x.Studios)[0] ?? '',
    directors: Array.isArray(x.People) ? x.People.filter((p) => p?.Type === 'Director').map((p) => str(p.Name)).filter(Boolean).slice(0, 3) : [],
    runtime: ticks > 0 ? Math.round(ticks / 600_000_000) : null, // 10,000,000 ticks per second
    tmdbId: Number.isInteger(tmdb) && tmdb > 0 ? tmdb : null,
    imageTag: str(x.ImageTags?.Primary),
  };
}

// ------------------------------------------------------------------ matching

/** "Seven Samurai (Criterion Collection)" and "Seven Samurai" must compare equal. */
export const matchTitle = (t) => String(t ?? '').toLowerCase().replace(/['’`]/g, '')
  .replace(/\s*[[(][^\])]*[\])]/g, ' ').replace(/^(the|a|an)\s+/, '').replace(/[^\p{L}\p{N}]+/gu, ' ').trim();

const yearOf = (v) => (/^\d{4}/.test(String(v ?? '')) ? String(v).slice(0, 4) : '');
const yearsCompatible = (a, b) => !a || !b || Math.abs(Number(a) - Number(b)) <= 1; // release-year conventions differ by one

/**
 * Merge owned discs and server movies into one list of films.
 * Matching: TMDB id when both sides have one, else normalised title with a compatible year.
 * @param {object[]} discs  library rows with media dvd|bluray
 * @param {object[]} server normalized Jellyfin items
 */
/** First candidate in the path, the rest in `also`; the image route tries them in order until one is served. */
function posterUrl(posters) {
  if (!posters.length) return '';
  const [first, ...rest] = posters.slice(0, 5);
  const also = rest.map((p) => `${p.id}.${p.tag}`).join(',');
  return `/api/jellyfin/image/${first.id}?tag=${encodeURIComponent(first.tag)}${also ? `&also=${encodeURIComponent(also)}` : ''}`;
}

export function combineMovies(discs, server) {
  const entries = [];
  const byTmdb = new Map();
  const byTitle = new Map();

  const find = (tmdbId, title, year) => {
    if (tmdbId && byTmdb.has(tmdbId)) return byTmdb.get(tmdbId);
    return (byTitle.get(matchTitle(title)) ?? []).find((e) => yearsCompatible(e.year, year) && !(tmdbId && e.tmdbId && e.tmdbId !== tmdbId));
  };
  const register = (e) => {
    entries.push(e);
    const k = matchTitle(e.title);
    byTitle.set(k, [...(byTitle.get(k) ?? []), e]);
  };
  const adopt = (e, tmdbId) => { if (tmdbId && !e.tmdbId) { e.tmdbId = tmdbId; byTmdb.set(tmdbId, e); } };

  for (const b of discs) {
    const tmdbId = b.meta_raw?.tmdb?.kind === 'movie' ? b.meta_raw.tmdb.id : null;
    const year = yearOf(b.published);
    let e = find(tmdbId, b.title, year);
    if (!e) {
      e = { title: b.title, year, tmdbId: null, copies: [], jellyfin: null, genre: b.genre, subgenre: b.subgenre, directors: b.authors ?? [], runtime: b.runtime ?? null, overview: b.description ?? '', coverFile: null, coverV: '' };
      register(e);
    }
    adopt(e, tmdbId);
    e.copies.push({ id: b.id, media: b.media, barcode: b.barcode, status: b.status, location: b.location, needs_review: b.needs_review });
    if (!e.coverFile && b.has_cover) { e.coverFile = b.cover_file; e.coverV = b.updated_at ?? ''; }
    if (!e.year && year) e.year = year;
  }

  for (const j of server) {
    let e = find(j.tmdbId, j.title, j.year);
    if (!e) {
      const c = classifyMovie({ title: j.title, description: j.overview, genres: j.genres, year: j.year });
      e = { title: j.title, year: j.year, tmdbId: null, copies: [], jellyfin: null, genre: c.genre, subgenre: c.subgenre, directors: j.directors, runtime: j.runtime, overview: j.overview, coverFile: null, coverV: '' };
      register(e);
    }
    adopt(e, j.tmdbId);
    // Jellyfin often lists a film twice (two copies/versions) and one entry may carry a stale poster, so keep every candidate.
    if (!e.jellyfin) e.jellyfin = { id: j.id, posters: [] };
    if (j.imageTag) e.jellyfin.posters.push({ id: j.id, tag: j.imageTag });
    if (!e.overview) e.overview = j.overview;
    if (!e.runtime) e.runtime = j.runtime;
    if (!e.directors.length) e.directors = j.directors;
  }

  return entries.map((e) => ({
    key: e.copies.length ? `d${e.copies[0].id}` : `j${e.jellyfin.id}`,
    title: e.title, year: e.year, genre: e.genre, subgenre: e.subgenre, directors: e.directors, runtime: e.runtime, overview: e.overview,
    availability: e.copies.length && e.jellyfin ? 'both' : e.copies.length ? 'disc' : 'server',
    copies: e.copies,
    jellyfin: e.jellyfin ? { id: e.jellyfin.id } : null,
    poster: e.coverFile ? `/covers/${e.coverFile}?v=${encodeURIComponent(e.coverV)}` : posterUrl(e.jellyfin?.posters ?? []),
  })).sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: 'base' }));
}
