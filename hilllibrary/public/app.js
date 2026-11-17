// Hill Library front end. Vanilla JS, no build step.
// Security note: book metadata is third-party content, so it is only ever inserted
// with textContent / DOM APIs (never innerHTML).

import { createScanner, cameraProblem, decodeIsbnFromImage } from './scanner.js';

const $app = document.getElementById('app');
const sheet = document.getElementById('sheet');
const state = { session: null, facets: null, taxonomy: null };
let activeScanner = null;

// ------------------------------------------------------------------ helpers
const RO_ATTRS = new Set(['value', 'checked', 'disabled', 'selected', 'open', 'textContent', 'hidden']);
function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props ?? {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
    else if (RO_ATTRS.has(k)) el[k] = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

const ICONS = {
  library: 'M4 4h4v16H4zM10 4h4v16h-4zM16.5 5.2l3.8-1 3.2 14.8-3.8 1z',
  scan: 'M3 5v4M6 5v14M9 5v8M12 5v14M15 5v4M18 5v14M21 5v8M3 15v4M9 17v2M15 15v4M21 17v2',
  movies: 'M4 5h16v14H4zM8 5v14M16 5v14M4 9h4M4 15h4M16 9h4M16 15h4',
  browse: 'M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z',
  more: 'M5 12h.01M12 12h.01M19 12h.01',
};
function icon(name) {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('fill', 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', '2');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  svg.setAttribute('aria-hidden', 'true');
  const p = document.createElementNS(ns, 'path');
  p.setAttribute('d', ICONS[name]);
  svg.append(p);
  return svg;
}

async function api(path, { method = 'GET', body } = {}) {
  const raw = body instanceof Blob;     // a photo is sent as-is, everything else as JSON
  const res = await fetch(path, {
    method,
    headers: raw ? { 'Content-Type': body.type } : body !== undefined ? { 'Content-Type': 'application/json' } : {},
    body: raw ? body : body !== undefined ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => null);
  if (res.status === 401 && data?.authRequired) {
    state.session = { authRequired: true, authed: false };
    route();
  }
  if (!res.ok) throw Object.assign(new Error(data?.error || `Request failed (${res.status})`), { status: res.status, data });
  return data;
}

function toast(msg, { error = false, ms = 2500 } = {}) {
  const box = document.getElementById('toasts');
  while (box.children.length >= 2) box.firstChild.remove(); // never let notices pile up over the content
  const t = h('div', { class: `toast${error ? ' err' : ''}` }, msg);
  box.append(t);
  setTimeout(() => t.remove(), error ? ms + 2000 : ms);
}

function hue(s) {
  let x = 0;
  for (const ch of String(s)) x = (x * 31 + ch.codePointAt(0)) % 360;
  return x;
}
const withHue = (el, name) => { el.style.setProperty('--h', hue(name)); return el; };

const pref = {
  get(k, d) { try { return JSON.parse(localStorage.getItem(`hl.${k}`)) ?? d; } catch { return d; } },
  set(k, v) { try { localStorage.setItem(`hl.${k}`, JSON.stringify(v)); } catch { /* private mode */ } },
};

/** Like el.append, but skips null/false (DOM append would print them as the text "null"). */
function add(el, ...kids) { el.append(...kids.flat().filter((k) => k != null && k !== false)); }

const PROVIDER_NAMES = { openlibrary: 'Open Library', googlebooks: 'Google Books' };
/** "Open Library: HTTP 429 · Google Books: …" from a failed lookup/search response, or '' if it carries no detail. */
function whyFailed(e) {
  const list = e?.data?.providers;
  return Array.isArray(list) && list.length ? list.map((x) => `${PROVIDER_NAMES[x.provider] ?? x.provider}: ${x.error}`).join(' · ') : '';
}

/** append() that skips null/false (Element.append(null) would insert the literal text "null"). */
function put(parent, ...kids) {
  parent.append(...kids.flat(Infinity).filter((k) => k != null && k !== false));
  return parent;
}

/** replaceChildren() that flattens arrays and skips null/false (the native method would insert the text "null"). */
function swap(parent, ...kids) {
  parent.replaceChildren(...kids.flat(Infinity).filter((k) => k != null && k !== false));
  return parent;
}

function debounce(fn, ms) { let t; return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); }; }
const isMovie = (b) => !!b.media && b.media !== 'book';
const FORMAT_LABEL = { dvd: 'DVD', bluray: 'Blu-ray' };
const MEDIA_FILTERS = [['', 'All media'], ['book', 'Books'], ['movie', 'All movies'], ['dvd', 'DVDs'], ['bluray', 'Blu-rays']];
const STATUS_LABELS = {
  book: { unread: 'Unread', reading: 'Reading', read: 'Read' },
  movie: { unread: 'Unwatched', reading: 'Watching', read: 'Watched' },
};
const authorsText = (b) => (b.authors?.length ? b.authors.join(', ') : isMovie(b) ? 'Unknown director' : 'Unknown author');
const itemNoun = (n, movie) => (movie ? (n === 1 ? 'movie' : 'movies') : (n === 1 ? 'book' : 'books'));
const changed = () => window.dispatchEvent(new CustomEvent('library-changed'));

// ------------------------------------------------------------------ photos
/**
 * Ask for a photo. With capture=true phones open their camera app directly (full-resolution,
 * proper autofocus, flash: far better than a video frame); without it the user picks a file.
 * Must be called from a click handler. Resolves to a File, or null if cancelled.
 */
function pickPhoto({ capture = false } = {}) {
  return new Promise((resolve) => {
    const input = h('input', { type: 'file', accept: 'image/*', capture: capture ? 'environment' : null, class: 'hidden' });
    input.addEventListener('change', () => { resolve(input.files?.[0] ?? null); input.remove(); }, { once: true });
    input.addEventListener('cancel', () => { resolve(null); input.remove(); }, { once: true });
    document.body.append(input);
    input.click();
  });
}

/** Downscale to a JPEG of at most maxSide px (phone photos are 4-12 MB; a cover needs ~100 KB). */
async function shrinkImage(file, maxSide = 1400) {
  const url = URL.createObjectURL(file);
  try {
    const img = new Image();
    img.src = url;
    await img.decode(); // browsers apply the photo's EXIF rotation when drawing it
    const scale = Math.min(1, maxSide / Math.max(img.naturalWidth, img.naturalHeight));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
    canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
    canvas.getContext('2d').drawImage(img, 0, 0, canvas.width, canvas.height);
    const blob = await new Promise((r) => canvas.toBlob(r, 'image/jpeg', 0.85));
    if (blob) return blob;
  } catch { /* fall through to the original file */ } finally { URL.revokeObjectURL(url); }
  if (file.size <= 4.5 * 1024 * 1024 && /^image\/(jpeg|png|webp)$/.test(file.type)) return file;
  throw new Error('Could not read that image. Try a JPEG or PNG.');
}

async function uploadCover(id, blob) {
  const res = await fetch(`/api/books/${id}/cover`, { method: 'PUT', headers: { 'Content-Type': blob.type || 'image/jpeg' }, body: blob });
  const data = await res.json().catch(() => null);
  if (res.status === 401 && data?.authRequired) { state.session = { authRequired: true, authed: false }; route(); }
  if (!res.ok) throw Object.assign(new Error(data?.error || `Upload failed (${res.status})`), { status: res.status });
  return data;
}

async function loadFacets(force = false) {
  if (force || !state.facets) state.facets = await api('/api/facets');
  if (!state.taxonomy) state.taxonomy = await api('/api/taxonomy');
}

// ------------------------------------------------------------------ covers
function coverEl(book) {
  // ?v= busts the browser's day-long cache when a cover file is replaced under the same name (e.g. a TMDB match).
  if (book.has_cover) return h('img', { src: `/covers/${book.cover_file}?v=${encodeURIComponent(book.updated_at ?? '')}`, alt: '', loading: 'lazy' });
  return withHue(h('div', { class: 'fake-cover' }, book.title), book.genre);
}

// ------------------------------------------------------------------ routing
function parseRoute() {
  const [path, qs = ''] = location.hash.replace(/^#\/?/, '').split('?');
  return { path: path || 'library', params: new URLSearchParams(qs) };
}

function stopScanner() { activeScanner?.stop(); activeScanner = null; }

function shell(path, page) {
  const link = (to, label, ic) => h('a', { href: `#/${to}`, 'aria-current': path === to ? 'page' : null }, icon(ic), h('span', {}, label));
  $app.replaceChildren(
    page,
    h('nav', { class: 'nav', 'aria-label': 'Main' },
      link('library', 'Library', 'library'), link('movies', 'Movies', 'movies'), link('scan', 'Scan', 'scan'), link('browse', 'Browse', 'browse'), link('more', 'More', 'more')),
  );
}

async function route() {
  stopScanner();
  try {
    if (!state.session) state.session = await api('/api/session');
    if (state.session.authRequired && !state.session.authed) return renderLogin();
    await loadFacets(true); // counts change whenever a book is added/edited on any tab, so never trust a cached copy
  } catch (e) {
    $app.replaceChildren(h('div', { class: 'page' }, h('div', { class: 'notice err' }, `Can't reach the library server: ${e.message}`),
      h('button', { class: 'btn', onclick: () => route() }, 'Try again')));
    return;
  }
  const { path, params } = parseRoute();
  const views = { library: viewLibrary, movies: viewMovies, scan: viewScan, browse: viewBrowse, more: viewMore };
  const view = views[path] ?? viewLibrary;
  const page = h('div', { class: 'page' });
  shell(views[path] ? path : 'library', page);
  window.scrollTo(0, 0);
  await view(page, params);
}
window.addEventListener('hashchange', route);
document.addEventListener('visibilitychange', () => { if (document.hidden) stopScanner(); });

// -------------------------------------------------------------------- login
function renderLogin() {
  const pw = h('input', { type: 'password', autocomplete: 'current-password', required: true, id: 'pw' });
  const err = h('p', { class: 'muted', role: 'alert' });
  $app.replaceChildren(h('div', { class: 'page login' },
    h('h1', {}, 'Hill Library'),
    h('form', {
      onsubmit: async (e) => {
        e.preventDefault();
        try {
          await api('/api/login', { method: 'POST', body: { password: pw.value } });
          state.session = null; route();
        } catch (ex) { err.textContent = ex.message; pw.select(); }
      },
    }, h('div', { class: 'field' }, h('label', { for: 'pw' }, 'Password'), pw), h('button', { class: 'btn primary', type: 'submit' }, 'Unlock'), err)));
  pw.focus();
}

// ------------------------------------------------------------------ library
async function viewLibrary(page, params) {
  const filters = ['media', 'genre', 'subgenre', 'tag', 'status', 'location'].reduce((o, k) => ({ ...o, [k]: params.get(k) || '' }), {});
  filters.review = params.get('review') === '1';
  let q = params.get('q') || '';
  let sort = pref.get('sort', 'genre');
  let view = pref.get('view', 'grid');
  let seq = 0;
  // Genres and tags to offer, limited to the chosen media type so every option leads to results.
  const fx = await api(`/api/facets?${new URLSearchParams(Object.entries({ media: filters.media, genre: filters.genre }).filter(([, v]) => v))}`).catch(() => state.facets);

  const list = h('div', { 'aria-live': 'polite' });
  const count = h('span', { class: 'count' });
  const chips = h('div', { class: 'chips' });

  const urlFor = (o = {}) => {
    const p = new URLSearchParams();
    const all = { ...filters, q, ...o };
    for (const [k, v] of Object.entries(all)) if (v) p.set(k, v === true ? '1' : v);
    return `#/library${p.toString() ? `?${p}` : ''}`;
  };

  const search = h('input', {
    type: 'search', id: 'q', value: q, placeholder: 'Search title, author, director, tag, barcode…', autocomplete: 'off',
    'aria-label': 'Search your library', enterkeyhint: 'search',
  });
  search.addEventListener('input', debounce(() => {
    if (!search.isConnected) return; // the user already navigated away; a stale timer must not rewrite the new view's URL
    q = search.value.trim(); history.replaceState(null, '', urlFor()); load();
  }, 180));

  const sortSel = h('select', { 'aria-label': 'Sort', onchange: () => { sort = sortSel.value; pref.set('sort', sort); load(); } },
    ...[['genre', 'Shelve by genre'], ['title', 'Title'], ['author', 'Author'], ['added', 'Recently added'], ['published', 'Publication / release date']]
      .map(([v, l]) => h('option', { value: v, selected: v === sort }, l)));
  const statusSel = h('select', { 'aria-label': 'Reading or watching status', onchange: () => { location.hash = urlFor({ status: statusSel.value }); } },
    h('option', { value: '' }, 'Any status'),
    ...['unread', 'reading', 'read'].map((s) => h('option', { value: s, selected: filters.status === s }, `${STATUS_LABELS.book[s]} / ${STATUS_LABELS.movie[s].toLowerCase()}`)));
  const mediaSel = h('select', { 'aria-label': 'Media type', onchange: () => { location.hash = urlFor({ media: mediaSel.value, genre: '', subgenre: '', tag: '' }); } },
    ...MEDIA_FILTERS.map(([v, l]) => h('option', { value: v, selected: filters.media === v }, l)));
  // Genre / subgenre / tag dropdowns. A selection coming from the URL stays listed even if it has no items here.
  const merged = new Map();
  for (const g of filters.media === 'book' ? fx.genres : filters.media ? fx.movieGenres : [...fx.genres, ...fx.movieGenres]) {
    const m = merged.get(g.genre) ?? { count: 0, subs: new Map() };
    m.count += g.count;
    for (const s of g.subgenres) m.subs.set(s.name, (m.subs.get(s.name) ?? 0) + s.count);
    merged.set(g.genre, m);
  }
  if (filters.genre && !merged.has(filters.genre)) merged.set(filters.genre, { count: 0, subs: new Map() });
  const genreNames = [...merged.keys()].sort((a, b) => (a === 'Uncategorized') - (b === 'Uncategorized') || a.localeCompare(b));
  const genreSel = h('select', { 'aria-label': 'Genre', onchange: () => { location.hash = urlFor({ genre: genreSel.value, subgenre: '' }); } },
    h('option', { value: '' }, 'Any genre'),
    ...genreNames.map((n) => h('option', { value: n, selected: filters.genre === n }, `${n} · ${merged.get(n).count}`)));
  const subs = filters.genre ? [...merged.get(filters.genre).subs.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])) : [];
  if (filters.subgenre && !subs.some(([n]) => n === filters.subgenre)) subs.push([filters.subgenre, 0]);
  const subSel = subs.length ? h('select', { 'aria-label': 'Subgenre', onchange: () => { location.hash = urlFor({ subgenre: subSel.value }); } },
    h('option', { value: '' }, 'Any subgenre'),
    ...subs.map(([n, c]) => h('option', { value: n, selected: filters.subgenre === n }, `${n} · ${c}`))) : null;
  const tagList = fx.tags.slice(0, 120);
  if (filters.tag && !tagList.some((t) => t.name === filters.tag)) tagList.push({ name: filters.tag, count: 0 });
  const tagSel = h('select', { 'aria-label': 'Tag', onchange: () => { location.hash = urlFor({ tag: tagSel.value }); } },
    h('option', { value: '' }, 'Any tag'),
    ...tagList.map((t) => h('option', { value: t.name, selected: filters.tag === t.name }, `${t.name} · ${t.count}`)));
  const viewBtn = h('button', {
    class: 'btn small', type: 'button', 'aria-label': 'Toggle grid or list',
    onclick: () => { view = view === 'grid' ? 'list' : 'grid'; pref.set('view', view); viewBtn.textContent = view === 'grid' ? 'List' : 'Grid'; draw(); },
  }, view === 'grid' ? 'List' : 'Grid');

  let books = [];
  const bookCard = (b) => h('button', { class: 'book', type: 'button', onclick: () => openBook(b), 'aria-label': `${b.title} by ${authorsText(b)}` },
    h('div', { class: 'cover' }, coverEl(b), b.status !== 'unread' ? h('span', { class: 'badge' }, STATUS_LABELS[isMovie(b) ? 'movie' : 'book'][b.status]) : null,
      b.needs_review ? h('span', { class: 'badge review' }, 'review') : null, isMovie(b) ? h('span', { class: 'badge fmt' }, FORMAT_LABEL[b.media] ?? b.media) : null),
    h('div', { class: 'meta' }, h('div', { class: 't' }, b.title), h('div', { class: 'a' }, isMovie(b) && b.published ? `${authorsText(b)} · ${b.published.slice(0, 4)}` : authorsText(b)),
      view === 'list' ? withHue(h('span', { class: 'pill genre' }, b.subgenre ? `${b.genre} · ${b.subgenre}` : b.genre), b.genre) : null));

  function draw() {
    if (!books.length) {
      const nothing = state.facets.total === 0 && !q && !Object.values(filters).some(Boolean);
      list.replaceChildren(h('div', { class: 'empty' },
        h('p', {}, nothing ? 'Your library is empty.' : 'Nothing matches.'),
        nothing ? h('a', { class: 'btn primary', href: '#/scan' }, 'Scan your first book or movie') : null,
        q ? h('a', { class: 'btn primary', href: `#/scan?title=${encodeURIComponent(q)}` }, `Search online for “${q}”`) : null,
        !nothing ? h('a', { class: 'btn', href: '#/library' }, 'Clear search and filters') : null));
      return;
    }
    const grid = () => h('div', { class: `books${view === 'list' ? ' list' : ''}` });
    if (sort !== 'genre' || q) { const g = grid(); g.append(...books.map(bookCard)); list.replaceChildren(g); return; }

    // Shelf view: genre -> subgenre groups, in the server's shelf order.
    const out = [];
    const shelfKey = (x) => `${isMovie(x) ? 'm' : 'b'}:${x.genre}`; // a film's "Horror" is a different shelf from a novel's
    let curGenre = null, curSub = null, shelf = null, hasSubs = false;
    for (const b of books) {
      if (shelfKey(b) !== curGenre) {
        curGenre = shelfKey(b); curSub = null;
        const inGenre = books.filter((x) => shelfKey(x) === curGenre);
        hasSubs = inGenre.some((x) => x.subgenre);
        out.push(withHue(h('div', { class: 'shelf-head' }, h('h2', {}, b.genre), isMovie(b) ? h('span', { class: 'pill' }, 'Movies') : null,
          h('span', { class: 'n' }, `${inGenre.length} ${itemNoun(inGenre.length, isMovie(b))}`)), b.genre));
        shelf = null;
      }
      if (shelf === null || b.subgenre !== curSub) {
        curSub = b.subgenre;
        if (hasSubs) out.push(h('div', { class: 'subshelf' }, b.subgenre || 'Other'));
        shelf = grid(); out.push(shelf);
      }
      shelf.append(bookCard(b));
    }
    list.replaceChildren(...out);
  }

  async function load() {
    const mine = ++seq;
    const p = new URLSearchParams({ sort });
    if (q) { p.set('q', q); p.set('sort', 'relevance'); }
    for (const k of ['media', 'genre', 'subgenre', 'tag', 'status', 'location']) if (filters[k]) p.set(k, filters[k]);
    if (filters.review) p.set('review', '1');
    try {
      const r = await api(`/api/books?${p}`);
      if (mine !== seq) return; // a newer keystroke already replaced this request
      books = r.books;
      count.textContent = `${r.total} ${r.total === 1 ? 'item' : 'items'}${r.total > books.length ? ` (showing ${books.length})` : ''}`;
      draw();
    } catch (e) { if (mine === seq) list.replaceChildren(h('div', { class: 'notice err' }, e.message)); }
  }

  const active = [['media', filters.media && (MEDIA_FILTERS.find(([v]) => v === filters.media)?.[1] ?? filters.media)], ['genre', filters.genre], ['subgenre', filters.subgenre], ['tag', filters.tag && `#${filters.tag}`],
    ['location', filters.location && `📍 ${filters.location}`], ['review', filters.review && 'Needs review']].filter(([, v]) => v);
  chips.replaceChildren(...active.map(([k, v]) => h('a', { class: 'chip active', href: urlFor({ [k]: '' }), 'aria-label': `Remove filter ${v}` }, v, ' ✕')));

  add(page, 
    h('div', { class: 'topbar' }, h('h1', {}, 'Library'), h('span', { class: 'row' },
      h('a', { class: 'btn small primary', href: '#/scan?find=1' }, 'Find a book'),
      h('button', { class: 'btn small', type: 'button', onclick: () => openBook(null) }, '+ Book'),
      h('button', { class: 'btn small', type: 'button', onclick: () => openBook(null, { media: 'dvd' }) }, '+ Movie'))),
    h('div', { class: 'searchbar' }, search),
    h('div', { class: 'filters' }, mediaSel, sortSel, statusSel, viewBtn, count),
    h('div', { class: 'filters' }, genreSel, subSel, tagSel),
    chips, list,
  );
  const onChange = () => { if (document.body.contains(list)) { loadFacets(true).then(load); } else window.removeEventListener('library-changed', onChange); };
  window.addEventListener('library-changed', onChange);
  load();
}

// ------------------------------------------------------------------- movies
// Every disc you own plus every movie on Jellyfin, as one list: "Disc", "Server" or both.
const AVAIL = [['', 'All'], ['both', 'Disc + server'], ['disc', 'Disc only'], ['server', 'Server only']];

/** A missing or broken poster falls back to the generated cover instead of a broken-image icon. */
function moviePoster(e) {
  const fake = () => withHue(h('div', { class: 'fake-cover' }, e.title), e.genre);
  if (!e.poster) return fake();
  const img = h('img', { src: e.poster, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer' });
  img.addEventListener('error', () => img.replaceWith(fake()), { once: true });
  return img;
}

async function viewMovies(page, params) {
  let avail = params.get('show') || '';
  let genre = params.get('genre') || '';
  let tag = params.get('tag') || '';
  let q = '';
  let sort = pref.get('movieSort', 'title');
  let data = null;
  const sync = () => {   // keep the filters in the address so a refresh or a shared link shows the same view
    const p = new URLSearchParams(Object.entries({ show: avail, genre, tag }).filter(([, v]) => v));
    history.replaceState(null, '', `#/movies${p.toString() ? `?${p}` : ''}`);
  };

  const grid = h('div', { 'aria-live': 'polite' });
  const status = h('div');
  const chips = h('div', { class: 'chips' });
  const search = h('input', { type: 'search', placeholder: 'Search title, director, genre…', autocomplete: 'off', 'aria-label': 'Search movies' });
  const sortSel = h('select', { 'aria-label': 'Sort', onchange: () => { sort = sortSel.value; pref.set('movieSort', sort); draw(); } },
    ...[['title', 'Title'], ['year', 'Newest first'], ['genre', 'Shelve by genre']].map(([v, l]) => h('option', { value: v, selected: v === sort }, l)));
  const genreSel = h('select', { 'aria-label': 'Genre', onchange: () => { genre = genreSel.value; sync(); draw(); } });
  const tagSel = h('select', { 'aria-label': 'Tag', onchange: () => { tag = tagSel.value; sync(); draw(); } });
  const refreshBtn = h('button', { class: 'btn small', type: 'button', onclick: () => load(true) }, 'Refresh server');
  search.addEventListener('input', debounce(() => { q = search.value.trim().toLowerCase(); draw(); }, 120));

  const card = (e) => h('button', { class: 'book', type: 'button', onclick: () => openMovie(e, load, data.jellyfin), 'aria-label': `${e.title}${e.year ? ` (${e.year})` : ''}` },
    h('div', { class: 'cover' },
      moviePoster(e),
      e.copies.length ? h('span', { class: 'badge fmt' }, [...new Set(e.copies.map((c) => FORMAT_LABEL[c.media] ?? c.media))].join(' + ')) : null,
      e.jellyfin ? h('span', { class: 'badge srv' }, 'Server') : null),
    h('div', { class: 'meta' }, h('div', { class: 't' }, e.title), h('div', { class: 'a' }, [e.directors[0], e.year].filter(Boolean).join(' · ') || '\u00a0')));

  function draw() {
    const all = data.items;
    const counts = Object.fromEntries(AVAIL.map(([v]) => [v, v ? all.filter((e) => e.availability === v).length : all.length]));
    chips.replaceChildren(...AVAIL.filter(([v]) => !v || data.jellyfin.enabled).map(([v, l]) => h('button', {
      class: `chip${avail === v ? ' active' : ''}`, type: 'button', onclick: () => { avail = v; sync(); draw(); },
    }, `${l} · ${counts[v]}`)));

    // Genre and tag options are counted among the films that match the other filters (availability + search),
    // so each choice shows what picking it would leave. The current pick always stays listed.
    const matchesBase = (e) => (!avail || e.availability === avail)
      && (!q || `${e.title} ${e.directors.join(' ')} ${e.genre} ${e.subgenre} ${e.tags.join(' ')} ${e.year}`.toLowerCase().includes(q));
    const base = all.filter(matchesBase);
    // Cross-faceted: genre counts respect the chosen tag, and tag counts respect the chosen genre.
    const tally = (list, pick) => { const m = new Map(); for (const e of list) for (const k of pick(e)) m.set(k, (m.get(k) ?? 0) + 1); return m; };
    const gCounts = tally(base.filter((e) => !tag || e.tags.includes(tag)), (e) => [e.genre]);
    const tCounts = tally(base.filter((e) => !genre || e.genre === genre), (e) => e.tags);
    if (genre && !gCounts.has(genre)) gCounts.set(genre, 0);
    if (tag && !tCounts.has(tag)) tCounts.set(tag, 0);
    genreSel.replaceChildren(h('option', { value: '' }, 'Any genre'),
      ...[...gCounts.keys()].sort((a, b) => (a === 'Uncategorized') - (b === 'Uncategorized') || a.localeCompare(b))
        .map((n) => h('option', { value: n, selected: n === genre }, `${n} · ${gCounts.get(n)}`)));
    tagSel.replaceChildren(h('option', { value: '' }, 'Any tag'),
      ...[...tCounts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 120)
        .map(([n, c]) => h('option', { value: n, selected: n === tag }, `${n} · ${c}`)));
    genreSel.value = genre; tagSel.value = tag;

    let rows = base.filter((e) => (!genre || e.genre === genre) && (!tag || e.tags.includes(tag)));
    if (sort === 'year') rows = [...rows].sort((a, b) => (b.year || '0').localeCompare(a.year || '0') || a.title.localeCompare(b.title));
    if (!rows.length) {
      grid.replaceChildren(h('div', { class: 'empty' }, all.length ? 'No movies match.' : 'No movies yet.', h('br'),
        all.length ? null : h('a', { class: 'btn primary', href: '#/scan' }, 'Scan a DVD or Blu-ray')));
      return;
    }
    if (sort !== 'genre') { grid.replaceChildren(h('div', { class: 'books' }, rows.map(card))); return; }
    const groups = new Map();
    for (const e of rows) groups.set(e.genre, [...(groups.get(e.genre) ?? []), e]);
    const names = [...groups.keys()].sort((a, b) => (a === 'Uncategorized') - (b === 'Uncategorized') || a.localeCompare(b));
    grid.replaceChildren(...names.flatMap((g) => [
      withHue(h('div', { class: 'shelf-head' }, h('h2', {}, g), h('span', { class: 'n' }, `${groups.get(g).length} ${itemNoun(groups.get(g).length, true)}`)), g),
      h('div', { class: 'books' }, groups.get(g).map(card)),
    ]));
  }

  async function load(refresh = false) {
    try {
      refreshBtn.disabled = true;
      data = await api(`/api/movies${refresh ? '?refresh=1' : ''}`);
      const j = data.jellyfin;
      status.replaceChildren(...[
        j.ok && j.libraries?.length ? h('p', { class: 'muted' }, `From your Jellyfin “${j.libraries.join('”, “')}” ${j.libraries.length === 1 ? 'library' : 'libraries'}.`) : null,
        !j.enabled ? h('div', { class: 'notice' }, 'Jellyfin isn’t connected, so this shows only the discs you own. To add your media server, set JELLYFIN_API_KEY on the server (see .env.example).') : null,
        j.enabled && !j.ok ? h('div', { class: 'notice err', role: 'alert' }, `Couldn’t read Jellyfin: ${j.error} Showing your discs only.`) : null].filter(Boolean));
      refreshBtn.classList.toggle('hidden', !j.enabled);
      draw();
    } catch (e) { grid.replaceChildren(h('div', { class: 'notice err' }, e.message)); }
    refreshBtn.disabled = false;
  }

  add(page,
    h('div', { class: 'topbar' }, h('h1', {}, 'Movies'), refreshBtn),
    h('div', { class: 'searchbar' }, search),
    h('div', { class: 'filters' }, sortSel, genreSel, tagSel), chips, status, grid);
  const onChange = () => { if (document.body.contains(grid)) load(); else window.removeEventListener('library-changed', onChange); };
  window.addEventListener('library-changed', onChange);
  await load();
}

/** Details for one film: where you have it, and shortcuts to edit the disc or open it in Jellyfin. */
async function openMovie(e, reload, info = {}) {
  const sub = [e.directors.join(', '), e.year, e.runtime ? `${e.runtime} min` : ''].filter(Boolean).join(' · ');
  const editCopy = async (c) => {
    try { const book = await api(`/api/books/${c.id}`); sheet.close(); openBook(book); } catch (ex) { toast(ex.message, { error: true }); }
  };
  const link = e.jellyfin && info.publicUrl ? `${info.publicUrl}/web/#/details?id=${e.jellyfin.id}` : '';
  sheet.replaceChildren(h('div', { class: 'sheet' },
    h('header', {}, h('h2', {}, e.title), h('button', { class: 'btn small', type: 'button', onclick: () => sheet.close(), 'aria-label': 'Close' }, '✕')),
    h('div', { class: 'body' },
      h('div', { class: 'row' },
        h('div', { class: 'detail-cover' }, moviePoster(e)),
        h('div', { class: 'grow' }, h('p', { class: 'muted' }, sub),
          h('div', { class: 'row' }, withHue(h('span', { class: 'pill genre' }, e.subgenre ? `${e.genre} · ${e.subgenre}` : e.genre), e.genre),
            ...e.copies.map((c) => h('span', { class: 'pill ok' }, FORMAT_LABEL[c.media] ?? c.media)),
            e.jellyfin ? h('span', { class: 'pill' }, 'On Jellyfin') : null))),
      e.overview ? h('p', {}, e.overview) : null,
      !e.copies.length ? h('p', { class: 'muted' }, 'You don’t have this one on disc (or it isn’t in your library yet).') : null,
      !e.jellyfin ? h('p', { class: 'muted' }, 'Not found on your Jellyfin server.') : null),
    h('footer', {},
      ...e.copies.map((c) => h('button', { class: 'btn', type: 'button', onclick: () => editCopy(c) }, `Edit ${FORMAT_LABEL[c.media] ?? c.media}`)),
      !e.copies.length ? h('button', { class: 'btn primary', type: 'button', onclick: () => {
        sheet.close();
        openBook(null, { media: 'dvd', prefill: { title: e.title, authors: e.directors, published: e.year, runtime: e.runtime, description: e.overview }, onSaved: reload });
      } }, 'I own this on disc') : null,
      link ? h('a', { class: 'btn', href: link, target: '_blank', rel: 'noopener noreferrer' }, 'Open in Jellyfin') : null,
      h('span', { class: 'spacer' }), h('button', { class: 'btn', type: 'button', onclick: () => sheet.close() }, 'Close'))));
  sheet.showModal();
}

// ------------------------------------------------------------------- browse
async function viewBrowse(page) {
  const f = state.facets;
  add(page, h('div', { class: 'topbar' }, h('h1', {}, 'Browse')));
  if (!f.total) { add(page, h('div', { class: 'empty' }, 'Nothing to browse yet.', h('br'), h('a', { class: 'btn primary', href: '#/scan' }, 'Scan a book or movie'))); return; }
  const link = (params, text, cls = 'chip') => h('a', { class: cls, href: `#/library?${new URLSearchParams(params)}` }, text);

  if (f.needsReview) {
    add(page, h('div', { class: 'notice warn' }, `${f.needsReview} ${f.needsReview === 1 ? 'item was' : 'items were'} hard to classify. `, link({ review: 1 }, 'Review them', 'chip')));
  }
  const genreSection = (title, genres, media) => genres.length ? h('section', { class: 'block' }, h('h2', {}, title),
    h('div', { class: 'genre-list' }, genres.map((g) => withHue(h('article', { class: 'genre-card' },
      h('header', {}, link({ media, genre: g.genre }, g.genre, ''), h('span', { class: 'n' }, g.count)),
      g.subgenres.length ? h('div', { class: 'chips' }, g.subgenres.map((s) => link({ media, genre: g.genre, subgenre: s.name }, `${s.name} · ${s.count}`))) : null), g.genre)))) : null;
  const both = f.genres.length && f.movieGenres.length;
  add(page, genreSection(both ? 'Book genres' : 'Genres', f.genres, 'book'), genreSection(both ? 'Movie genres' : 'Genres', f.movieGenres, 'movie'));
  if (f.tags.length) add(page, h('section', { class: 'block' }, h('h2', {}, 'Tags'),
    h('div', { class: 'chips' }, f.tags.slice(0, 60).map((t) => link({ tag: t.name }, `${t.name} · ${t.count}`)))));
  if (f.locations.length) add(page, h('section', { class: 'block' }, h('h2', {}, 'Locations'),
    h('div', { class: 'chips' }, f.locations.map((l) => link({ location: l.name }, `${l.name} · ${l.count}`)))));
}

// --------------------------------------------------------------------- more
async function viewMore(page) {
  const f = state.facets;
  const by = Object.fromEntries(f.statuses.map((s) => [s.name, s.count]));
  const stat = (n, label) => h('div', { class: 'stat' }, h('b', {}, n ?? 0), label);
  add(page, 
    h('div', { class: 'topbar' }, h('h1', {}, 'More')),
    h('div', { class: 'stats' }, stat(f.media.book, 'books'), stat(f.media.dvd + f.media.bluray, 'movies'), stat(f.genres.length + f.movieGenres.length, 'genres'), stat(by.unread, 'to read / watch')),
    h('section', { class: 'block' }, h('h2', {}, 'Genre detection'),
      h('p', { class: 'muted' }, 'Re-runs automatic genre and tag detection on every book and movie, using the metadata already saved. Genres and tags you edited yourself are left alone.'),
      h('button', {
        class: 'btn', type: 'button', onclick: async (e) => {
          e.target.disabled = true;
          try { const r = await api('/api/reclassify', { method: 'POST', body: {} }); toast(`Re-checked ${r.total} items, ${r.changed} changed.`); await loadFacets(true); }
          catch (ex) { toast(ex.message, { error: true }); }
          e.target.disabled = false;
        },
      }, 'Re-run genre detection')),
    h('section', { class: 'block' }, h('h2', {}, 'Back up your data'),
      h('p', { class: 'muted' }, 'Download a copy of your catalogue. (To back up cover images too, copy the server’s data folder.)'),
      h('div', { class: 'row' }, h('a', { class: 'btn', href: '/api/export.csv', download: '' }, 'Download CSV'), h('a', { class: 'btn', href: '/api/export.json', download: '' }, 'Download JSON'))),
    h('section', { class: 'block' }, h('h2', {}, 'This device'),
      h('p', { class: 'muted' }, window.isSecureContext ? 'Secure connection: camera scanning is available.' : 'Not a secure (HTTPS) connection: the camera is blocked by your browser. Typing an ISBN or barcode still works.')),
    state.session.authRequired ? h('section', { class: 'block' }, h('button', {
      class: 'btn', type: 'button',
      onclick: async () => { await api('/api/logout', { method: 'POST', body: {} }); state.session = null; route(); },
    }, 'Sign out')) : null,
  );
}

// --------------------------------------------------------------------- scan
/** Shrink a phone photo (often 12 MP / 5 MB) to a 1800 px JPEG: plenty to read a cover, fast to upload. */
async function prepareCoverImage(file) {
  const bmp = await createImageBitmap(file);   // applies the photo's rotation (EXIF) so the cover isn't sideways
  const scale = Math.min(1, 1800 / Math.max(bmp.width, bmp.height));
  const canvas = document.createElement('canvas');
  canvas.width = Math.round(bmp.width * scale); canvas.height = Math.round(bmp.height * scale);
  canvas.getContext('2d').drawImage(bmp, 0, 0, canvas.width, canvas.height);
  bmp.close?.();
  const blob = await new Promise((resolve) => canvas.toBlob(resolve, 'image/jpeg', 0.88));
  if (!blob) throw new Error('Couldn’t process that photo.');
  return blob;
}

async function viewScan(page, params = new URLSearchParams()) {
  const problem = cameraProblem();
  const recent = [];
  let autoAdd = pref.get('autoAdd', false);
  let busy = false;

  const video = h('video', { playsinline: true, muted: true, 'aria-label': 'Camera preview' });
  const overlay = h('div', { class: 'overlay' }, problem ? '' : 'Camera is off');
  const status = h('p', { class: 'statusline', role: 'status', 'aria-live': 'polite' });
  const result = h('div', { 'aria-live': 'polite' });
  const recentList = h('ul', { class: 'recent' });
  const recentHeading = h('h2', { class: 'hidden' }, 'Added this session');
  const startBtn = h('button', { class: 'btn primary', type: 'button' }, 'Start camera');
  const torchBtn = h('button', { class: 'btn hidden', type: 'button' }, 'Light');
  // One photo from the phone's own camera app instead of a live video feed. Works on plain HTTP too.
  const photoScanBtn = h('button', { class: 'btn', type: 'button', onclick: async () => {
    const file = await pickPhoto({ capture: true });
    if (!file) return;
    setStatus('Reading the barcode from your photo…');
    try {
      const isbn = await decodeIsbnFromImage(file);
      if (!isbn) { setStatus('No barcode found in that photo. Get the whole barcode in frame, flat and in good light, or type the ISBN below.'); return; }
      busy = false; handle(isbn);
    } catch (e) { setStatus(`Couldn’t read that photo (${e.message}). Try again, or type the ISBN below.`); }
  } }, 'Scan from photo');
  const locInput = h('input', { type: 'text', list: 'loc-list', placeholder: 'e.g. Living room, shelf 2', value: pref.get('scanLocation', ''), maxlength: '100', id: 'scan-loc' });
  locInput.addEventListener('input', () => pref.set('scanLocation', locInput.value));
  const isbnInput = h('input', { type: 'text', inputmode: 'numeric', autocomplete: 'off', placeholder: 'ISBN or barcode digits', id: 'isbn-in', 'aria-label': 'ISBN or barcode' });
  const auto = h('input', { type: 'checkbox', checked: autoAdd, id: 'auto', onchange: () => { autoAdd = auto.checked; pref.set('autoAdd', autoAdd); } });

  const setStatus = (m) => { status.textContent = m; };
  const next = () => { result.replaceChildren(); busy = false; activeScanner?.resume(); };
  const nextBtn = (label = 'Scan next') => h('button', { class: 'btn', type: 'button', onclick: next }, label);

  function drawRecent() {
    recentHeading.classList.toggle('hidden', !recent.length);
    recentList.replaceChildren(...recent.map((r) => h('li', {},
      h('span', { class: 'grow' }, h('b', {}, r.book.title), ` — ${authorsText(r.book)}`, isMovie(r.book) ? ` (${FORMAT_LABEL[r.book.media]})` : ''),
      withHue(h('span', { class: 'pill genre' }, r.book.genre), r.book.genre),
      h('button', {
        class: 'btn small', type: 'button', onclick: async () => {
          try { await api(`/api/books/${r.book.id}`, { method: 'DELETE' }); recent.splice(recent.indexOf(r), 1); drawRecent(); changed(); toast('Removed'); }
          catch (e) { toast(e.message, { error: true }); }
        },
      }, 'Undo'))));
  }

  /** `code` is an ISBN for books, or a retail barcode for DVDs/Blu-rays (`media` says which). */
  async function addBook(code, media, edited) {
    try {
      const id = media === 'book' ? { isbn: code } : { barcode: code, media };
      const book = await api('/api/books', { method: 'POST', body: { ...id, location: locInput.value.trim(), ...(edited ?? {}) } });
      recent.unshift({ book }); drawRecent(); changed();
      toast(`Added “${book.title}”`);
      return book;
    } catch (e) {
      if (e.status === 409) { toast('Already in your library'); return null; }
      throw e;
    }
  }

  function preview(meta, code) {
    const movie = isMovie(meta);
    // Listings often don't say DVD vs Blu-ray, so let the user confirm the format before adding.
    const fmt = movie ? h('select', { 'aria-label': 'Disc format' }, ...Object.entries(FORMAT_LABEL).map(([v, l]) => h('option', { value: v, selected: v === meta.media }, l))) : null;
    const addNow = h('button', { class: 'btn primary', type: 'button', onclick: async (e) => {
      e.target.disabled = true;
      try { await addBook(code, movie ? fmt.value : 'book'); next(); } catch (ex) { toast(ex.message, { error: true }); e.target.disabled = false; }
    } }, 'Add to library');
    result.replaceChildren(h('div', { class: 'card' },
      h('div', { class: 'preview' },
        h('div', { class: 'cover' }, meta.cover_url ? h('img', { src: meta.cover_url, alt: '', referrerpolicy: 'no-referrer' }) : withHue(h('div', { class: 'fake-cover' }, meta.title), meta.genre)),
        h('div', { class: 'info' },
          h('h2', {}, meta.title), h('div', { class: 'muted' }, movie && meta.published ? `${authorsText(meta)} · ${meta.published.slice(0, 4)}` : authorsText(meta)),
          movie ? h('div', { class: 'row' }, fmt, meta.format_guess === 'guess' ? h('span', { class: 'muted' }, 'Format is a guess: please check.') : null) : null,
          h('div', { class: 'row' }, withHue(h('span', { class: 'pill genre' }, meta.subgenre ? `${meta.genre} · ${meta.subgenre}` : meta.genre), meta.genre),
            meta.needs_review ? h('span', { class: 'pill warn' }, 'check genre') : null),
          h('div', { class: 'chips' }, (meta.tags ?? []).slice(0, 6).map((t) => h('span', { class: 'chip' }, t))))),
      h('div', { class: 'row' }, addNow,
        h('button', { class: 'btn', type: 'button', onclick: () => openBook(null, { ...(movie ? { barcode: code, media: fmt.value } : { isbn: code }), prefill: meta, location: locInput.value, onSaved: next }) }, 'Edit first'),
        nextBtn('Skip'))));
  }

  /** No title found. Offer to keep whatever the databases did know, plus a photo of the cover as the picture. */
  function showNoMatch(isbn, partial) {
    const known = partial ? [
      partial.authors?.length && `by ${partial.authors.join(', ')}`, partial.publisher && `publisher ${partial.publisher}`,
      partial.published && `published ${partial.published}`, partial.pages && `${partial.pages} pages`,
      partial.subjects?.length && `${partial.subjects.length} subject tags`,
    ].filter(Boolean) : [];
    const openForm = (photo) => openBook(null, { isbn, prefill: partial ?? undefined, photo, location: locInput.value, onSaved: next });
    result.replaceChildren(h('div', { class: 'notice' },
      h('b', {}, `No match for ISBN ${isbn}.`),
      h('p', { class: 'muted' }, known.length
        ? `The databases only knew: ${known.join(' · ')}. That will be saved with the book.`
        : 'The databases have nothing for this ISBN.'),
      h('p', { class: 'muted' }, 'Photograph the cover to use as its picture, then type the title.'),
      h('div', { class: 'row' },
        h('button', { class: 'btn primary', type: 'button', onclick: async () => {
          const file = await pickPhoto({ capture: true });
          if (!file) return;
          try { openForm(await shrinkImage(file)); } catch (ex) { toast(ex.message, { error: true }); }
        } }, 'Take cover photo & add'),
        h('button', { class: 'btn', type: 'button', onclick: () => openForm(null) }, 'Add without photo'),
        nextBtn('Skip'))));
  }

  async function handle(isbn, { forcePreview = false } = {}) {
    if (busy) return;
    busy = true;
    activeScanner?.pause();
    setStatus(`Looking up ${isbn}…`);
    result.replaceChildren(h('p', { class: 'muted' }, 'Looking up…'));
    const reveal = () => { if (result.firstChild) result.scrollIntoView({ block: 'nearest', behavior: 'smooth' }); };
    reveal();
    try {
      const r = await api(`/api/lookup/${isbn}`);
      const kind = r.media === 'book' ? 'book' : 'movie';
      setStatus('');
      if (r.existing) {
        result.replaceChildren(h('div', { class: 'notice warn' }, h('b', {}, 'Already in your library: '), r.existing.title, h('div', { class: 'row' },
          h('button', { class: 'btn small', type: 'button', onclick: () => openBook(r.existing) }, 'View'), nextBtn())));
        if (autoAdd) setTimeout(() => { if (busy && activeScanner) next(); }, 1800);
      } else if (!r.found) {
        if (kind === 'book') showNoMatch(r.code, r.partial);
        else result.replaceChildren(h('div', { class: 'notice' }, `No match found for barcode ${r.code}.`, h('div', { class: 'row' },
          h('button', { class: 'btn primary', type: 'button', onclick: () => openBook(null, { barcode: r.code, media: 'dvd', location: locInput.value, onSaved: next }) }, 'Add it manually'), nextBtn('Skip'))));
      } else if (autoAdd && !forcePreview) {
        const book = await addBook(r.code, r.media);
        setStatus(book ? `Added “${book.title}”. Next…` : '');
        setTimeout(() => { if (busy) next(); }, 1000);
      } else preview(r.book, r.code);
    } catch (e) {
      setStatus('');
      result.replaceChildren(h('div', { class: 'notice err', role: 'alert' }, e.message,
        whyFailed(e) ? h('p', { class: 'muted' }, whyFailed(e)) : null, h('div', { class: 'row' },
        h('button', { class: 'btn small', type: 'button', onclick: () => { busy = false; handle(isbn); } }, 'Try again'), nextBtn('Cancel'))));
    }
    reveal();
  }

  // ---- find by title / author
  const titleInput = h('input', { type: 'search', id: 'title-in', autocomplete: 'off', enterkeyhint: 'search', maxlength: '200',
    placeholder: 'Title or author, e.g. “dune herbert”', 'aria-label': 'Search for a book by title or author', value: params.get('title') ?? '' });
  const findBtn = h('button', { class: 'btn', type: 'submit' }, 'Search');
  const findResults = h('div', { 'aria-live': 'polite' });
  let findSeq = 0;

  /** Pick a search result: already owned -> open it; has an ISBN -> the normal preview; otherwise a pre-filled form. */
  async function choose(r) {
    if (r.in_library) {
      try { openBook(await api(`/api/books/${r.in_library}`)); } catch (e) { toast(e.message, { error: true }); }
    } else if (r.isbn13) {
      busy = false; handle(r.isbn13, { forcePreview: true });
    } else {
      openBook(null, { prefill: { title: r.title, subtitle: r.subtitle, authors: r.authors, publisher: r.publisher, published: r.published },
        coverUrl: r.cover_url, location: locInput.value });
    }
  }

  const resultItem = (r) => h('li', { class: 'result' },
    h('div', { class: 'cover' }, r.cover_url
      ? h('img', { src: r.cover_url, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer' })
      : withHue(h('div', { class: 'fake-cover' }, r.title), r.title)),
    h('div', { class: 'meta' },
      h('div', { class: 't' }, r.title, r.subtitle ? `: ${r.subtitle}` : ''),
      h('div', { class: 'a' }, [r.authors.join(', '), r.published.slice(0, 4), r.publisher].filter(Boolean).join(' · ') || 'Unknown author'),
      r.in_library ? h('span', { class: 'pill ok' }, 'In your library') : (!r.isbn13 ? h('span', { class: 'pill warn' }, 'no ISBN') : null)),
    h('button', { class: 'btn small', type: 'button', onclick: () => choose(r) }, r.in_library ? 'View' : r.isbn13 ? 'Preview' : 'Add details'));

  function drawFind(q, r) {
    const manual = h('button', { class: 'btn', type: 'button', onclick: () => openBook(null, { prefill: { title: q }, location: locInput.value }) }, 'Add it manually');
    swap(findResults,
      r.provider_errors?.length ? h('div', { class: 'notice warn' }, 'One book database didn’t answer, so this list may be incomplete.') : null,
      r.results.length
        ? [h('ul', { class: 'results' }, r.results.map(resultItem)), h('p', { class: 'muted' }, 'Not the right one? Try the author’s name too, or ', manual)]
        : h('div', { class: 'notice' }, `No books found for “${q}”. Try fewer or different words.`, h('div', { class: 'row' }, manual)));
  }

  async function runFind(raw) {
    const q = raw.replace(/\s+/g, ' ').trim();
    if (q.length < 2) { findResults.replaceChildren(h('p', { class: 'muted' }, 'Type at least 2 characters.')); return; }
    const mine = ++findSeq;
    findBtn.disabled = true;
    findResults.replaceChildren(h('p', { class: 'muted' }, `Searching for “${q}”…`));
    try {
      const r = await api(`/api/search?q=${encodeURIComponent(q)}`);
      if (mine === findSeq) {
        drawFind(q, r);
        // On a phone the camera view fills the screen above this: bring the search box to the top so the results are visible.
        titleInput.scrollIntoView({ block: 'start', behavior: 'smooth' });
      }
    } catch (e) {
      if (mine === findSeq) findResults.replaceChildren(h('div', { class: 'notice err', role: 'alert' }, e.message, whyFailed(e) ? h('p', { class: 'muted' }, whyFailed(e)) : null));
    } finally {
      if (mine === findSeq) findBtn.disabled = false;
    }
  }

  // ---- no barcode: photograph the cover and pick from the matches
  const coverIn = h('input', { type: 'file', accept: 'image/*', capture: 'environment', class: 'hidden', id: 'cover-in', 'aria-label': 'Photo of a book cover' });
  const coverBtn = h('button', { class: 'btn', type: 'button', onclick: () => coverIn.click() }, 'No barcode? Photograph the cover');
  coverIn.addEventListener('change', () => { const file = coverIn.files[0]; coverIn.value = ''; if (file) scanCoverPhoto(file); });

  function showCoverResult(r) {
    const said = r.read.title ? `${r.read.title}${r.read.authors.length ? ` — ${r.read.authors.join(', ')}` : ''}` : r.read.headline;
    const pick = (c) => {
      busy = false;
      if (c.isbn13) handle(c.isbn13);
      else openBook(null, { prefill: { title: c.title, authors: c.authors, published: c.year }, location: locInput.value, onSaved: next });
    };
    const cards = r.candidates.map((c) => h('button', { class: 'find-result', type: 'button', onclick: () => pick(c) },
      c.cover_url ? h('img', { src: c.cover_url, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer' }) : h('div', { class: 'noposter' }),
      h('span', {}, h('b', {}, c.title), c.year ? ` (${c.year})` : '', c.existing ? h('span', { class: 'pill ok' }, 'In library') : null,
        h('span', { class: 'muted block' }, c.authors.join(', ') || 'Unknown author'))));
    result.replaceChildren(h('div', { class: 'card' },
      h('p', { class: 'muted' }, said ? `Read from the cover: “${said.slice(0, 100)}”` : 'Couldn’t read any text from that photo.'),
      r.isbn ? h('div', { class: 'notice' }, `An ISBN is printed on it: ${r.isbn}. `, h('button', { class: 'btn small primary', type: 'button', onclick: () => { busy = false; handle(r.isbn); } }, 'Look it up')) : null,
      cards.length ? h('p', {}, h('b', {}, 'Is it one of these?')) : null,
      h('div', { class: 'find-results' }, cards),
      !cards.length ? h('p', { class: 'muted' }, r.notes.filter((n) => !/Google Books/.test(n)).join(' ') || 'No matching book found.') : null,
      h('div', { class: 'row' },
        h('button', { class: 'btn', type: 'button', onclick: () => openBook(null, { prefill: { title: r.read.title, authors: r.read.authors }, location: locInput.value, onSaved: next }) }, 'None of these: add by hand'),
        nextBtn('Try another photo'))));
  }

  async function scanCoverPhoto(file) {
    if (busy) return;
    busy = true;
    activeScanner?.pause();
    setStatus('');
    result.replaceChildren(h('p', { class: 'muted', role: 'status' }, 'Reading the cover… this can take a few seconds.'));
    try {
      showCoverResult(await api('/api/cover/scan', { method: 'POST', body: await prepareCoverImage(file) }));
    } catch (e) {
      result.replaceChildren(h('div', { class: 'notice err', role: 'alert' }, e.message, h('div', { class: 'row' }, nextBtn('Cancel'))));
    }
  }

  startBtn.addEventListener('click', async () => {
    if (activeScanner?.active) { stopScanner(); overlay.classList.remove('hidden'); overlay.textContent = 'Camera is off'; startBtn.textContent = 'Start camera'; torchBtn.classList.add('hidden'); setStatus(''); return; }
    try {
      activeScanner = createScanner({ video, onCode: handle, onStatus: setStatus });
      await activeScanner.start();
      overlay.classList.add('hidden'); startBtn.textContent = 'Stop camera';
      if (activeScanner.torchSupported()) {
        torchBtn.classList.remove('hidden');
        let on = false;
        torchBtn.onclick = async () => { on = !on; try { await activeScanner.setTorch(on); } catch { toast('Light not available', { error: true }); } };
      }
    } catch (e) {
      stopScanner();
      overlay.textContent = e.name === 'NotAllowedError' ? 'Camera permission was denied. Allow it in your browser’s site settings, or type the ISBN below.' : `Couldn’t start the camera: ${e.message}`;
    }
  });

  const problemText = {
    insecure: 'Phone and tablet browsers only allow the camera on secure (HTTPS) pages. See the README section “Using it from your phone”. You can still use Scan from photo, or type an ISBN or barcode below.',
    unsupported: 'This browser can’t access a live camera. You can still use Scan from photo, or type an ISBN or barcode below.',
  }[problem];

  add(page, 
    h('div', { class: 'topbar' }, h('h1', {}, 'Scan')),
    problem ? h('div', { class: 'notice warn' }, problemText) : h('div', { class: 'viewport' }, video, h('div', { class: 'guide' }), overlay),
    h('div', { class: 'row' }, problem ? null : [startBtn, torchBtn], photoScanBtn),
    status,
    h('div', { class: 'row' },
      h('div', { class: 'field grow' }, h('label', { for: 'scan-loc' }, 'Put these items in (optional)'), locInput),
      h('label', { class: 'row' }, auto, h('span', {}, 'Add automatically when found'))),
    h('form', { class: 'row', onsubmit: (e) => {
      e.preventDefault();
      const digits = isbnInput.value.replace(/[\s-]/g, '');
      if (!/^(\d{9}[\dXx]|\d{12,13})$/.test(digits)) { setStatus('That doesn’t look like an ISBN or barcode: it should be 10, 12 or 13 characters.'); return; }
      isbnInput.value = ''; busy = false; handle(digits);
    } }, h('div', { class: 'grow' }, h('label', { class: 'sr-only', for: 'isbn-in' }, 'ISBN or barcode'), isbnInput), h('button', { class: 'btn', type: 'submit' }, 'Look up')),
    state.taxonomy.coverScan ? h('div', { class: 'row' }, coverBtn, coverIn) : null,
    h('h2', { class: 'section-title' }, 'Or find it by title or author'),
    h('form', { class: 'row', onsubmit: (e) => { e.preventDefault(); runFind(titleInput.value); } },
      h('div', { class: 'grow' }, h('label', { class: 'sr-only', for: 'title-in' }, 'Title or author'), titleInput), findBtn),
    findResults,
    result,
    h('datalist', { id: 'loc-list' }, state.facets.locations.map((l) => h('option', { value: l.name }))),
    recentHeading,
    recentList,
  );
  if (params.get('title')) runFind(params.get('title'));
  else if (params.get('find')) titleInput.focus();
}

// ------------------------------------------------------------- book dialog
/**
 * Edit an existing book/movie, or (book = null) add one.
 * opts: {isbn | barcode, media ('book'|'dvd'|'bluray'), prefill, location, onSaved}
 */
function openBook(book, opts = {}) {
  const creating = !book;
  const p = opts.prefill ?? {};
  const media = book?.media ?? opts.media ?? p.media ?? 'book';
  const movie = media !== 'book';
  const kind = movie ? 'movie' : 'book';
  const b = book ?? {
    title: p.title ?? '', subtitle: p.subtitle ?? '', authors: p.authors ?? [], publisher: p.publisher ?? '', published: p.published ?? '',
    pages: p.pages ?? null, runtime: p.runtime ?? null, barcode: opts.barcode ?? '', description: p.description ?? '', genre: p.genre ?? 'Uncategorized', subgenre: p.subgenre ?? '', genre_locked: false,
    tags: p.tags ?? [], tags_locked: false, status: 'unread', location: opts.location ?? '', notes: '', isbn13: opts.isbn ?? '', needs_review: false,
  };
  const genreList = movie ? state.taxonomy.movieGenres : state.taxonomy.genres;
  const genreNames = genreList.map((g) => g.name);
  const subsOf = (name) => genreList.find((g) => g.name === name)?.subgenres ?? [];
  let tags = [...b.tags];
  let tagsDirty = false;

  const f = {
    title: h('input', { type: 'text', value: b.title, required: true, maxlength: '300', id: 'f-title' }),
    subtitle: h('input', { type: 'text', value: b.subtitle, maxlength: '300', id: 'f-sub' }),
    authors: h('input', { type: 'text', value: b.authors.join('; '), id: 'f-auth', placeholder: movie ? 'Separate several directors with semicolons' : 'Separate several authors with semicolons' }),
    publisher: h('input', { type: 'text', value: b.publisher, id: 'f-pub' }),
    published: h('input', { type: 'text', value: b.published, id: 'f-date' }),
    pages: h('input', { type: 'number', min: '0', value: b.pages ?? '', id: 'f-pages' }),
    runtime: h('input', { type: 'number', min: '0', value: b.runtime ?? '', id: 'f-runtime' }),
    format: movie ? h('select', { id: 'f-format' }, ...Object.entries(FORMAT_LABEL).map(([v, l]) => h('option', { value: v, selected: v === media }, l))) : null,
    isbn: h('input', { type: 'text', value: (movie ? b.barcode : b.isbn13) ?? '', id: 'f-isbn', inputmode: 'numeric', disabled: !creating || !!opts.isbn || !!opts.barcode }),
    location: h('input', { type: 'text', value: b.location, list: 'f-loc-list', id: 'f-loc', maxlength: '100' }),
    notes: h('textarea', { id: 'f-notes', maxlength: '5000' }, ''),
    description: h('textarea', { id: 'f-desc' }, ''),
    subgenre: h('input', { type: 'text', value: b.subgenre, list: 'f-sub-list', id: 'f-subg', maxlength: '60' }),
  };
  f.notes.value = b.notes; f.description.value = b.description;

  const autoLabel = creating && !p.genre ? 'Auto (decide from the title)' : `Auto (detected: ${b.genre})`;
  let detected = b.genre;   // what "Auto" will pick; the title finder updates it
  const genreSel = h('select', { id: 'f-genre' },
    h('option', { value: 'auto', selected: !b.genre_locked }, autoLabel),
    ...[...new Set([...genreNames, 'Uncategorized', ...(b.genre_locked ? [b.genre] : [])])].map((g) => h('option', { value: g, selected: b.genre_locked && b.genre === g }, g)));
  const subList = h('datalist', { id: 'f-sub-list' });
  const refreshSubs = () => subList.replaceChildren(...subsOf(genreSel.value === 'auto' ? detected : genreSel.value).map((s) => h('option', { value: s })));
  genreSel.addEventListener('change', () => { f.subgenre.value = ''; refreshSubs(); });
  refreshSubs();

  const status = b.status;
  const statusRadios = ['unread', 'reading', 'read'].map((s) => h('label', {}, h('input', { type: 'radio', name: 'status', value: s, checked: s === status }), h('span', {}, STATUS_LABELS[kind][s])));

  // tag editor
  const tagBox = h('div', { class: 'tag-edit' });
  const tagInput = h('input', { type: 'text', placeholder: 'Add a tag…', 'aria-label': 'Add a tag', maxlength: '40' });
  const drawTags = () => tagBox.replaceChildren(...tags.map((t) => h('span', { class: 'chip' }, t, ' ',
    h('button', { type: 'button', 'aria-label': `Remove tag ${t}`, onclick: () => { tags = tags.filter((x) => x !== t); tagsDirty = true; drawTags(); } }, '✕'))), tagInput);
  const addTag = () => { const t = tagInput.value.replace(/,/g, '').trim().toLowerCase(); if (t && !tags.includes(t)) { tags.push(t); tagsDirty = true; } tagInput.value = ''; drawTags(); tagInput.focus(); };
  tagInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ',') { e.preventDefault(); addTag(); }
    else if (e.key === 'Backspace' && !tagInput.value && tags.length) { tags.pop(); tagsDirty = true; drawTags(); tagInput.focus(); }
  });
  tagInput.addEventListener('blur', () => { if (tagInput.value.trim()) addTag(); });
  drawTags();

  // "Find this title on TMDB": search, pick, and the form below fills in. The server re-fetches the pick on save.
  let match = null;       // {id, kind} chosen in the finder; sent with the save
  let runFinder = null;
  let finder = null;
  if (movie && state.taxonomy.tmdb) {
    const q = h('input', { type: 'search', id: 'f-find', value: b.title.replace(/\s*[\[(][^\])]*[\])]/g, '').trim(), placeholder: 'Film or series title', enterkeyhint: 'search', autocomplete: 'off' });
    const yr = h('input', { type: 'text', id: 'f-find-year', inputmode: 'numeric', maxlength: '4', placeholder: 'Year', 'aria-label': 'Year (optional)', class: 'year' });
    const out = h('div', { class: 'find-results' });
    const msg = h('p', { class: 'muted', role: 'status' });

    async function pick(r) {
      msg.textContent = 'Loading…';
      try {
        const d = await api(`/api/tmdb/${r.kind}/${r.id}`);
        f.title.value = d.title; f.authors.value = d.authors.join('; '); f.publisher.value = d.publisher; f.published.value = d.published;
        f.runtime.value = d.runtime ?? ''; f.description.value = d.description;
        match = d.tmdb;
        if (!b.genre_locked && genreSel.value === 'auto') {
          detected = d.genre; genreSel.options[0].textContent = `Auto (detected: ${d.genre})`;
          f.subgenre.value = d.subgenre; refreshSubs();
        }
        if (!b.tags_locked && !tagsDirty) { tags = [...d.tags]; drawTags(); }
        out.replaceChildren();
        msg.textContent = `Filled in from “${d.title}”${d.published ? ` (${d.published})` : ''}. Check it, then Save: the poster is added when you save.`;
      } catch (ex) { msg.textContent = ex.message; }
    }
    async function search() {
      const query = q.value.trim();
      if (!query) { msg.textContent = 'Type a title first.'; return; }
      msg.textContent = 'Searching…'; out.replaceChildren();
      try {
        const year = yr.value.trim();
        const r = await api(`/api/tmdb/search?${new URLSearchParams({ q: query, ...(year ? { year } : {}) })}`);
        msg.textContent = r.results.length ? 'Pick the right one:' : 'No matches. Try fewer words, or clear the year.';
        out.replaceChildren(...r.results.map((x) => h('button', { class: 'find-result', type: 'button', onclick: () => pick(x) },
          x.poster ? h('img', { src: x.poster, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer' }) : h('div', { class: 'noposter' }),
          h('span', {}, h('b', {}, x.title), x.year ? ` (${x.year})` : '', x.kind === 'tv' ? h('span', { class: 'pill' }, 'TV') : null,
            h('span', { class: 'muted block' }, x.overview)))));
      } catch (ex) { msg.textContent = ex.message; }
    }
    runFinder = search;
    q.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); search(); } });
    yr.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); search(); } });
    finder = h('details', { class: 'find', open: !!b.needs_review || (creating && !opts.prefill) },
      h('summary', {}, b.needs_review ? 'Genre unsure: find this title on TMDB' : 'Find this title on TMDB'),
      h('div', { class: 'row' }, h('div', { class: 'grow' }, h('label', { class: 'sr-only', for: 'f-find' }, 'Title to search for'), q), yr,
        h('button', { class: 'btn', type: 'button', onclick: search }, 'Search')),
      msg, out);
  }

  const err = h('p', { class: 'notice err hidden', role: 'alert' });
  const showErr = (m) => { err.textContent = m; err.classList.remove('hidden'); err.scrollIntoView({ block: 'nearest' }); };
  const field = (label, input, id) => h('div', { class: 'field' }, h('label', { for: id }, label), input);

  // Cover photo: held until Save, then uploaded after the book exists.
  let photoBlob = opts.photo ?? null;
  let photoUrl = photoBlob ? URL.createObjectURL(photoBlob) : '';
  sheet.addEventListener('close', () => { if (photoUrl) URL.revokeObjectURL(photoUrl); }, { once: true });
  const coverBox = h('div', { class: 'detail-cover' });
  const photoBtns = h('div', { class: 'row' });
  const drawCover = () => coverBox.replaceChildren(photoUrl
    ? h('img', { src: photoUrl, alt: 'New cover photo' })
    : creating && opts.coverUrl ? h('img', { src: opts.coverUrl, alt: 'Cover from the search result', referrerpolicy: 'no-referrer' })
    : creating ? withHue(h('div', { class: 'fake-cover' }, 'No cover yet'), b.genre) : coverEl(book));
  const drawPhotoBtns = () => swap(photoBtns,
    h('button', { class: 'btn small', type: 'button', onclick: async () => setPhoto(await pickPhoto({ capture: true })) }, 'Take cover photo'),
    h('button', { class: 'btn small', type: 'button', onclick: async () => setPhoto(await pickPhoto()) }, 'Choose image'),
    photoBlob ? h('button', { class: 'btn small', type: 'button', onclick: () => { URL.revokeObjectURL(photoUrl); photoBlob = null; photoUrl = ''; drawCover(); drawPhotoBtns(); } }, 'Remove photo') : null);
  async function setPhoto(file) {
    if (!file) return;
    try {
      const blob = await shrinkImage(file);
      if (photoUrl) URL.revokeObjectURL(photoUrl);
      photoBlob = blob; photoUrl = URL.createObjectURL(blob);
      drawCover(); drawPhotoBtns();
    } catch (ex) { showErr(ex.message); }
  }
  drawCover(); drawPhotoBtns();

  async function save(e) {
    e.preventDefault();
    const val = (el) => el.value.trim();
    const patch = {
      title: val(f.title), subtitle: val(f.subtitle), authors: val(f.authors).split(';').map((a) => a.trim()).filter(Boolean),
      publisher: val(f.publisher), published: val(f.published),
      ...(movie
        ? { runtime: val(f.runtime) === '' ? null : Number(val(f.runtime)), media: f.format.value }
        : { pages: val(f.pages) === '' ? null : Number(val(f.pages)) }),
      description: f.description.value.trim(), location: val(f.location), notes: f.notes.value.trim(),
      status: sheet.querySelector('input[name=status]:checked').value,
    };
    const genreChanged = genreSel.value !== (b.genre_locked ? b.genre : 'auto'); // compare with what the dropdown started on
    if (genreChanged) patch.genre = genreSel.value;
    if (genreChanged || val(f.subgenre) !== b.subgenre) patch.subgenre = val(f.subgenre);
    if (tagsDirty) patch.tags = tags;
    if (match) patch.tmdb = match;
    saveBtn.disabled = true;
    try {
      let saved;
      if (creating) {
        const code = val(f.isbn);
        saved = await api('/api/books', { method: 'POST', body: { ...(code ? (movie ? { barcode: code } : { isbn: code }) : {}), ...(opts.coverUrl ? { cover_url: opts.coverUrl } : {}), ...patch } });
        toast(`Added “${saved.title}”`);
      } else {
        saved = await api(`/api/books/${book.id}`, { method: 'PATCH', body: patch });
        toast('Saved');
      }
      if (photoBlob) {
        try { saved = await uploadCover(saved.id, photoBlob); }
        catch (ex) { toast(`Saved, but the cover photo didn’t upload: ${ex.message}`, { error: true, ms: 6000 }); }
      }
      sheet.close(); changed(); opts.onSaved?.(saved);
    } catch (ex) {
      if (ex.status === 409 && ex.data?.existing) { sheet.close(); toast('That is already in your library'); openBook(ex.data.existing); return; }
      showErr(ex.message); saveBtn.disabled = false;
    }
  }

  const saveBtn = h('button', { class: 'btn primary', type: 'submit' }, creating ? 'Add book' : 'Save');
  const footer = h('footer', {}, saveBtn, h('button', { class: 'btn', type: 'button', onclick: () => sheet.close() }, 'Cancel'), h('span', { class: 'spacer' }));
  if (!creating) {
    if (b.isbn13 || b.barcode) footer.append(h('button', { class: 'btn', type: 'button', onclick: async (e) => {
      e.target.disabled = true;
      try { await api(`/api/books/${book.id}/refresh`, { method: 'POST', body: {} }); toast('Metadata refreshed'); sheet.close(); changed(); }
      catch (ex) { showErr(ex.message); e.target.disabled = false; }
    } }, 'Refresh data'));
    footer.append(h('button', { class: 'btn danger', type: 'button', onclick: async () => {
      if (!confirm(`Remove “${b.title}” from your library?`)) return;
      try { await api(`/api/books/${book.id}`, { method: 'DELETE' }); toast('Removed'); sheet.close(); changed(); } catch (ex) { showErr(ex.message); }
    } }, 'Delete'));
  }

  const form = h('form', { class: 'sheet', onsubmit: save },
    h('header', {}, h('h2', {}, creating ? (movie ? 'Add a movie' : 'Add a book') : (movie ? 'Movie details' : 'Book details')), h('button', { class: 'btn small', type: 'button', onclick: () => sheet.close(), 'aria-label': 'Close' }, '✕')),
    h('div', { class: 'body' },
      h('div', { class: 'row' }, coverBox,
        h('div', { class: 'grow' },
          !creating ? [withHue(h('span', { class: 'pill genre' }, b.subgenre ? `${b.genre} · ${b.subgenre}` : b.genre), b.genre), ' ',
            movie ? h('span', { class: 'pill' }, FORMAT_LABEL[b.media] ?? b.media) : null, b.needs_review ? h('span', { class: 'pill warn' }, 'genre unsure') : null, b.genre_locked ? h('span', { class: 'pill ok' }, 'genre set by you') : null] : null,
          photoBtns)),
      err, finder,
      field('Title', f.title, 'f-title'), movie ? null : field('Subtitle', f.subtitle, 'f-sub'), field(movie ? 'Director(s)' : 'Author(s)', f.authors, 'f-auth'),
      movie ? field('Format', f.format, 'f-format') : null,
      h('div', { class: 'grid2' }, field('Genre', genreSel, 'f-genre'), field('Subgenre', f.subgenre, 'f-subg')), subList,
      h('div', { class: 'field' }, h('label', {}, 'Tags'), tagBox,
        !creating && b.tags_locked ? h('button', { class: 'btn small', type: 'button', onclick: async () => {
          try { await api(`/api/books/${book.id}`, { method: 'PATCH', body: { autoTags: true } }); toast('Tags reset to automatic'); sheet.close(); changed(); } catch (ex) { showErr(ex.message); }
        } }, 'Reset to automatic tags') : null),
      h('div', { class: 'field' }, h('span', {}, movie ? 'Watching status' : 'Reading status'), h('div', { class: 'segmented', role: 'radiogroup' }, statusRadios)),
      field('Location', f.location, 'f-loc'),
      h('datalist', { id: 'f-loc-list' }, state.facets.locations.map((l) => h('option', { value: l.name }))),
      field('Notes', f.notes, 'f-notes'),
      h('div', { class: 'grid2' }, field(movie ? 'Studio' : 'Publisher', f.publisher, 'f-pub'), field(movie ? 'Released' : 'Published', f.published, 'f-date'),
        movie ? field('Runtime (min)', f.runtime, 'f-runtime') : field('Pages', f.pages, 'f-pages'), field(movie ? 'Barcode' : 'ISBN', f.isbn, 'f-isbn')),
      h('details', {}, h('summary', {}, 'Description'), f.description)),
    footer);
  sheet.replaceChildren(form);
  sheet.showModal();
  if (creating && !finder) f.title.focus();
  if (finder && !creating && b.needs_review) runFinder();   // a title that needed review: show likely matches straight away
}

// --------------------------------------------------------------------- boot
sheet.addEventListener('close', () => { if (!sheet.open) sheet.replaceChildren(); }); // drop the old form so ids/labels never duplicate
route();
if ('serviceWorker' in navigator) navigator.serviceWorker.register('/sw.js').catch(() => {});
