import test from 'node:test';
import assert from 'node:assert/strict';
import { LibraryDb, buildFtsQuery } from '../server/db.js';

function seeded() {
  const db = new LibraryDb(':memory:');
  const add = (o) => db.insert({ status: 'unread', ...o });
  add({ isbn13: '9780441013593', title: 'Dune', authors: ['Frank Herbert'], genre: 'Science Fiction', subgenre: 'Space Opera', tags: ['desert', 'politics'], description: 'Spice and sandworms on Arrakis.' });
  add({ isbn13: '9780441172719', title: 'Dune Messiah', authors: ['Frank Herbert'], genre: 'Science Fiction', tags: ['prophecy'] });
  add({ isbn13: '9780547928227', title: 'The Hobbit', authors: ['J. R. R. Tolkien'], genre: 'Fantasy', subgenre: 'Epic Fantasy', tags: ['dragons'], status: 'read', location: 'Living room' });
  add({ isbn13: '9780375704024', title: 'Les Misérables', authors: ['Victor Hugo'], genre: 'Classics', location: 'Study' });
  add({ title: 'Mystery Book Without ISBN', authors: ['Zed Last'], genre: 'Uncategorized', needs_review: true });
  add({ isbn13: '9780140449136', title: 'A Wizard of Earthsea', authors: ['Ursula K. Le Guin'], genre: 'Fantasy', tags: ['wizards'] });
  return db;
}
const titles = (r) => r.books.map((b) => b.title);

test('FTS query builder quotes tokens and strips syntax', () => {
  assert.equal(buildFtsQuery('harry pott'), '"harry"* "pott"*');
  assert.equal(buildFtsQuery('"AND" OR NOT ( * )'), '"AND"* "OR"* "NOT"*');
  assert.equal(buildFtsQuery('   ***  '), null);
  assert.equal(buildFtsQuery(undefined), null);
});

test('search by title prefix, author, tag, description, isbn', () => {
  const db = seeded();
  assert.deepEqual(titles(db.list({ q: 'dun' })).sort(), ['Dune', 'Dune Messiah']);
  assert.ok(titles(db.list({ q: 'tolkien' })).includes('The Hobbit'));
  assert.deepEqual(titles(db.list({ q: 'dragons' })), ['The Hobbit']);
  assert.deepEqual(titles(db.list({ q: 'sandworms' })), ['Dune']);
  assert.deepEqual(titles(db.list({ q: '9780547928227' })), ['The Hobbit']);
});

test('multi-word search is AND; title matches outrank description matches', () => {
  const db = seeded();
  assert.deepEqual(titles(db.list({ q: 'frank messiah' })), ['Dune Messiah']);
  const r = db.list({ q: 'dune' });
  assert.equal(r.books[0].title.startsWith('Dune'), true);
});

test('search ignores diacritics and case', () => {
  const db = seeded();
  assert.deepEqual(titles(db.list({ q: 'MISERABLES' })), ['Les Misérables']);
  assert.deepEqual(titles(db.list({ q: 'miserábles' })), ['Les Misérables']);
});

test('hostile search input cannot break the query', () => {
  const db = seeded();
  for (const q of ['"', "'; DROP TABLE books; --", 'a OR b', 'title:dune', '(((', 'NEAR(a b)', '*']) {
    assert.doesNotThrow(() => db.list({ q }), q);
  }
  assert.equal(db.count(), 6);
});

test('filters: genre, subgenre, tag, status, location, review', () => {
  const db = seeded();
  assert.equal(db.list({ genre: 'Fantasy' }).total, 2);
  assert.deepEqual(titles(db.list({ subgenre: 'Epic Fantasy' })), ['The Hobbit']);
  assert.deepEqual(titles(db.list({ tag: 'Wizards' })), ['A Wizard of Earthsea']);
  assert.deepEqual(titles(db.list({ status: 'read' })), ['The Hobbit']);
  assert.deepEqual(titles(db.list({ location: 'Study' })), ['Les Misérables']);
  assert.deepEqual(titles(db.list({ review: true })), ['Mystery Book Without ISBN']);
  assert.deepEqual(titles(db.list({ q: 'dune', genre: 'Science Fiction', tag: 'desert' })), ['Dune']);
});

test('default sort is shelf order: genre, subgenre (unsubgenred last), author, title; Uncategorized last', () => {
  assert.deepEqual(titles(seeded().list()), [
    'Les Misérables',            // Classics
    'The Hobbit',                // Fantasy / Epic Fantasy
    'A Wizard of Earthsea',      // Fantasy / (none) sorts after named subgenres
    'Dune',                      // Science Fiction / Space Opera
    'Dune Messiah',              // Science Fiction / (none)
    'Mystery Book Without ISBN', // Uncategorized always last
  ]);
});

test('author sort uses surnames; title sort ignores leading articles', () => {
  const db = seeded();
  assert.deepEqual(titles(db.list({ sort: 'author' })), [
    'Dune', 'Dune Messiah',          // herbert
    'Les Misérables',                // hugo
    'Mystery Book Without ISBN',     // last
    'A Wizard of Earthsea',          // le guin
    'The Hobbit',                    // tolkien
  ]);
  assert.deepEqual(titles(db.list({ sort: 'title' })), [
    'Dune', 'Dune Messiah', 'The Hobbit', 'Les Misérables', 'Mystery Book Without ISBN', 'A Wizard of Earthsea',
  ]); // hobbit < les < mystery < wizard once "The"/"A" are ignored
});

test('duplicate ISBN is rejected; books without ISBN are fine', () => {
  const db = seeded();
  assert.throws(() => db.insert({ isbn13: '9780441013593', title: 'Dune again' }), /UNIQUE/);
  db.insert({ title: 'Another no-ISBN book' });
  assert.equal(db.count(), 7);
  assert.equal(db.list({ q: 'Dune again' }).total, 0); // failed insert rolled back, not indexed
});

test('update re-syncs tags and search index; delete removes from search', () => {
  const db = seeded();
  const dune = db.findByIsbn('9780441013593');
  db.update(dune.id, { title: 'Dune (Deluxe)', tags: ['spice', 'Arrakis'], notes: 'borrowed by Sam' });
  assert.equal(db.list({ q: 'deluxe' }).total, 1);
  assert.equal(db.list({ q: 'arrakis' }).total, 1);
  assert.equal(db.list({ q: 'desert' }).total, 0);       // old tag gone from index
  assert.equal(db.list({ tag: 'desert' }).total, 0);
  assert.equal(db.list({ q: 'borrowed' }).total, 1);
  assert.deepEqual(db.get(dune.id).tags, ['arrakis', 'spice']);
  assert.equal(db.delete(dune.id), true);
  assert.equal(db.list({ q: 'dune' }).total, 1);          // only Messiah left
  assert.equal(db.get(dune.id), null);
  assert.equal(db.db.prepare('SELECT COUNT(*) n FROM book_tags WHERE book_id = ?').get(dune.id).n, 0);
});

test('facets: genre tree with counts, tags, locations, review count', () => {
  const f = seeded().facets();
  const g = Object.fromEntries(f.genres.map((x) => [x.genre, x]));
  assert.equal(g.Fantasy.count, 2);
  assert.deepEqual(g.Fantasy.subgenres, [{ name: 'Epic Fantasy', count: 1 }]);
  assert.equal(f.genres.at(-1).genre, 'Uncategorized');
  assert.equal(f.needsReview, 1);
  assert.equal(f.total, 6);
  assert.deepEqual(f.locations.map((l) => l.name), ['Living room', 'Study']);
});

test('limit/offset paginate and report the full total', () => {
  const db = seeded();
  const p = db.list({ limit: 2, offset: 2, sort: 'title' });
  assert.equal(p.books.length, 2);
  assert.equal(p.total, 6);
});

test('rebuildIndex restores search after the index is wiped', () => {
  const db = seeded();
  db.db.exec('DELETE FROM books_fts');
  assert.equal(db.list({ q: 'dune' }).total, 0);
  db.rebuildIndex();
  assert.equal(db.list({ q: 'dune' }).total, 2);
});
