// SQLite storage using Node's built-in `node:sqlite` (no native dependencies).
// Full-text search uses FTS5, kept in sync by `reindex()` on every write.

import { DatabaseSync } from 'node:sqlite';
import { authorSortKey, titleSortKey } from './text.js';
import { UNCATEGORIZED } from './genres.js';
import { DISC_MEDIA } from './barcode.js';

const STATUSES = ['unread', 'reading', 'read'];

const SCHEMA = `
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS books (
  id            INTEGER PRIMARY KEY,
  isbn13        TEXT UNIQUE,
  isbn10        TEXT,
  title         TEXT NOT NULL,
  subtitle      TEXT NOT NULL DEFAULT '',
  authors       TEXT NOT NULL DEFAULT '[]',          -- JSON array
  author_sort   TEXT NOT NULL DEFAULT '',
  title_sort    TEXT NOT NULL DEFAULT '',
  publisher     TEXT NOT NULL DEFAULT '',
  published     TEXT NOT NULL DEFAULT '',
  pages         INTEGER,
  description   TEXT NOT NULL DEFAULT '',
  cover_file    TEXT,
  genre         TEXT NOT NULL DEFAULT '${UNCATEGORIZED}',
  subgenre      TEXT NOT NULL DEFAULT '',
  genre_locked  INTEGER NOT NULL DEFAULT 0,          -- user chose the genre; never auto-change
  tags_locked   INTEGER NOT NULL DEFAULT 0,          -- user edited tags; never auto-change
  needs_review  INTEGER NOT NULL DEFAULT 0,
  confidence    TEXT NOT NULL DEFAULT 'low',
  status        TEXT NOT NULL DEFAULT 'unread' CHECK (status IN ('unread','reading','read')),
  location      TEXT NOT NULL DEFAULT '',
  notes         TEXT NOT NULL DEFAULT '',
  meta_raw      TEXT NOT NULL DEFAULT '{}',          -- subjects/categories/dewey kept so reclassify works offline
  added_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS idx_books_genre ON books(genre, subgenre);
CREATE INDEX IF NOT EXISTS idx_books_author ON books(author_sort);

CREATE TABLE IF NOT EXISTS book_tags (
  book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  tag     TEXT NOT NULL,
  PRIMARY KEY (book_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_tags_tag ON book_tags(tag);

CREATE VIRTUAL TABLE IF NOT EXISTS books_fts USING fts5(
  title, subtitle, authors, tags, genre, description, notes, isbn,
  tokenize = 'unicode61 remove_diacritics 2'
);
`;

export class LibraryDb {
  constructor(path = ':memory:') {
    this.db = new DatabaseSync(path);
    this.db.exec(SCHEMA);
    this.#migrate();
  }

  /** Add the DVD/Blu-ray columns to libraries created before they existed. Safe to run on every start. */
  #migrate() {
    const have = new Set(this.db.prepare('PRAGMA table_info(books)').all().map((c) => c.name));
    if (!have.has('media')) this.db.exec("ALTER TABLE books ADD COLUMN media TEXT NOT NULL DEFAULT 'book'");
    if (!have.has('barcode')) this.db.exec('ALTER TABLE books ADD COLUMN barcode TEXT');
    if (!have.has('runtime')) this.db.exec('ALTER TABLE books ADD COLUMN runtime INTEGER');
    this.db.exec(`
      CREATE UNIQUE INDEX IF NOT EXISTS idx_books_barcode ON books(barcode) WHERE barcode IS NOT NULL;
      CREATE INDEX IF NOT EXISTS idx_books_media ON books(media);`);
  }

  close() { this.db.close(); }

  #tx(fn) {
    this.db.exec('BEGIN IMMEDIATE');
    try { const r = fn(); this.db.exec('COMMIT'); return r; }
    catch (e) { this.db.exec('ROLLBACK'); throw e; }
  }

  // ---------------------------------------------------------------- reads

  #select(where = '', params = [], tail = '') {
    const rows = this.db.prepare(`
      SELECT b.*, (SELECT group_concat(tag, char(31)) FROM book_tags t WHERE t.book_id = b.id) AS tags_joined
      FROM books b ${where} ${tail}`).all(...params);
    return rows.map(serialize);
  }

  get(id) { return this.#select('WHERE b.id = ?', [id])[0] ?? null; }
  findByBarcode(gtin13) { return this.#select('WHERE b.barcode = ?', [gtin13])[0] ?? null; }
  findByIsbn(isbn13) { return this.#select('WHERE b.isbn13 = ?', [isbn13])[0] ?? null; }
  all() { return this.#select('', [], 'ORDER BY b.id'); }
  count() { return this.db.prepare('SELECT COUNT(*) n FROM books').get().n; }

  /**
   * @param {{q?:string, genre?:string, subgenre?:string, tag?:string, status?:string,
   *          location?:string, review?:boolean, sort?:string, limit?:number, offset?:number}} p
   */
  list(p = {}) {
    const where = [];
    const params = [];
    let from = 'books b';
    let rank = '';

    const fts = buildFtsQuery(p.q);
    if (p.q && !fts) return { books: [], total: 0 };
    if (fts) {
      from = 'books_fts JOIN books b ON b.id = books_fts.rowid';
      where.push('books_fts MATCH ?');
      params.push(fts);
      rank = 'bm25(books_fts, 10, 4, 8, 5, 3, 1, 2, 6)'; // weights follow the FTS column order
    }
    if (p.media === 'movie') where.push(`b.media IN (${DISC_MEDIA.map((m) => `'${m}'`).join(',')})`);
    else if (p.media) { where.push('b.media = ?'); params.push(p.media); }
    if (p.genre) { where.push('b.genre = ?'); params.push(p.genre); }
    if (p.subgenre) { where.push('b.subgenre = ?'); params.push(p.subgenre); }
    if (p.status) { where.push('b.status = ?'); params.push(p.status); }
    if (p.location) { where.push('b.location = ?'); params.push(p.location); }
    if (p.review) where.push('b.needs_review = 1');
    if (p.tag) { where.push('EXISTS (SELECT 1 FROM book_tags t WHERE t.book_id = b.id AND t.tag = ?)'); params.push(p.tag.toLowerCase()); }

    const w = where.length ? 'WHERE ' + where.join(' AND ') : '';
    const sort = p.sort ?? (fts ? 'relevance' : 'genre');
    const unc = `(b.genre = '${UNCATEGORIZED}')`; // shelve Uncategorized last
    const orders = {
      relevance: rank ? `${rank}, b.title_sort` : 'b.title_sort',
      genre: `(b.media <> 'book'), ${unc}, b.genre COLLATE NOCASE, (b.subgenre = ''), b.subgenre COLLATE NOCASE, b.author_sort, b.title_sort`,
      title: 'b.title_sort, b.author_sort',
      author: 'b.author_sort, b.title_sort',
      added: 'b.added_at DESC, b.id DESC',
      published: "b.published DESC, b.title_sort",
    };
    const order = orders[sort] ?? orders.genre;

    const total = this.db.prepare(`SELECT COUNT(*) n FROM ${from} ${w}`).get(...params).n;
    const limit = Math.min(Math.max(parseInt(p.limit ?? 500, 10) || 500, 1), 2000);
    const offset = Math.max(parseInt(p.offset ?? 0, 10) || 0, 0);
    const rows = this.db.prepare(`
      SELECT b.*, (SELECT group_concat(tag, char(31)) FROM book_tags t WHERE t.book_id = b.id) AS tags_joined
      FROM ${from} ${w} ORDER BY ${order} LIMIT ? OFFSET ?`).all(...params, limit, offset);
    return { books: rows.map(serialize), total };
  }

  /** Genre tree with counts, tags with counts, locations, statuses. */
  facets() {
    // Books and movies are shelved separately: a film's "Science Fiction" is not a novel's.
    const tree = (cond) => {
      const g = this.db.prepare(`SELECT genre, subgenre, COUNT(*) n FROM books WHERE ${cond} GROUP BY genre, subgenre`).all();
      const byGenre = new Map();
      for (const r of g) {
        if (!byGenre.has(r.genre)) byGenre.set(r.genre, { genre: r.genre, count: 0, subgenres: [] });
        const e = byGenre.get(r.genre);
        e.count += r.n;
        if (r.subgenre) e.subgenres.push({ name: r.subgenre, count: r.n });
      }
      return [...byGenre.values()]
        .map((e) => ({ ...e, subgenres: e.subgenres.sort((a, b) => b.count - a.count || a.name.localeCompare(b.name)) }))
        .sort((a, b) => (a.genre === UNCATEGORIZED) - (b.genre === UNCATEGORIZED) || a.genre.localeCompare(b.genre));
    };
    const genres = tree("media = 'book'");
    const movieGenres = tree("media <> 'book'");
    const mediaCounts = Object.fromEntries(this.db.prepare('SELECT media AS name, COUNT(*) AS n FROM books GROUP BY media').all().map((r) => [r.name, r.n]));
    return {
      genres, movieGenres,
      media: { book: mediaCounts.book ?? 0, dvd: mediaCounts.dvd ?? 0, bluray: mediaCounts.bluray ?? 0 },
      tags: this.db.prepare('SELECT tag AS name, COUNT(*) AS count FROM book_tags GROUP BY tag ORDER BY count DESC, tag LIMIT 200').all(),
      locations: this.db.prepare("SELECT location AS name, COUNT(*) AS count FROM books WHERE location <> '' GROUP BY location ORDER BY location").all(),
      statuses: this.db.prepare('SELECT status AS name, COUNT(*) AS count FROM books GROUP BY status').all(),
      needsReview: this.db.prepare('SELECT COUNT(*) n FROM books WHERE needs_review = 1').get().n,
      total: this.count(),
    };
  }

  // --------------------------------------------------------------- writes

  /** @returns the stored book. Throws on duplicate ISBN (UNIQUE constraint). */
  insert(b) {
    return this.#tx(() => {
      const authors = b.authors ?? [];
      const r = this.db.prepare(`
        INSERT INTO books (media, barcode, runtime, isbn13, isbn10, title, subtitle, authors, author_sort, title_sort, publisher, published, pages,
                           description, cover_file, genre, subgenre, genre_locked, tags_locked, needs_review, confidence,
                           status, location, notes, meta_raw)
        VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`).run(
        b.media ?? 'book', b.barcode ?? null, b.runtime ?? null, b.isbn13 ?? null, b.isbn10 ?? null, b.title, b.subtitle ?? '', JSON.stringify(authors),
        authorSortKey(authors[0] ?? ''), titleSortKey(b.title), b.publisher ?? '', b.published ?? '',
        b.pages ?? null, b.description ?? '', b.cover_file ?? null,
        b.genre ?? UNCATEGORIZED, b.subgenre ?? '', b.genre_locked ? 1 : 0, b.tags_locked ? 1 : 0,
        b.needs_review ? 1 : 0, b.confidence ?? 'low', b.status ?? 'unread', b.location ?? '', b.notes ?? '',
        JSON.stringify(b.meta_raw ?? {}));
      const id = Number(r.lastInsertRowid);
      this.#setTags(id, b.tags ?? []);
      this.#reindex(id);
      return this.get(id);
    });
  }

  /** Partial update. `tags` replaces the tag set. */
  update(id, patch) {
    return this.#tx(() => {
      const cur = this.db.prepare('SELECT * FROM books WHERE id = ?').get(id);
      if (!cur) return null;
      const cols = ['media', 'barcode', 'runtime', 'isbn13', 'isbn10', 'title', 'subtitle', 'publisher', 'published', 'pages', 'description', 'cover_file',
        'genre', 'subgenre', 'genre_locked', 'tags_locked', 'needs_review', 'confidence', 'status', 'location', 'notes'];
      const sets = [];
      const vals = [];
      // node:sqlite cannot bind JS booleans, so convert them to 0/1 here for every caller.
      for (const c of cols) if (c in patch) { sets.push(`${c} = ?`); vals.push(typeof patch[c] === 'boolean' ? +patch[c] : patch[c]); }
      if ('meta_raw' in patch) { sets.push('meta_raw = ?'); vals.push(JSON.stringify(patch.meta_raw ?? {})); }
      if ('authors' in patch) {
        sets.push('authors = ?', 'author_sort = ?');
        vals.push(JSON.stringify(patch.authors), authorSortKey(patch.authors[0] ?? ''));
      }
      if ('title' in patch) { sets.push('title_sort = ?'); vals.push(titleSortKey(patch.title)); }
      sets.push("updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')");
      this.db.prepare(`UPDATE books SET ${sets.join(', ')} WHERE id = ?`).run(...vals, id);
      if ('tags' in patch) this.#setTags(id, patch.tags);
      this.#reindex(id);
      return this.get(id);
    });
  }

  delete(id) {
    return this.#tx(() => {
      this.db.prepare('DELETE FROM books_fts WHERE rowid = ?').run(id);
      return this.db.prepare('DELETE FROM books WHERE id = ?').run(id).changes > 0;
    });
  }

  #setTags(id, tags) {
    this.db.prepare('DELETE FROM book_tags WHERE book_id = ?').run(id);
    const ins = this.db.prepare('INSERT OR IGNORE INTO book_tags (book_id, tag) VALUES (?, ?)');
    for (const t of new Set(tags.map(cleanTag).filter(Boolean))) ins.run(id, t);
  }

  #reindex(id) {
    const row = this.#select('WHERE b.id = ?', [id])[0];
    this.db.prepare('DELETE FROM books_fts WHERE rowid = ?').run(id);
    if (!row) return;
    this.db.prepare(`INSERT INTO books_fts (rowid, title, subtitle, authors, tags, genre, description, notes, isbn)
                     VALUES (?,?,?,?,?,?,?,?,?)`).run(
      id, row.title, row.subtitle, row.authors.join(' '), row.tags.join(' '),
      `${row.genre} ${row.subgenre}`, row.description, row.notes,
      // The "isbn" column also holds identifiers/labels users search by: barcode, and "dvd" / "blu-ray".
      `${row.isbn13 ?? ''} ${row.isbn10 ?? ''} ${row.barcode ?? ''} ${row.media === 'bluray' ? 'blu-ray bluray' : row.media === 'dvd' ? 'dvd' : ''}`.trim());
  }

  /** Rebuild the whole search index (e.g. after manual DB edits). */
  rebuildIndex() {
    this.#tx(() => {
      this.db.exec('DELETE FROM books_fts');
      for (const { id } of this.db.prepare('SELECT id FROM books').all()) this.#reindex(id);
    });
  }
}

// ------------------------------------------------------------------ helpers

export function cleanTag(t) {
  return String(t ?? '').replace(/\s+/g, ' ').trim().toLowerCase().slice(0, 40);
}

export function isStatus(s) { return STATUSES.includes(s); }

/**
 * Turn user text into a safe FTS5 query: every word becomes a quoted prefix term
 * ("harry"* "pott"*) so typing is forgiving and no FTS syntax can be injected.
 */
export function buildFtsQuery(q) {
  const tokens = String(q ?? '').normalize('NFKC').match(/[\p{L}\p{N}]+/gu);
  if (!tokens) return null;
  return tokens.slice(0, 12).map((t) => `"${t}"*`).join(' ');
}

function serialize(r) {
  const parse = (s, d) => { try { return JSON.parse(s); } catch { return d; } };
  const { tags_joined, authors, meta_raw, genre_locked, tags_locked, needs_review, ...rest } = r;
  return {
    ...rest,
    authors: parse(authors, []),
    tags: tags_joined ? tags_joined.split('\u001f').sort() : [],
    genre_locked: !!genre_locked,
    tags_locked: !!tags_locked,
    needs_review: !!needs_review,
    meta_raw: parse(meta_raw, {}),
    has_cover: !!r.cover_file,
  };
}
