// Browser end-to-end check (headless Chromium via Playwright). Not part of `npm test`:
// Playwright is not a project dependency. Run against a running preview server:
//   npm run build && npx vite preview --port 4173 &   (then)   node e2e/live.cjs
// Needs `playwright` resolvable (e.g. `npm i -g playwright` or a local --no-save install).
// Set HILLSING_URL to test a different address.
const { chromium } = require('playwright');
const URL = process.env.HILLSING_URL || 'http://localhost:4173/';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (name, ok, detail = '') => {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? '  — ' + detail : ''}`);
  if (!ok) failures++;
};

(async () => {
  const browser = await chromium.launch({ args: ['--autoplay-policy=no-user-gesture-required'] });
  const page = await browser.newPage();
  const errors = [];
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  page.on('pageerror', (e) => errors.push(String(e)));

  // Fake Web MIDI device so the real MIDI code path runs.
  await page.addInitScript(() => {
    const input = { name: 'Fake Keyboard', onmidimessage: null };
    window.__fakeMidi = (bytes) => input.onmidimessage && input.onmidimessage({ data: new Uint8Array(bytes) });
    const access = { inputs: new Map([['in1', input]]), onstatechange: null };
    navigator.requestMIDIAccess = async () => access;
  });

  await page.goto(URL);
  await page.waitForFunction(() => document.getElementById('dict-status').textContent.startsWith('Dictionary loaded'), null, { timeout: 15000 });
  const chipCount = await page.locator('.syl').count();
  check('dictionary loads and lyrics are syllabified', chipCount >= 12, `${chipCount} syllables for the default text`);
  check('no guessed words in default lyrics', (await page.locator('.word.guessed').count()) === 0);
  check('MIDI status shows the fake device', (await page.textContent('#midi-status')).includes('Fake Keyboard'));

  await page.click('#start');
  await page.waitForFunction(() => document.getElementById('audio-status').textContent.startsWith('Audio on'), null, { timeout: 15000 });
  check('AudioWorklet starts', true, await page.textContent('#audio-status'));

  const level = () => page.evaluate(() => window.__hillsing.peakLevel());
  const nextIndex = () => page.evaluate(() => Number(document.querySelector('.syl.next')?.dataset.index ?? -1));
  const maxLevelOver = async (ms) => { let m = 0; const end = Date.now() + ms; while (Date.now() < end) { m = Math.max(m, await level()); await sleep(15); } return m; };

  check('silent before any key', (await level()) < 0.001);
  check('pointer starts at syllable 0', (await nextIndex()) === 0);

  // Computer keyboard: hold A
  await page.keyboard.down('a');
  const held = await maxLevelOver(400);
  check('holding a key makes sound', held > 0.05, `peak ${held.toFixed(3)}`);
  check('pointer advanced by one', (await nextIndex()) === 1);
  await page.keyboard.up('a');
  await sleep(700);
  check('sound dies after release', (await level()) < 0.005, `level ${(await level()).toFixed(4)}`);

  // Legato: hold A, press S while A held, level must stay up, pointer +1
  await page.keyboard.down('a');
  await sleep(150);
  await page.keyboard.down('s');
  const legato = await maxLevelOver(300);
  check('legato overlap stays audible', legato > 0.05, `peak ${legato.toFixed(3)}`);
  check('legato consumed a syllable', (await nextIndex()) === 3, `next=${await nextIndex()}`);
  await page.keyboard.up('s');
  await page.keyboard.up('a');
  await sleep(600);

  // Navigation
  await page.keyboard.press('Home');
  check('Home resets', (await nextIndex()) === 0);
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('ArrowRight');
  check('ArrowRight skips forward', (await nextIndex()) === 2);
  await page.keyboard.press('ArrowLeft');
  check('ArrowLeft steps back', (await nextIndex()) === 1);
  await page.keyboard.press('Shift+ArrowRight');
  check('Shift+ArrowRight skips a word', (await nextIndex()) === 2);

  // Typing in the textarea must NOT play notes or navigate
  await page.click('#lyrics');
  const before = await nextIndex();
  await page.keyboard.type('asdf');
  await sleep(100);
  check('typing in the lyrics box does not play or move the pointer', (await nextIndex()) === before && (await maxLevelOver(150)) < 0.001, `index ${before} -> ${await nextIndex()}`);
  await page.fill('#lyrics', 'hello world');
  await sleep(400);
  check('editing lyrics re-syllabifies', (await page.locator('.syl').count()) === 3, `${await page.locator('.syl').count()} syllables`);
  await page.click('#nav-reset');

  // Fake MIDI keyboard: note on / pitch bend / mod wheel / note off
  await page.evaluate(() => window.__fakeMidi([0x90, 60, 100]));
  const midiLevel = await maxLevelOver(400);
  check('MIDI note-on makes sound', midiLevel > 0.05, `peak ${midiLevel.toFixed(3)}`);
  await page.evaluate(() => { window.__fakeMidi([0xe0, 0, 100]); window.__fakeMidi([0xb0, 1, 90]); });
  await sleep(150);
  check('still sounding after bend + mod wheel', (await level()) > 0.02);
  await page.evaluate(() => window.__fakeMidi([0x80, 60, 0]));
  await sleep(700);
  check('MIDI note-off ends sound', (await level()) < 0.005);

  // MIDI learn: bind note 36 to "forward", then it navigates and is not sung
  await page.locator('#bindings tr', { hasText: 'Skip forward' }).locator('button', { hasText: 'Learn' }).click();
  await page.evaluate(() => window.__fakeMidi([0x90, 36, 100]));
  await page.evaluate(() => window.__fakeMidi([0x80, 36, 0]));
  check('MIDI learn binds a key', (await page.textContent('#bindings')).includes('note 36'));
  await page.click('#nav-reset');
  await page.evaluate(() => window.__fakeMidi([0x90, 36, 100]));
  await sleep(200);
  check('bound note navigates and is silent', (await nextIndex()) === 1 && (await level()) < 0.005, `next=${await nextIndex()} level=${(await level()).toFixed(4)}`);
  await page.evaluate(() => window.__fakeMidi([0x80, 36, 0]));

  // Latency estimate present
  await sleep(600);
  check('latency estimate shown', (await page.textContent('#latency')).includes('ms'), await page.textContent('#latency'));

  check('no console/page errors', errors.length === 0, errors.join(' | '));
  await page.screenshot({ path: require('path').join(require('os').tmpdir(), 'hillsing-live.png'), fullPage: true });
  await browser.close();
  console.log(failures ? `\n${failures} FAILED` : '\nALL PASSED');
  process.exit(failures ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
