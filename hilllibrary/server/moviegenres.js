// Genre classification for DVDs / Blu-rays.
//
// Movies are shelved separately from books (a film's "Science Fiction" is not a novel's).
// When TMDB answered we get real genre names, which are far more reliable than anything
// we could guess, so they are mapped onto a small set of shelves. Without TMDB (no API key,
// or no match) a weak keyword guess runs on the title/description and the item is always
// flagged "needs review".
//
// TMDB genre lists: https://developer.themoviedb.org/reference/genre-movie-list and
// https://developer.themoviedb.org/reference/genre-tv-list

export const MOVIE_GENRES = [
  'Action & Adventure', 'Animation', 'Comedy', 'Crime', 'Documentary', 'Drama', 'Family', 'Fantasy',
  'Horror', 'Music & Musical', 'Mystery & Thriller', 'Romance', 'Science Fiction', 'War & History', 'Western',
];
export const MOVIE_UNCATEGORIZED = 'Uncategorized';

const TMDB_TO_SHELF = {
  'action': 'Action & Adventure', 'adventure': 'Action & Adventure', 'action & adventure': 'Action & Adventure',
  'animation': 'Animation', 'comedy': 'Comedy', 'crime': 'Crime', 'documentary': 'Documentary', 'drama': 'Drama',
  'family': 'Family', 'kids': 'Family', 'fantasy': 'Fantasy', 'sci-fi & fantasy': 'Science Fiction',
  'science fiction': 'Science Fiction', 'history': 'War & History', 'war': 'War & History', 'war & politics': 'War & History',
  'horror': 'Horror', 'music': 'Music & Musical', 'mystery': 'Mystery & Thriller', 'thriller': 'Mystery & Thriller',
  'romance': 'Romance', 'western': 'Western',
};

const KEYWORDS = [
  ['Horror', /horror|haunt|zombie|slasher|vampire|demon|possess|nightmare/],
  ['Science Fiction', /sci-?fi|\bspace\b|alien|robot|galaxy|cyborg|time travel|\bplanet\b|starship/],
  ['Animation', /animated|animation|pixar|cartoon/],
  ['Documentary', /documentary|\bdocu|nature series/],
  ['Comedy', /comedy|funny|hilarious|\blaughs?\b|sitcom/],
  ['Action & Adventure', /\baction\b|adventure|explosive|\bheist\b|superhero|\bbattle\b/],
  ['Romance', /romance|romantic|love story|wedding/],
  ['Mystery & Thriller', /thriller|suspense|mystery|detective|\bspy\b|serial killer/],
  ['Crime', /\bcrime\b|gangster|\bmafia\b|\bmob\b|\bpolice\b/],
  ['War & History', /\bwar\b|world war|soldiers?\b|historical/],
  ['Western', /western|cowboy|outlaw|frontier/],
  ['Family', /\bfamily\b|\bkids\b|children/],
  ['Music & Musical', /concert|musical|live in\b/],
  ['Fantasy', /fantasy|wizard|\bmagic\b|dragon/],
  ['Drama', /\bdrama\b/],
];

/**
 * @param {{title?:string, description?:string, genres?:string[], tv?:boolean, year?:string, fourK?:boolean}} input
 *   `genres` are TMDB genre names when a TMDB match was found.
 */
export function classifyMovie(input) {
  const shelves = [];
  for (const g of input.genres ?? []) {
    const s = TMDB_TO_SHELF[String(g).trim().toLowerCase()];
    if (s && !shelves.includes(s)) shelves.push(s);
  }

  let genre = MOVIE_UNCATEGORIZED;
  let subgenre = '';
  let confidence = 'low';

  if (shelves.length) {
    genre = shelves.includes('Animation') ? 'Animation' : shelves.includes('Documentary') ? 'Documentary' : shelves[0];
    subgenre = shelves.find((s) => s !== genre) ?? '';
    confidence = 'high';
  } else {
    const title = String(input.title ?? '').toLowerCase();
    const body = String(input.description ?? '').toLowerCase();
    let best = null;
    for (const [name, re] of KEYWORDS) {
      const n = (re.test(title) ? 2 : 0) + (re.test(body) ? 1 : 0);
      if (n > 0 && (!best || n > best.n)) best = { name, n };
    }
    if (best) genre = best.name;
  }

  const tags = [];
  for (const g of input.genres ?? []) { const t = String(g).trim().toLowerCase(); if (t && !tags.includes(t)) tags.push(t); }
  if (input.tv) tags.push('tv series');
  if (input.fourK) tags.push('4k');
  const year = Number.parseInt(input.year, 10);
  if (year >= 1900 && year <= 2100) tags.push(`${Math.floor(year / 10) * 10}s`);

  return {
    genre, subgenre, confidence, tags: tags.slice(0, 12), secondary: [],
    needsReview: genre === MOVIE_UNCATEGORIZED || confidence !== 'high',
  };
}
