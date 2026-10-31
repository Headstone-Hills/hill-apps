// Browser end-to-end check of the Voice Lab (headless Chromium via Playwright). See the header
// of e2e/live.cjs for how to run. Set HILLSING_URL to test a different address.
const { chromium } = require('playwright');
const URL = process.env.HILLSING_URL || 'http://localhost:4173/';
const SP = require('fs').mkdtempSync(require('path').join(require('os').tmpdir(), 'hillsing-e2e-'));

let failures = 0;
const check = (n, ok, d = '') => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${n}${d ? '  — ' + d : ''}`); if (!ok) failures++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const browser = await chromium.launch({ args: ['--autoplay-policy=no-user-gesture-required'] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 1300 } });
  const errors = [];
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  page.on('pageerror', (e) => errors.push(String(e)));

  await page.goto(URL);
  await page.waitForFunction(() => document.getElementById('dict-status').textContent.startsWith('Dictionary loaded'), null, { timeout: 15000 });

  const voices = await page.$$eval('#voice option', (o) => o.map((x) => x.textContent));
  check('at least six voices are offered', voices.length >= 6, voices.join(', '));
  const auditions = await page.$$eval('#audition-buttons button[data-audition]', (b) => b.map((x) => x.dataset.audition));
  check('all six listening phrases are offered', auditions.length === 6, auditions.join(', '));

  const level = () => page.evaluate(() => (window.__hillsing ? window.__hillsing.peakLevel() : -1));
  const maxLevel = async (ms) => { let m = 0; const end = Date.now() + ms; while (Date.now() < end) { m = Math.max(m, await level()); await sleep(15); } return m; };

  // Auditioning is itself the user gesture that starts audio.
  await page.click('[data-audition="vowels"]');
  await page.waitForFunction(() => document.getElementById('audio-status').textContent.startsWith('Audio on'), null, { timeout: 15000 });
  const peak = await maxLevel(1500);
  check('the vowels phrase makes sound', peak > 0.1, `peak ${peak.toFixed(3)}`);
  check('its button shows as playing', await page.$eval('[data-audition="vowels"]', (b) => b.classList.contains('armed')));
  check('and it stays well under clipping', peak < 0.95, `peak ${peak.toFixed(3)}`);

  // Stop
  await page.click('#audition-buttons button:has-text("Stop")');
  await sleep(500);
  check('stop silences it and clears the playing mark', (await level()) < 0.01 && !(await page.$eval('[data-audition="vowels"]', (b) => b.classList.contains('armed'))));

  // Every phrase plays and makes sound, with each voice
  for (const id of auditions) {
    await page.click(`[data-audition="${id}"]`);
    const p = await maxLevel(1800);
    check(`phrase "${id}" is audible`, p > 0.05 && p < 0.97, `peak ${p.toFixed(3)}`);
    await page.click('#audition-buttons button:has-text("Stop")');
    await sleep(150);
  }
  const voiceIds = await page.$$eval('#voice option', (o) => o.map((x) => x.value));
  for (const v of voiceIds) {
    await page.selectOption('#voice', v);
    await page.click('[data-audition="words"]');
    const p = await maxLevel(1500);
    check(`voice "${v}" sings the sentence audibly and unclipped`, p > 0.05 && p < 0.97, `peak ${p.toFixed(3)}`);
    await page.click('#audition-buttons button:has-text("Stop")');
    await sleep(120);
  }
  check('voice description is shown', (await page.textContent('#voice-desc')).length > 10);

  // Voice persists across reload
  await page.selectOption('#voice', 'female');
  await page.reload();
  await page.waitForFunction(() => document.getElementById('dict-status').textContent.startsWith('Dictionary loaded'));
  check('the chosen voice is remembered after reload', (await page.inputValue('#voice')) === 'female');

  // Sliders and settings text
  await page.fill('#bright', '0.5'); await page.dispatchEvent('#bright', 'input');
  await page.fill('#conlevel', '1.5'); await page.dispatchEvent('#conlevel', 'input');
  check('slider read-outs update', (await page.textContent('#out-bright')).includes('+0.50') && (await page.textContent('#out-conlevel')).includes('1.50'));
  await page.click('#copy-settings');
  const txt = await page.inputValue('#settings-out');
  const parsed = JSON.parse(txt);
  check('settings text is valid JSON with the voice and sliders', parsed.voice === 'female' && parsed.brightness === 0.5 && parsed.consonantLevel === 1.5, txt.replace(/\s+/g, ' ').slice(0, 160));

  // The spectrogram canvas draws something while a phrase plays
  await page.click('[data-audition="consonants"]');
  await sleep(2500);
  const lit = await page.$eval('#spectrogram', (c) => {
    const g = c.getContext('2d');
    const d = g.getImageData(c.width - 40, 0, 30, c.height).data;
    let bright = 0;
    for (let i = 0; i < d.length; i += 4) if (d[i] + d[i + 1] > 150) bright++;
    return bright;
  });
  check('the spectrogram shows live energy', lit > 50, `${lit} bright pixels`);
  await page.locator('#lab').screenshot({ path: require('path').join(SP, 'lab.png') });
  await page.click('#audition-buttons button:has-text("Stop")');

  // Live play still works after auditions, in the selected voice
  await page.keyboard.press('Home');
  await page.keyboard.down('a');
  const live = await maxLevel(400);
  await page.keyboard.up('a');
  check('live keyboard still plays after auditions', live > 0.05, `peak ${live.toFixed(3)}`);

  check('no console/page errors', errors.length === 0, errors.join(' | '));
  console.log('screenshot dir:', SP);
  await browser.close();
  console.log(failures ? `\n${failures} FAILED` : '\nALL PASSED');
  process.exit(failures ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
