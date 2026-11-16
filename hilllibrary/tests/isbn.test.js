import test from 'node:test';
import assert from 'node:assert/strict';
import { toIsbn13, toIsbn10, isValidIsbn13, isValidIsbn10 } from '../server/isbn.js';
import { authorSortKey, titleSortKey } from '../server/text.js';

test('ISBN-13 validation', () => {
  assert.ok(isValidIsbn13('9780140328721'));
  assert.ok(!isValidIsbn13('9780140328722'));   // bad checksum
  assert.ok(!isValidIsbn13('9770140328721'));   // not 978/979
  assert.ok(!isValidIsbn13('978014032872'));    // wrong length
});

test('ISBN-10 validation, including X check digit', () => {
  assert.ok(isValidIsbn10('0140328726'));
  assert.ok(isValidIsbn10('080442957X'));
  assert.ok(!isValidIsbn10('0140328727'));
});

test('toIsbn13 converts ISBN-10 and strips hyphens/spaces', () => {
  assert.equal(toIsbn13('0-14-032872-6'), '9780140328721');
  assert.equal(toIsbn13(' 978-0-14-032872-1 '), '9780140328721');
  assert.equal(toIsbn13('080442957x'), '9780804429573');
  assert.equal(toIsbn13('not an isbn'), null);
  assert.equal(toIsbn13('9780140328722'), null);
});

test('toIsbn10 round-trips and refuses 979', () => {
  assert.equal(toIsbn10('9780140328721'), '0140328726');
  assert.equal(toIsbn10('9780804429573'), '080442957X');
  assert.equal(toIsbn10('9791032305690'), null);
});

test('author sort keys', () => {
  assert.equal(authorSortKey('Ursula K. Le Guin'), 'le guin ursula k.');
  assert.equal(authorSortKey('J. R. R. Tolkien'), 'tolkien j. r. r.');
  assert.equal(authorSortKey('Martin Luther King Jr.'), 'king martin luther');
  assert.equal(authorSortKey('Plato'), 'plato');
  assert.equal(authorSortKey(''), '');
});

test('title sort ignores leading articles', () => {
  assert.equal(titleSortKey('The Hobbit'), 'hobbit');
  assert.equal(titleSortKey('A Wizard of Earthsea'), 'wizard of earthsea');
  assert.equal(titleSortKey('Theory of Everything'), 'theory of everything');
});
