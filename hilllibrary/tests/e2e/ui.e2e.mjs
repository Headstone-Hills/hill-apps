// Browser smoke test (not part of `npm test`; needs Playwright + Chromium).
//   node tests/e2e/ui.e2e.mjs [path/to/fake-camera.y4m] [screenshot-dir]
// The optional .y4m is played as the fake camera feed and should show the barcode
// for ISBN 9780547928227 (The Hobbit) so the scanner pipeline is exercised end to end.
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
const [video, shotDir] = [process.argv[2], process.argv[3]];

const OL = {
  'ISBN:9780441013593': { title: 'Dune', authors: [{ name: 'Frank Herbert' }], publishers: [{ name: 'Ace' }], publish_date: '2005', number_of_pages: 535,
    subjects: [{ name: 'Science fiction' }, { name: 'Fiction' }, { name: 'Ecology' }], classifications: { dewey_decimal_class: ['813/.54'] } },
  'ISBN:9780547928227': { title: 'The Hobbit', authors: [{ name: 'J. R. R. Tolkien' }], publishers: [{ name: 'Mariner' }], publish_date: '2012', number_of_pages: 300,
    subjects: [{ name: 'Fantasy fiction' }, { name: 'Dragons' }, { name: 'Wizards' }, { name: 'Fiction' }] },
};
const fakeFetch = async (url) => {
  const u = String(url);
  const json = (body) => ({ ok: true, status: 200, json: async () => body });
  if (u.includes('openlibrary.org/api/books')) return json(OL);
  if (u.includes('googleapis.com')) return json({ totalItems: 0 });
  return { ok: false, status: 404 };
};

const dataDir = mkdtempSync(join(tmpdir(), 'hl-e2e-'));
const publicDir = join(here, '..', '..', 'public');
const db = new LibraryDb(':memory:');
const server = createServer(createApp({ db, lookup: createLookup({ fetchImpl: fakeFetch }), dataDir, publicDir, fetchImpl: fakeFetch }));
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

const args = ['--use-fake-ui-for-media-stream'];
if (video) args.push('--use-fake-device-for-media-stream', `--use-file-for-fake-video-capture=${video}`);
const browser = await chromium.launch({ args });
const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, permissions: ['camera'] });
const page = await ctx.newPage();
const problems = [];
page.on('console', (m) => { if (['error', 'warning'].includes(m.type())) problems.push(`console.${m.type()}: ${m.text()}`); });
page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));
const shot = async (name) => { if (shotDir) { mkdirSync(shotDir, { recursive: true }); await page.screenshot({ path: join(shotDir, `${name}.png`) }); } };
const step = (s) => console.log('•', s);

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
  await page.getByText('No match found for ISBN 9780140328721.').waitFor();
  await page.getByRole('button', { name: 'Add it manually' }).click();
  assert.equal(await page.locator('#f-isbn').inputValue(), '9780140328721');
  await page.locator('#f-title').fill('Matilda');
  await page.locator('#sheet').getByRole('button', { name: 'Add book' }).click();
  await page.getByText('Added “Matilda”').first().waitFor();

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

  step('desktop layout');
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto(`${base}/#/library`);
  await page.locator('.shelf-head').first().waitFor();
  await shot('7-desktop');

  const real = problems.filter((p) => !/favicon/.test(p));
  assert.deepEqual(real, [], `browser reported problems:\n${real.join('\n')}`);
  console.log('E2E PASSED');
} catch (e) {
  await shot('FAILURE').catch(() => {});
  console.error('E2E FAILED:', e.message, '\n', problems.join('\n'));
  process.exitCode = 1;
} finally {
  await browser.close(); server.close(); db.close();
}
