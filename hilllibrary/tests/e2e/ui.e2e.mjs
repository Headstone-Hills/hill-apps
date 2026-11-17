// Browser smoke test (not part of `npm test`; needs Playwright + Chromium).
//   node tests/e2e/ui.e2e.mjs [path/to/fake-camera.y4m] [screenshot-dir] [path/to/barcode-photo.jpg]
// The optional .y4m is played as the fake camera feed and should show the barcode
// for ISBN 9780547928227 (The Hobbit) so the scanner pipeline is exercised end to end. The optional photo
// should be a still image of the same barcode, for the "Scan from photo" path.
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { mkdtempSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { LibraryDb } from '../../server/db.js';
import { createApp } from '../../server/app.js';
import { createLookup } from '../../server/metadata.js';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
const here = dirname(fileURLToPath(import.meta.url));
const [video, shotDir, barcodePhoto] = [process.argv[2], process.argv[3], process.argv[4]];

const OL = {
  'ISBN:9780441013593': { title: 'Dune', authors: [{ name: 'Frank Herbert' }], publishers: [{ name: 'Ace' }], publish_date: '2005', number_of_pages: 535,
    subjects: [{ name: 'Science fiction' }, { name: 'Fiction' }, { name: 'Ecology' }], classifications: { dewey_decimal_class: ['813/.54'] } },
  'ISBN:9780547928227': { title: 'The Hobbit', authors: [{ name: 'J. R. R. Tolkien' }], publishers: [{ name: 'Mariner' }], publish_date: '2012', number_of_pages: 300,
    subjects: [{ name: 'Fantasy fiction' }, { name: 'Dragons' }, { name: 'Wizards' }, { name: 'Fiction' }] },
  'ISBN:9780000001009': { title: 'Searchable Wonder', authors: [{ name: 'Ada Writer' }], publishers: [{ name: 'Wonder Press' }], publish_date: '2001',
    subjects: [{ name: 'Fantasy fiction' }, { name: 'Dragons' }, { name: 'Fiction' }] },
  // A record with no title: the providers know *something*, but it is not a match.
  'ISBN:9780743273565': { publishers: [{ name: 'Scribner' }], number_of_pages: 180, authors: [{ name: 'F. Scott Fitzgerald' }],
    subjects: [{ name: 'Rich people' }, { name: 'Long Island (N.Y.)' }, { name: 'Fiction' }] },
};
// Title-search responses (shapes follow the providers' documented search formats).
const OL_SEARCH = { docs: [
  { title: 'Searchable Wonder', author_name: ['Ada Writer'], first_publish_year: 2001, isbn: ['9780000001009'], cover_i: 1, publisher: ['Wonder Press'] },
  { title: 'Searchable Mystery (no isbn)', author_name: ['Bo Writer'], first_publish_year: 1995, cover_i: 2, publisher: ['Old Press'] },
] };
const GB_SEARCH = { items: [{ volumeInfo: { title: 'Searchable Wonder', authors: ['Ada Writer'], publishedDate: '2001',
  industryIdentifiers: [{ type: 'ISBN_13', identifier: '9780000001009' }] } }] };
const FLAKY_ISBN = '9780000001016'; // every provider answers HTTP 500 for this one: a simulated outage
const fakeFetch = async (url) => {
  const u = String(url);
  const json = (body) => ({ ok: true, status: 200, json: async () => body });
  if (u.includes(FLAKY_ISBN)) return { ok: false, status: 500, json: async () => ({}) };
  if (u.includes('openlibrary.org/search.json')) return json(/searchable/i.test(decodeURIComponent(u)) ? OL_SEARCH : { docs: [] });
  if (u.includes('googleapis.com/books/v1/volumes') && !u.includes('q=isbn:')) return json(/searchable/i.test(decodeURIComponent(u)) ? GB_SEARCH : { totalItems: 0 });
  if (u.includes('openlibrary.org/api/books')) return json(OL);
  if (u.includes('googleapis.com')) return json({ totalItems: 0 });
  return { ok: false, status: 404 };
};

const dataDir = mkdtempSync(join(tmpdir(), 'hl-e2e-'));
const publicDir = join(here, '..', '..', 'public');
const db = new LibraryDb(':memory:');
const handler = createApp({ db, lookup: createLookup({ fetchImpl: fakeFetch }), dataDir, publicDir, fetchImpl: fakeFetch });
const server = createServer(handler);
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

const args = ['--use-fake-ui-for-media-stream'];
if (video) args.push('--use-fake-device-for-media-stream', `--use-file-for-fake-video-capture=${video}`);
const browser = await chromium.launch({ args });
const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, permissions: ['camera'] });
await ctx.addInitScript(() => {
  window.__gum = 0;
  const md = navigator.mediaDevices;
  if (md?.getUserMedia) { const orig = md.getUserMedia.bind(md); md.getUserMedia = (...a) => { window.__gum++; return orig(...a); }; }
});
const PNG_1PX = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==', 'base64');
await ctx.route(/^https:\/\/(covers\.openlibrary\.org|books\.google\.com)\//, (route) => route.fulfill({ contentType: 'image/png', body: PNG_1PX }));
const page = await ctx.newPage();
const problems = [];
page.on('console', (m) => { if (['error', 'warning'].includes(m.type())) problems.push(`console.${m.type()}: ${m.text()}`); });
page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));
const shot = async (name) => { if (shotDir) { mkdirSync(shotDir, { recursive: true }); await page.screenshot({ path: join(shotDir, `${name}.png`) }); } };
const step = (s) => console.log('•', s);
// Covers use loading="lazy", so an off-screen card's image doesn't load until it is scrolled near.
const coverRenders = async (img) => {
  await img.scrollIntoViewIfNeeded();
  return img.evaluate(async (el) => { try { await el.decode(); } catch { return false; } return el.naturalWidth > 0; });
};

try {
  step('empty library');
  await page.goto(base);
  await page.getByText('Your library is empty.').waitFor();
  await shot('1-empty');

  step('manual ISBN -> preview -> add');
  await page.locator('.nav').getByRole('link', { name: 'Scan' }).click();
  await page.locator('#isbn-in').fill('978-0-441-01359-3');
  await page.getByRole('button', { name: 'Look up' }).click();
  await page.getByRole('heading', { name: 'Dune' }).waitFor();
  await page.locator('.preview .pill.genre', { hasText: 'Science Fiction' }).waitFor();
  await shot('2-preview');
  await page.getByRole('button', { name: 'Add to library' }).click();
  await page.getByText('Added “Dune”').first().waitFor();

  step('shelf view groups by genre');
  await page.locator('.nav').getByRole('link', { name: 'Library' }).click();
  await page.locator('.shelf-head h2', { hasText: 'Science Fiction' }).waitFor();
  await page.locator('.book', { hasText: 'Dune' }).waitFor();
  await shot('3-library');

  step('search (prefix, no results)');
  await page.getByLabel('Search your library').fill('arrak');   // not in title; proves description/tags are searched only if present
  await page.getByText('No books match.').waitFor();
  await page.getByLabel('Search your library').fill('ecolo');   // tag
  await page.locator('.book', { hasText: 'Dune' }).waitFor();
  await page.getByLabel('Search your library').fill('frank her');
  await page.locator('.book', { hasText: 'Dune' }).waitFor();
  await page.getByLabel('Search your library').fill('');

  step('browse -> genre filter');
  await page.locator('.nav').getByRole('link', { name: 'Browse' }).click();
  await page.locator('.genre-card a', { hasText: 'Science Fiction' }).click();
  await page.locator('.chip.active', { hasText: 'Science Fiction' }).waitFor();
  await page.locator('.book', { hasText: 'Dune' }).waitFor();
  await shot('4-browse-filter');

  step('edit dialog: set genre, locks it');
  await page.locator('.book', { hasText: 'Dune' }).click();
  await page.locator('#sheet').waitFor({ state: 'visible' });
  await page.locator('#f-genre').selectOption('Classics');
  await page.locator('#sheet').getByRole('button', { name: 'Save' }).click();
  await page.getByText('Saved').first().waitFor();
  await page.goto(`${base}/#/library`);
  await page.locator('.shelf-head h2', { hasText: 'Classics' }).waitFor();
  await page.locator('.book', { hasText: 'Dune' }).click();
  await page.getByText('genre set by you').waitFor();
  await shot('5-dialog');
  await page.keyboard.press('Escape');

  step('unknown ISBN offers manual entry');
  await page.goto(`${base}/#/scan`);
  await page.locator('#isbn-in').fill('9780140328721');
  await page.getByRole('button', { name: 'Look up' }).click();
  await page.getByText('No match for ISBN 9780140328721.').waitFor();
  await page.getByText('The databases have nothing for this ISBN.').waitFor();
  await page.getByRole('button', { name: 'Add without photo' }).click();
  assert.equal(await page.locator('#f-isbn').inputValue(), '9780140328721');
  await page.locator('#f-title').fill('Matilda');
  await page.locator('#sheet').getByRole('button', { name: 'Add book' }).click();
  await page.getByText('Added “Matilda”').first().waitFor();

  step('provider outage: the reason is on screen, retries really retry, and later lookups still work');
  await page.goto(`${base}/#/scan`);
  await page.locator('#isbn-in').fill(FLAKY_ISBN);
  await page.getByRole('button', { name: 'Look up' }).click();
  await page.getByText('Could not reach the book databases').waitFor();
  await page.getByText('Open Library: HTTP 500').waitFor();
  await page.getByText('Google Books: HTTP 500').waitFor();
  await page.getByRole('button', { name: 'Try again' }).click();                  // 2nd failure: both providers get paused
  await page.getByText('Open Library: HTTP 500').waitFor();
  await page.getByRole('button', { name: 'Try again' }).click();                  // 3rd: both are paused, but must still be ASKED, not skipped
  await page.getByText('Open Library: HTTP 500').waitFor();
  assert.ok(!/paused/.test(await page.locator('#app').innerText()), 'a paused provider must be re-asked when nothing else answered');
  await page.getByRole('button', { name: 'Cancel' }).click();

  step('no match + partial info: photo becomes the cover, partial data pre-fills the form');
  const jpeg = await page.screenshot({ type: 'jpeg', quality: 80 }); // a real JPEG to act as the "photo"
  await page.goto(`${base}/#/scan`);
  await page.locator('#isbn-in').fill('9780743273565');
  await page.getByRole('button', { name: 'Look up' }).click();
  await page.getByText('No match for ISBN 9780743273565.').waitFor();
  await page.getByText('publisher Scribner').waitFor();
  await page.getByText('180 pages').waitFor();
  await shot('8-no-match');
  const [chooser] = await Promise.all([page.waitForEvent('filechooser'), page.getByRole('button', { name: 'Take cover photo & add' }).click()]);
  await chooser.setFiles({ name: 'photo.jpg', mimeType: 'image/jpeg', buffer: jpeg });
  await page.locator('#sheet').waitFor({ state: 'visible' });
  await page.locator('#sheet img[alt="New cover photo"]').waitFor();             // photo previewed in the form
  assert.equal(await page.locator('#f-isbn').inputValue(), '9780743273565');
  assert.equal(await page.locator('#f-pub').inputValue(), 'Scribner');             // partial data pre-filled
  assert.equal(await page.locator('#f-pages').inputValue(), '180');
  assert.equal(await page.locator('#f-auth').inputValue(), 'F. Scott Fitzgerald');
  await shot('9-photo-form');
  await page.locator('#f-title').fill('The Great Gatsby');
  await page.locator('#sheet').getByRole('button', { name: 'Add book' }).click();
  await page.getByText('Added “The Great Gatsby”').first().waitFor();
  await page.goto(`${base}/#/library`);
  const gatsbyImg = page.locator('.book', { hasText: 'The Great Gatsby' }).locator('img');
  await gatsbyImg.waitFor();
  assert.match(await gatsbyImg.getAttribute('src'), /^\/covers\/\d+-[a-z0-9]+\.jpg$/);
  assert.ok(await coverRenders(gatsbyImg), 'uploaded cover should actually render');
  const stored = db.findByIsbn('9780743273565');
  assert.equal(stored.publisher, 'Scribner'); assert.equal(stored.pages, 180);       // data really saved, not just shown

  step('title search: results, preview and add');
  await page.goto(`${base}/#/scan`);
  await page.locator('#title-in').fill('searchable');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await page.locator('.result', { hasText: 'Searchable Wonder' }).waitFor();
  await page.locator('.result', { hasText: 'Searchable Mystery (no isbn)' }).waitFor();
  assert.equal(await page.locator('.result').count(), 2, 'the two providers\' copies of "Searchable Wonder" should be merged into one row');
  await page.waitForTimeout(900); // let the smooth scroll finish
  assert.ok(await page.locator('.result').first().isVisible() && (await page.locator('.result').first().boundingBox()).y < 844, 'results should be on screen after searching, not below the fold');
  await shot('10-title-search');
  await page.locator('.result', { hasText: 'Searchable Wonder' }).getByRole('button', { name: 'Preview' }).click();
  await page.locator('.preview h2', { hasText: 'Searchable Wonder' }).waitFor();          // the normal ISBN preview, with full metadata
  await page.locator('.preview .pill.genre', { hasText: 'Fantasy' }).waitFor();
  await page.getByRole('button', { name: 'Add to library' }).click();
  await page.getByText('Added “Searchable Wonder”').first().waitFor();

  step('title search: a book already on the shelf is marked, and opens instead of re-adding');
  await page.locator('#title-in').fill('searchable');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  const owned = page.locator('.result', { hasText: 'Searchable Wonder' });
  await owned.locator('.pill.ok', { hasText: 'In your library' }).waitFor();
  await owned.getByRole('button', { name: 'View' }).click();
  await page.locator('#sheet').waitFor({ state: 'visible' });
  assert.equal(await page.locator('#f-title').inputValue(), 'Searchable Wonder');
  await page.keyboard.press('Escape');

  step('title search: a result with no ISBN opens a pre-filled form and sends its cover URL');
  await page.locator('.result', { hasText: 'Searchable Mystery' }).getByRole('button', { name: 'Add details' }).click();
  await page.locator('#sheet').waitFor({ state: 'visible' });
  assert.equal(await page.locator('#f-title').inputValue(), 'Searchable Mystery (no isbn)');
  assert.equal(await page.locator('#f-auth').inputValue(), 'Bo Writer');
  assert.equal(await page.locator('#f-pub').inputValue(), 'Old Press');
  await page.locator('#sheet img[alt="Cover from the search result"]').waitFor();
  const [post] = await Promise.all([
    page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith('/api/books')),
    page.locator('#sheet').getByRole('button', { name: 'Add book' }).click(),
  ]);
  assert.equal(JSON.parse(post.postData()).cover_url, 'https://covers.openlibrary.org/b/id/2-M.jpg');
  await page.getByText('Added “Searchable Mystery (no isbn)”').first().waitFor();
  await handler.idle();
  const mystery = db.all().find((b) => b.title.startsWith('Searchable Mystery'));
  assert.equal(mystery.isbn13, null); assert.equal(mystery.publisher, 'Old Press');

  step('title search: nothing found is explained, with a way to add manually');
  await page.locator('#title-in').fill('qqqqzzzz');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await page.getByText('No books found for “qqqqzzzz”.').waitFor();

  step('entry points: Library "Find a book", and "Search online" from an empty library search');
  await page.goto(`${base}/#/library`);
  await page.getByRole('link', { name: 'Find a book' }).click();
  await page.locator('#title-in').waitFor();
  assert.equal(await page.evaluate(() => document.activeElement?.id), 'title-in', 'the title box should be focused');
  await page.goto(`${base}/#/library`);
  await page.getByLabel('Search your library').fill('nothingmatches');
  await page.getByRole('link', { name: /Search online for/ }).click();
  assert.equal(await page.locator('#title-in').inputValue(), 'nothingmatches');
  await page.getByText('No books found for “nothingmatches”.').waitFor();

  step('replace the cover of an existing book from its details');
  await page.goto(`${base}/#/library`);
  await page.locator('.book', { hasText: 'Matilda' }).click();
  await page.locator('#sheet').waitFor({ state: 'visible' });
  const [chooser2] = await Promise.all([page.waitForEvent('filechooser'), page.locator('#sheet').getByRole('button', { name: 'Choose image' }).click()]);
  await chooser2.setFiles({ name: 'new.jpg', mimeType: 'image/jpeg', buffer: jpeg });
  await page.locator('#sheet img[alt="New cover photo"]').waitFor();
  await page.locator('#sheet').getByRole('button', { name: 'Save' }).click();
  await page.getByText('Saved').first().waitFor();
  const matildaImg = page.locator('.book', { hasText: 'Matilda' }).locator('img');
  await matildaImg.waitFor();
  assert.ok(await coverRenders(matildaImg), 'replaced cover should actually render');

  if (barcodePhoto) {
    step('scan from photo: reads the barcode from a still, with no live camera stream');
    await page.goto(`${base}/#/scan`);
    const [fc] = await Promise.all([page.waitForEvent('filechooser'), page.getByRole('button', { name: 'Scan from photo' }).click()]);
    await fc.setFiles(barcodePhoto);
    await page.getByRole('heading', { name: 'The Hobbit' }).waitFor({ timeout: 20000 });
    await page.locator('.preview .pill.genre', { hasText: 'Fantasy' }).waitFor();
    assert.equal(await page.evaluate(() => window.__gum), 0, 'photo scanning must not call getUserMedia');
    await page.getByRole('button', { name: 'Skip' }).click();
    // a photo with no barcode in it is reported clearly instead of hanging or guessing
    const [fc2] = await Promise.all([page.waitForEvent('filechooser'), page.getByRole('button', { name: 'Scan from photo' }).click()]);
    await fc2.setFiles({ name: 'nothing.jpg', mimeType: 'image/jpeg', buffer: jpeg });
    await page.getByText('No barcode found in that photo.').waitFor({ timeout: 20000 });
    assert.equal(await page.evaluate(() => window.__gum), 0);
  }

  if (video) {
    step('camera scan decodes the barcode in the video feed');
    await page.goto(`${base}/#/scan`);
    await page.getByRole('button', { name: 'Start camera' }).click();
    const t0 = Date.now();
    await page.getByRole('heading', { name: 'The Hobbit' }).waitFor({ timeout: 20000 });
    console.log(`  decoded + looked up in ${Date.now() - t0} ms`);
    await page.locator('.preview .pill.genre', { hasText: 'Fantasy' }).waitFor();
    await shot('6-camera');
    await page.getByRole('button', { name: 'Add to library' }).click();
    await page.getByText('Added “The Hobbit”').first().waitFor();
  }

  step('no stray "null"/"undefined" text on any main screen');
  for (const route of ['library', 'scan', 'browse', 'more']) {
    await page.goto(`${base}/#/${route}`);
    await page.locator('.page h1').waitFor();
    const text = await page.locator('.page').innerText();
    assert.ok(!/\b(null|undefined|NaN)\b/.test(text), `"${route}" screen shows stray text: ${text.match(/.{0,30}\b(null|undefined|NaN)\b.{0,30}/)?.[0]}`);
  }

  step('no stray text in the book dialog either (add mode and edit mode)');
  await page.goto(`${base}/#/library`);
  await page.getByRole('button', { name: '+ Add manually' }).click();
  await page.locator('#sheet').waitFor({ state: 'visible' });
  const addText = await page.locator('#sheet').innerText();
  assert.ok(!/\b(null|undefined|NaN)\b/.test(addText), `add dialog shows stray text: ${addText.match(/.{0,30}\b(null|undefined|NaN)\b.{0,30}/)?.[0]}`);
  await page.keyboard.press('Escape');
  await page.locator('.book').first().click();
  await page.locator('#sheet').waitFor({ state: 'visible' });
  const editText = await page.locator('#sheet').innerText();
  assert.ok(!/\b(null|undefined|NaN)\b/.test(editText), `edit dialog shows stray text: ${editText.match(/.{0,30}\b(null|undefined|NaN)\b.{0,30}/)?.[0]}`);
  await page.keyboard.press('Escape');

  step('desktop layout');
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto(`${base}/#/library`);
  await page.locator('.shelf-head').first().waitFor();
  await shot('7-desktop');

  const real = problems.filter((p) => !/favicon/.test(p) && !/status of 502/.test(p)); // the 502s are the simulated outage
  assert.deepEqual(real, [], `browser reported problems:\n${real.join('\n')}`);
  console.log('E2E PASSED');
} catch (e) {
  await shot('FAILURE').catch(() => {});
  console.error('E2E FAILED:', e.message, '\n', problems.join('\n'));
  process.exitCode = 1;
} finally {
  await browser.close(); server.close(); db.close();
}
