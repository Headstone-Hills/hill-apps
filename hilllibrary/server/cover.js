// Cover scanning: photo of a book cover (no barcode) -> candidate books.
//
//   1. READ    pull text off the image. Interchangeable readers, tried in order until one is confident:
//                * Apple Vision (macOS, local, free): by far the best on stylised covers. A tiny Swift helper
//                  (server/native/cover-ocr.swift) is compiled once on first use.
//                * Claude vision (optional, ANTHROPIC_API_KEY): returns title/authors directly; best on
//                  stylised covers. https://docs.anthropic.com/en/api/messages (image content blocks)
//                * Tesseract OCR (local, free; `brew install tesseract`): returns words with their sizes.
//                  Big type is almost always the title, so the largest words drive the search.
//   2. SEARCH  Open Library (title/author and free-text queries) and Google Books, in parallel.
//                https://openlibrary.org/dev/docs/api/search
//   3. RANK    score each candidate by how much of its title and author surname actually appears in the
//              text read from the cover (OCR-error tolerant), so a wrong-but-similar book ranks low.
//
// Nothing here is final: the user always picks from the candidates, then the normal ISBN flow
// (preview -> add) takes over. A wrong guess therefore costs one tap, never a wrong entry.

import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, writeFile, rm, mkdir, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { toIsbn13 } from './isbn.js';

const run = promisify(execFile);
const UA = 'HillLibrary/1.0 (personal home library app)';
const TIMEOUT_MS = 12000;
const PARALLEL = 3;     // Open Library throttles bursts (it answered 500s and timeouts to ten at once)
const str = (v) => (typeof v === 'string' ? v.trim() : '');

// ------------------------------------------------------------------ helpers

const fold = (s) => String(s ?? '').normalize('NFKD').replace(/\p{M}+/gu, '').toLowerCase();
/** Lowercase, accent-free word tokens (letters/digits only). */
export const tokens = (s) => fold(s).match(/[\p{L}\p{N}]+/gu) ?? [];

function editDistanceAtMost1(a, b) {
  if (a === b) return true;
  if (Math.abs(a.length - b.length) > 1) return false;
  let i = 0, j = 0, edits = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) { i++; j++; continue; }
    if (++edits > 1) return false;
    if (a.length > b.length) i++;
    else if (a.length < b.length) j++;
    else { i++; j++; }
  }
  return edits + (a.length - i) + (b.length - j) <= 1;
}

/**
 * Is `word` among the cover's words? Exact, or one OCR slip. Words of 5+ letters tolerate any single edit;
 * 4-letter words only a dropped/added letter ("ogone" for "gone"), because a swapped letter (bone/gone)
 * is far more likely a different word than a misread.
 */
function seen(word, bag) {
  if (bag.has(word)) return true;
  if (word.length < 4) return false;
  for (const w of bag) {
    if (w.length < 4 || Math.abs(w.length - word.length) > 1) continue;
    if (word.length === 4 && w.length === 4) continue;
    if (editDistanceAtMost1(word, w)) return true;
  }
  return false;
}

const STOP = new Set(['the', 'a', 'an', 'of', 'and', 'in', 'to', 'for', 'on', 'by', 'or']);

// ------------------------------------------------------------------ reading: tesseract

/** Parse `tesseract ... tsv` output into words: {text, conf, left, top, height}. */
export function parseTsv(tsv) {
  const words = [];
  for (const row of String(tsv).split('\n').slice(1)) {
    const c = row.split('\t');
    if (c.length < 12) continue;
    const conf = Number(c[10]);
    const text = c.slice(11).join('\t').trim();
    if (!text || !(conf >= 0)) continue;       // conf -1 rows are layout containers, not words
    words.push({ text, conf, left: Number(c[6]), top: Number(c[7]), height: Number(c[9]) });
  }
  return words;
}

/** Turn OCR words into the pieces the search needs. */
export function summarizeWords(words, extraPass = []) {
  const ok = (w) => /\p{L}{2,}|\d{3,}/u.test(w.text) && w.conf >= 30;
  const usable = words.filter(ok);
  const extra = extraPass.filter(ok);
  // Both passes feed the matching text; the first pass (best layout) decides the "headline".
  const all = [...usable, ...extra].map((w) => w.text).join(' ');
  if (!usable.length && !extra.length) return { text: '', headline: '', words: [] };
  if (!usable.length) return { text: all, headline: '', words: extra };
  const hs = usable.map((w) => w.height).sort((a, b) => a - b);
  const maxH = hs[hs.length - 1];
  // "Headline" = the biggest type, read top-to-bottom then left-to-right: normally the title (and often the author).
  const big = usable.filter((w) => w.height >= maxH * 0.6 && w.conf >= 45)
    .sort((a, b) => Math.round(a.top / (maxH || 1)) - Math.round(b.top / (maxH || 1)) || a.left - b.left);
  return { text: all, headline: big.map((w) => w.text).join(' '), words: [...usable, ...extra] };
}

/**
 * Concurrency-limited tesseract runner: OCR is CPU heavy, so never run more than two at once.
 * `exec` is injectable for tests.
 */
export function createTesseractReader({ bin = 'tesseract', lang = 'eng', exec = run, maxParallel = 2 } = {}) {
  let active = 0;
  const waiting = [];
  const acquire = () => new Promise((res) => (active < maxParallel ? (active++, res()) : waiting.push(res)));
  const release = () => { const next = waiting.shift(); if (next) next(); else active--; };

  return {
    name: 'tesseract',
    async read(buffer) {
      await acquire();
      const dir = await mkdtemp(join(tmpdir(), 'hl-cover-'));
      try {
        const file = join(dir, 'cover');
        await writeFile(file, buffer);
        // --psm 11 = sparse text: covers are not paragraphs. Two passes because binarisation matters a lot on
        // coloured, gradient covers: the default (Otsu) and an adaptive one (Sauvola) catch different words.
        // Args go straight to the binary, never through a shell.
        const pass = async (extra) => {
          try { return parseTsv((await exec(bin, [file, 'stdout', '-l', lang, '--psm', '11', ...extra, 'tsv'], { timeout: 40000, maxBuffer: 16 * 1024 * 1024 })).stdout); }
          catch (e) { if (extra.length) return []; throw e; }   // only the first pass may fail the whole read
        };
        const [a, b] = await Promise.all([pass([]), pass(['-c', 'thresholding_method=2'])]);
        return summarizeWords(a, b);
      } finally {
        await rm(dir, { recursive: true, force: true });
        release();
      }
    },
  };
}

// ------------------------------------------------------------------ reading: Apple Vision (macOS)

const HERE = dirname(fileURLToPath(import.meta.url));
export const OCR_SOURCE = join(HERE, 'native', 'cover-ocr.swift');

/** Parse the helper's `top left height confidence text` lines into the same word shape the OCR path uses. */
export function parseVisionLines(out) {
  const words = [];
  for (const row of String(out).split('\n')) {
    const c = row.split('\t');
    if (c.length < 5) continue;
    const [top, left, height, conf] = c.slice(0, 4).map(Number);
    const text = c.slice(4).join(' ').trim();
    if (text && [top, left, height].every(Number.isFinite)) words.push({ text, conf: Number.isFinite(conf) && conf > 0 ? conf * 100 : 90, left, top, height });
  }
  return words;
}

/**
 * Apple's on-device text recogniser, via a small Swift helper compiled on first use into `binDir`.
 * Throws (so the chain moves on) when not on macOS or when Swift isn't installed.
 */
export function createVisionReader({ binDir, exec = run, platform = process.platform, source = OCR_SOURCE, maxParallel = 2 } = {}) {
  let building = null;
  let active = 0;
  const waiting = [];
  const acquire = () => new Promise((res) => (active < maxParallel ? (active++, res()) : waiting.push(res)));
  const release = () => { const next = waiting.shift(); if (next) next(); else active--; };
  const bin = join(binDir, 'cover-ocr');

  async function ensureBinary() {
    if (platform !== 'darwin') throw new Error('Apple Vision is only available on macOS');
    building ??= (async () => {
      const [b, src] = await Promise.all([stat(bin).catch(() => null), stat(source)]);
      if (b && b.mtimeMs >= src.mtimeMs) return;                          // up to date
      await mkdir(binDir, { recursive: true });
      try { await exec('swiftc', ['-O', source, '-o', bin], { timeout: 180000 }); }
      catch (e) { throw new Error(`could not build the macOS text reader (is the Xcode command line tools package installed? ${e.code ?? e.message})`); }
    })().catch((e) => { building = null; throw e; });                     // a failed build may be retried later
    return building;
  }

  return {
    name: 'vision',
    /** Build the helper ahead of time (called at startup) so the first scan isn't slow. Never throws. */
    warm: () => ensureBinary().then(() => true, () => false),
    async read(buffer) {
      await ensureBinary();
      await acquire();
      const dir = await mkdtemp(join(tmpdir(), 'hl-cover-'));
      try {
        const file = join(dir, 'cover');
        await writeFile(file, buffer);
        const { stdout } = await exec(bin, [file], { timeout: 40000, maxBuffer: 8 * 1024 * 1024 });
        return summarizeWords(parseVisionLines(stdout));
      } finally { await rm(dir, { recursive: true, force: true }); release(); }
    },
  };
}

// ------------------------------------------------------------------ reading: Claude vision

const VISION_PROMPT = `This is a photo of the front cover of a book. Read only what is printed on it. Reply with ONLY a JSON object, no prose:
{"title": string, "subtitle": string, "authors": [string], "series": string, "publisher": string, "isbn": string, "language": string}
Use "" or [] for anything you cannot read. Do not guess from memory: if the title is unreadable, return an empty title.`;

/** Claude reads the cover and returns structured fields. Throws on HTTP failure. */
export function createClaudeReader({ apiKey, model = 'claude-haiku-5-5', fetchImpl = fetch } = {}) {
  return {
    name: 'claude',
    async read(buffer, mime) {
      const res = await fetchImpl('https://api.anthropic.com/v1/messages', {
        method: 'POST',
        headers: { 'x-api-key': apiKey, 'anthropic-version': '2023-06-01', 'content-type': 'application/json', 'User-Agent': UA },
        body: JSON.stringify({
          model, max_tokens: 400,
          messages: [{ role: 'user', content: [
            { type: 'image', source: { type: 'base64', media_type: mime, data: buffer.toString('base64') } },
            { type: 'text', text: VISION_PROMPT },
          ] }],
        }),
        signal: AbortSignal.timeout(30000),
      });
      if (!res.ok) throw new Error(`Claude API HTTP ${res.status}`);
      const body = await res.json();
      const text = (Array.isArray(body?.content) ? body.content : []).filter((b) => b?.type === 'text').map((b) => b.text).join('');
      const json = text.match(/\{[\s\S]*\}/)?.[0];
      let o = {};
      try { o = JSON.parse(json ?? '{}'); } catch { /* unreadable reply: treat as nothing read */ }
      const authors = Array.isArray(o.authors) ? o.authors.map(str).filter(Boolean).slice(0, 4) : [];
      const title = str(o.title), subtitle = str(o.subtitle);
      return {
        title, subtitle, authors, publisher: str(o.publisher), series: str(o.series), isbnText: str(o.isbn),
        headline: [title, authors.join(' ')].filter(Boolean).join(' '),
        text: [title, subtitle, authors.join(' '), str(o.series), str(o.publisher)].filter(Boolean).join(' '),
      };
    },
  };
}

// ------------------------------------------------------------------ ISBNs printed on the cover

/** A valid ISBN printed in text ("ISBN 978-0-547-92822-7"), or null. Checksums weed out OCR noise. */
export function findIsbn(text) {
  for (const m of String(text ?? '').matchAll(/(?:97[89][\d\- ]{10,16}\d|\b\d[\d\- ]{8,12}[\dXx]\b)/g)) {
    const isbn = toIsbn13(m[0]);
    if (isbn) return isbn;
  }
  return null;
}

// ------------------------------------------------------------------ searching

async function getJson(url, fetchImpl, retries = 1) {
  try {
    const res = await fetchImpl(url, { headers: { 'User-Agent': UA, Accept: 'application/json' }, signal: AbortSignal.timeout(TIMEOUT_MS) });
    if (!res.ok) throw Object.assign(new Error(`HTTP ${res.status}`), { retry: res.status >= 500 || res.status === 429 });
    return await res.json();
  } catch (e) {
    if (retries > 0 && (e.retry || e.name === 'TimeoutError' || e.name === 'AbortError')) {
      await new Promise((r) => setTimeout(r, 500));
      return getJson(url, fetchImpl, retries - 1);
    }
    throw e;
  }
}

/** Run async tasks with at most `limit` in flight; results keep their order. */
async function pooled(items, limit, fn) {
  const out = new Array(items.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (next < items.length) { const i = next++; out[i] = await fn(items[i]); }
  }));
  return out;
}

const firstIsbn13 = (list) => {
  for (const raw of Array.isArray(list) ? list : []) {
    const s = String(raw).replace(/[-\s]/g, '');
    if (/^97[89]\d{10}$/.test(s) && toIsbn13(s)) return s;
  }
  return null;
};

async function searchOpenLibrary({ title, author, qs }, fetchImpl) {
  const fields = 'key,title,subtitle,author_name,first_publish_year,isbn,cover_i,cover_edition_key,edition_count';
  const urls = [];
  if (title) urls.push(`https://openlibrary.org/search.json?${new URLSearchParams({ title, ...(author ? { author } : {}), limit: '10', fields })}`);
  for (const q of qs) urls.push(`https://openlibrary.org/search.json?${new URLSearchParams({ q, limit: '10', fields })}`);
  const out = [];
  for (const body of await pooled(urls, PARALLEL, (u) => getJson(u, fetchImpl).catch((e) => ({ error: e })))) {
    if (body?.error) { out.error = body.error; continue; }
    for (const d of Array.isArray(body?.docs) ? body.docs : []) {
      const t = str(d.title);
      if (!t) continue;
      out.push({
        source: 'openlibrary', id: str(d.key), title: t, subtitle: str(d.subtitle),
        authors: Array.isArray(d.author_name) ? d.author_name.map(str).filter(Boolean).slice(0, 3) : [],
        year: Number.isInteger(d.first_publish_year) ? String(d.first_publish_year) : '',
        popularity: Number.isInteger(d.edition_count) ? d.edition_count : 0,
        coverEdition: /^OL\d+M$/.test(str(d.cover_edition_key)) ? str(d.cover_edition_key) : '',
        isbn13: firstIsbn13(d.isbn),
        cover_url: Number.isInteger(d.cover_i) ? `https://covers.openlibrary.org/b/id/${d.cover_i}-M.jpg` : '',
      });
    }
  }
  return out;
}

async function searchGoogle({ title, author, q }, apiKey, fetchImpl) {
  const query = title ? `intitle:${title}${author ? ` inauthor:${author}` : ''}` : q;
  if (!query) return [];
  const url = `https://www.googleapis.com/books/v1/volumes?${new URLSearchParams({ q: query, maxResults: '8', printType: 'books', ...(apiKey ? { key: apiKey } : {}) })}`;
  const body = await getJson(url, fetchImpl);
  return (Array.isArray(body?.items) ? body.items : []).map((it) => {
    const v = it?.volumeInfo ?? {};
    const ids = Array.isArray(v.industryIdentifiers) ? v.industryIdentifiers.map((i) => i?.identifier) : [];
    return {
      source: 'googlebooks', id: str(it?.id), title: str(v.title), subtitle: str(v.subtitle),
      authors: Array.isArray(v.authors) ? v.authors.map(str).filter(Boolean).slice(0, 3) : [],
      year: str(v.publishedDate).slice(0, 4), isbn13: firstIsbn13(ids),
      cover_url: str(v.imageLinks?.thumbnail).replace(/^http:/, 'https:').replace(/&edge=curl/, ''),
    };
  }).filter((c) => c.title);
}

// ------------------------------------------------------------------ query building

/** Distinctive words from the OCR, biggest/most confident first. Garbled words are fine: ranking filters them. */
export function keywords(words, max = 6) {
  const best = new Map();
  for (const w of words ?? []) {
    for (const t of tokens(w.text)) {
      if (t.length < 4 || /\d/.test(t) || STOP.has(t)) continue;
      const score = w.height * (w.conf / 100);
      if (score > (best.get(t) ?? 0)) best.set(t, score);
    }
  }
  return [...best.entries()].sort((a, b) => b[1] - a[1]).slice(0, max).map(([t]) => t);
}

/** Search strings for free-text queries, most specific first. One bad word must not sink the search. */
export function buildQueries(headline, kw) {
  const qs = [];
  if (tokens(headline).join('').length >= 4) qs.push(tokens(headline).slice(0, 8).join(' '));
  const top = kw.slice(0, 4);
  for (let i = 0; i < top.length; i++) for (let j = i + 1; j < top.length; j++) qs.push(`${top[i]} ${top[j]}`);
  for (const k of kw.slice(0, 3)) qs.push(k);
  return [...new Set(qs)].slice(0, 7);
}

// ------------------------------------------------------------------ ranking

/**
 * 0..1: how well a candidate explains the text on the cover. Title words (minus stop words) and the first
 * author's surname are looked up in the cover's words, tolerating one OCR slip in longer words.
 */
export function scoreCandidate(c, coverText, headlineText = '') {
  const bag = new Set(tokens(coverText));
  const big = new Set(tokens(headlineText));          // the large type: a title in the headline is far stronger evidence than one in a small blurb
  const mainTitle = c.title.split(/[:(]/)[0];
  const all = tokens(mainTitle).filter((w) => w.length >= 2);
  const tw = all.filter((w) => !STOP.has(w));
  const titleWords = tw.length ? tw : all;
  if (!titleWords.length) return 0;
  const hits = titleWords.filter((w) => seen(w, bag)).length;
  const titleCov = hits / titleWords.length;
  const surname = tokens(c.authors[0] ?? '').pop();
  const authorHit = surname && surname.length >= 3 && seen(surname, bag) ? 1 : 0;
  // A title match with no author evidence is plausible but weaker; an author match alone proves little.
  // A book titled with just the author's name (a biography, say) matches the cover for the wrong reason.
  const authorTokens = new Set(c.authors.flatMap(tokens));
  const nameOnly = titleWords.every((w) => authorTokens.has(w));
  let score = surname ? 0.7 * titleCov + 0.3 * authorHit : 0.85 * titleCov;
  score += 0.15 * (titleWords.filter((w) => seen(w, big)).length / titleWords.length);  // prominence
  score += Math.min(0.1, hits * 0.03);                                              // more matched words = more evidence
  score += 0.08 * Math.min(1, Math.log10(1 + (c.popularity ?? 0)) / 2.5);           // tie-break toward widely published works
  if (nameOnly) score *= 0.4;
  return Math.round(score * 100) / 100;                                              // may exceed 1: only the order matters
}

/** Collapse the same book arriving from both sources (and several editions) into one candidate. */
function dedupe(cands) {
  const byKey = new Map();
  for (const c of cands) {
    const k = `${tokens(c.title.split(/[:(]/)[0]).join(' ')}|${tokens(c.authors[0] ?? '').pop() ?? ''}`;
    const cur = byKey.get(k);
    if (!cur) { byKey.set(k, { ...c }); continue; }
    cur.isbn13 ??= c.isbn13;
    cur.coverEdition ||= c.coverEdition;
    cur.cover_url ||= c.cover_url;
    cur.year ||= c.year;
    cur.popularity = Math.max(cur.popularity ?? 0, c.popularity ?? 0);
    if (!cur.authors.length) cur.authors = c.authors;
  }
  return [...byKey.values()];
}

// ------------------------------------------------------------------ edition choice

/**
 * A work has hundreds of ISBNs (listed in arbitrary order). The edition whose cover Open Library shows is
 * normally a mainstream one, so for the best few candidates take the ISBN from that edition instead.
 * Best effort: any failure leaves the earlier pick in place.
 */
async function refineIsbns(cands, fetchImpl) {
  await pooled(cands.filter((c) => c.coverEdition), PARALLEL, async (c) => {
    try {
      const e = await getJson(`https://openlibrary.org/books/${c.coverEdition}.json`, fetchImpl, 0);
      const isbn = firstIsbn13(e?.isbn_13) ?? toIsbn13((Array.isArray(e?.isbn_10) ? e.isbn_10 : [])[0] ?? '');
      if (isbn) c.isbn13 = isbn;
    } catch { /* keep the guess */ }
  });
}

// ------------------------------------------------------------------ orchestration

const topScore = (r) => r.candidates[0]?.score ?? 0;
const CONFIDENT = 0.85;   // stop reading with more readers
const REASONABLE = 0.5;   // good enough that a later reader must not replace it

/**
 * Read a cover with each reader in turn (stopping once one yields a confident match) and return the best result.
 * @param {Buffer} buffer image bytes
 * @param {string} mime   image/jpeg | image/png | image/webp
 * @param {{readers?:object[], reader?:object, fetchImpl?:typeof fetch, googleApiKey?:string}} deps
 */
export async function scanCover(buffer, mime, { readers, reader, fetchImpl = fetch, googleApiKey = '' }) {
  const list = readers ?? [reader];
  let best = null;
  const failures = [];
  for (const r of list) {
    let res;
    try { res = await scanWith(r, buffer, mime, { fetchImpl, googleApiKey }); }
    catch (e) { failures.push(`${r.name} reader unavailable (${String(e?.message ?? e).slice(0, 120)})`); continue; }
    // Prefer the earlier (better) reader: a later one only replaces it while the earlier one has no reasonable match.
    // Scores from different readers aren't comparable, so never let a weak-reader fluke outrank a decent first result.
    if (!best || (topScore(best) < REASONABLE && (topScore(res) > topScore(best) || (!topScore(best) && res.read.text.length > best.read.text.length)))) best = res;
    if (topScore(best) >= CONFIDENT) break;
  }
  if (!best) throw Object.assign(new Error(`No cover reader is available. ${failures.join(' ')}`), { noReader: true });
  best.notes.push(...failures);
  return best;
}

async function scanWith(reader, buffer, mime, { fetchImpl, googleApiKey }) {
  const read = await reader.read(buffer, mime);
  const text = read.text ?? '';
  const result = {
    reader: reader.name,
    read: { title: read.title ?? '', authors: read.authors ?? [], headline: read.headline ?? '', keywords: keywords(read.words), text: text.slice(0, 600) },
    isbn: findIsbn(`${text} ${read.isbnText ?? ''}`),
    candidates: [], notes: [],
  };
  if (!text.trim()) { result.notes.push('No readable text found. Try better light, hold the camera square to the cover, and fill the frame.'); return result; }

  // Search with the structured read when we have it (Claude), else with the biggest type (OCR).
  const title = read.title || '';
  const author = (read.authors ?? [])[0] ?? '';
  const qs = read.title ? [] : buildQueries(read.headline, keywords(read.words));
  const q = read.title ? `${title} ${author}`.trim() : qs[0] ?? '';
  const queries = { title, author, q, qs };

  const [ol, gb] = await Promise.allSettled([searchOpenLibrary(queries, fetchImpl), searchGoogle(queries, googleApiKey, fetchImpl)]);
  const found = [];
  for (const [name, r] of [['Open Library', ol], ['Google Books', gb]]) {
    if (r.status === 'fulfilled') { found.push(...r.value); if (r.value.error) result.notes.push(`${name} search had a problem (${r.value.error.message}).`); }
    else result.notes.push(`${name} search failed (${r.reason?.message ?? 'error'}).`);
  }
  if (ol.status === 'rejected' && gb.status === 'rejected') result.notes.push('Could not reach the book databases to look this up.');

  // An ISBN printed on the cover beats every guess: put it first.
  const ranked = dedupe(found).map((c) => ({ ...c, score: scoreCandidate(c, `${text} ${read.title ?? ''} ${(read.authors ?? []).join(' ')}`, `${read.headline ?? ''} ${read.title ?? ''} ${(read.authors ?? []).join(' ')}`) }))
    .filter((c) => c.score >= 0.3)
    .sort((a, b) => b.score - a.score || Number(!!b.isbn13) - Number(!!a.isbn13));
  result.candidates = ranked.slice(0, 6);
  await refineIsbns(result.candidates.slice(0, 3), fetchImpl);
  for (const c of result.candidates) delete c.coverEdition;
  if (!result.candidates.length) result.notes.push('Read some text but found no matching book. You can add it by hand with what was read.');
  return result;
}
