// Browser end-to-end check (headless Chromium via Playwright). Not part of `npm test`:
// Playwright is not a project dependency. Run against a running preview server:
//   npm run build && npx vite preview --port 4173 &   (then)   node e2e/midi-file.cjs
// Needs `playwright` resolvable (e.g. `npm i -g playwright` or a local --no-save install).
// Set HILLSING_URL to test a different address.
const { chromium } = require('playwright');
const URL = process.env.HILLSING_URL || 'http://localhost:4173/';
const fs = require('fs');
const SP = require('fs').mkdtempSync(require('path').join(require('os').tmpdir(), 'hillsing-e2e-'));

// ---- tiny SMF builder ----
const vlq = (n) => { const o = [n & 127]; while ((n >>= 7) > 0) o.unshift((n & 127) | 128); return o; };
const be32 = (n) => [(n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255];
const trk = (evs) => { const b = []; for (const [d, ...x] of evs) b.push(...vlq(d), ...x); b.push(0, 0xff, 0x2f, 0); return [0x4d, 0x54, 0x72, 0x6b, ...be32(b.length), ...b]; };
const name = (s) => [0, 0xff, 3, s.length, ...[...s].map((c) => c.charCodeAt(0))];
const on = (d, n, ch = 0) => [d, 0x90 | ch, n, 100];
const off = (d, n, ch = 0) => [d, 0x80 | ch, n, 0];
const header = (n) => [0x4d, 0x54, 0x68, 0x64, 0, 0, 0, 6, 0, 1, 0, n, 1, 0xe0]; // format 1, 480 tpq
const tempo = [0, 0xff, 0x51, 3, 0x07, 0xa1, 0x20]; // 500000 us = 120 BPM
const lead = trk([name('Lead Vox'), on(0, 60), off(400, 60), on(560, 64), off(400, 64),   // C4 at 0 s, E4 at 1.0 s (960 ticks)
                  on(560, 67), on(0, 60), off(400, 67), off(0, 60),                          // chord G4+C4 at 2.0 s -> top note G4
                  on(80, 62), off(400, 62)]);                                                // D4 at ~2.5 s
const drums = trk([name('Drums'), on(0, 36, 9), off(100, 36, 9), on(380, 38, 9), off(100, 38, 9)]);
fs.writeFileSync(`${SP}/test.mid`, Buffer.from([...header(3), ...trk([tempo]), ...lead, ...drums]));
fs.writeFileSync(`${SP}/notmidi.mid`, Buffer.from('this is definitely not a midi file at all'));

let failures = 0;
const check = (n, ok, d = '') => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${n}${d ? '  — ' + d : ''}`); if (!ok) failures++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const browser = await chromium.launch({ args: ['--autoplay-policy=no-user-gesture-required'] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 1100 } });
  const errors = [];
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  page.on('pageerror', (e) => errors.push(String(e)));

  await page.goto(URL);
  await page.waitForFunction(() => document.getElementById('dict-status').textContent.startsWith('Dictionary loaded'), null, { timeout: 15000 });
  await page.fill('#lyrics', 'hi ho');
  await sleep(300);

  // Bad file first
  await page.setInputFiles('#file', `${SP}/notmidi.mid`);
  await page.waitForFunction(() => document.getElementById('file-summary').textContent.includes('Could not read'));
  check('non-MIDI file gives a readable error', (await page.textContent('#file-summary')).includes('Not a MIDI file'), await page.textContent('#file-summary'));
  check('play is disabled with no usable file', await page.isDisabled('#play'));

  // Good file
  await page.setInputFiles('#file', `${SP}/test.mid`);
  await page.waitForFunction(() => document.querySelectorAll('#track option').length > 0);
  const opts = await page.$$eval('#track option', (o) => o.map((x) => x.textContent));
  check('only the pitched track is offered (tempo and drum tracks excluded)', opts.length === 1 && opts[0].includes('Lead Vox'), opts.join(' | '));
  check('BPM shown', (await page.textContent('#file-name')).includes('120 BPM'), await page.textContent('#file-name'));
  const summary = await page.textContent('#file-summary');
  check('chord reduced, extra notes reported', summary.includes('2 of 4 notes sung') && summary.includes('2 notes have no syllable'), summary);
  check('play enabled', !(await page.isDisabled('#play')));

  // Sample the audio level in-page at ~4 ms resolution while playing
  const sampleLevels = (ms) => page.evaluate((ms) => new Promise((resolve) => {
    const out = []; const t0 = performance.now();
    const id = setInterval(() => {
      const t = performance.now() - t0;
      out.push([t, window.__hillsing ? window.__hillsing.peakLevel() : -1]);
      if (t >= ms) { clearInterval(id); resolve(out); }
    }, 4);
  }), ms);
  const onsets = (s, thr = 0.05) => { const r = []; let up = false; for (const [t, l] of s) { if (!up && l > thr) { up = true; r.push(t); } else if (up && l < thr * 0.2) up = false; } return r; };

  // Play: audio starts from this click (user gesture)
  await page.click('#play');
  const samples = await sampleLevels(3400);
  const ons = onsets(samples);
  check('playback makes sound', ons.length >= 2, `onsets at ${ons.map((t) => t.toFixed(0)).join(', ')} ms`);
  if (ons.length >= 2) {
    const dt = (ons[1] - ons[0]) / 1000;
    check('notes 1 s apart in the file are ~1 s apart in the audio', Math.abs(dt - 1.0) < 0.12, `${dt.toFixed(3)} s`);
  }
  check('only two syllables are sung (lyrics "hi ho")', ons.length === 2, `${ons.length} onsets`);
  check('button shows Play again when the sequence has finished', (await page.textContent('#play')).includes('Play'), await page.textContent('#play'));
  const parseClock = (txt) => txt.split('/').map((x) => { const [m, sec] = x.trim().split(':'); return Number(m) * 60 + Number(sec); });
  const [cur0, len0] = parseClock(await page.textContent('#time'));
  check('time label never exceeds the total, and reached the end', cur0 <= len0 + 1e-9 && cur0 > 1.0, await page.textContent('#time'));

  // Pause / resume / stop
  await page.click('#play');
  await sleep(1300);                    // well into note 2
  await page.click('#play');           // pause
  await sleep(500);
  const quiet = Math.max(...(await sampleLevels(150)).map((x) => x[1]));
  check('pause silences output', quiet < 0.01, `peak ${quiet.toFixed(4)}`);
  const t1 = await page.textContent('#time');
  check('paused at a non-zero position', parseClock(t1)[0] > 0.8, t1);
  await sleep(500);
  check('position holds while paused', (await page.textContent('#time')) === t1, t1);
  await page.click('#play');           // resume
  await sleep(400);
  check('resume continues from the paused position', parseClock(await page.textContent('#time'))[0] > parseClock(t1)[0], await page.textContent('#time'));
  await page.click('#stop');
  await sleep(300);
  check('stop rewinds to the start', parseClock(await page.textContent('#time'))[0] === 0, await page.textContent('#time'));

  // Tempo: 50% => notes 2 s apart. Change tempo before playing.
  await page.fill('#tempo', '50');
  await page.dispatchEvent('#tempo', 'input');
  await page.click('#play');
  const slow = onsets(await sampleLevels(4200));
  check('50% tempo doubles the spacing', slow.length >= 2 && Math.abs((slow[1] - slow[0]) / 1000 - 2.0) < 0.15, slow.map((t) => t.toFixed(0)).join(', '));
  await page.click('#stop');
  await page.fill('#tempo', '100');
  await page.dispatchEvent('#tempo', 'input');

  // Repeat lyrics => all 4 notes sung
  await page.check('#repeat');
  check('repeat lyrics sings every note', (await page.textContent('#file-summary')).includes('4 of 4 notes sung'), await page.textContent('#file-summary'));
  await page.click('#play');
  const trace = await sampleLevels(3300);
  // Anchor on the first onset; notes start at 0, 1.0, 2.0 and 2.5 s (score), each ~0.4 s long.
  const a0 = onsets(trace)[0];
  const at = (rel) => Math.max(...trace.filter(([t]) => t >= a0 + rel * 1000 && t < a0 + (rel + 0.06) * 1000).map((x) => x[1]));
  const loud = [0.2, 1.2, 2.2, 2.7].map(at);
  check('all four notes are sounding in their windows (repeat on)', loud.every((l) => l > 0.1), loud.map((l) => l.toFixed(2)).join(', '));
  const gaps = [0.65, 1.65].map(at);
  check('and silent in the gaps between them', gaps.every((l) => l < 0.01), gaps.map((l) => l.toFixed(3)).join(', '));
  await page.click('#stop');

  // Seek by clicking the piano-roll ruler (the playhead moves, and the time readout follows)
  await page.uncheck('#repeat');
  const box = await page.locator('#preview').boundingBox();
  await page.mouse.click(box.x + box.width * 0.6, box.y + 10); // y = 10 is inside the 22 px ruler
  await sleep(300);
  const [seekPos] = parseClock(await page.textContent('#time'));
  check('clicking the ruler moves the playhead', seekPos > 0.3, await page.textContent('#time'));

  // Editing: double-click adds a note, Delete removes it, and Undo brings it back
  const countNotes = async () => Number(/^(\d+) of (\d+) notes/.exec(await page.textContent('#file-summary'))[2]);
  const before = await countNotes();
  await page.mouse.dblclick(box.x + box.width * 0.9, box.y + box.height / 2);
  await sleep(200);
  check('double-clicking the roll adds a note', (await countNotes()) === before + 1, String(await countNotes()));
  await page.keyboard.press('Delete');
  await sleep(200);
  check('Delete removes the selected note', (await countNotes()) === before, String(await countNotes()));
  await page.keyboard.press('Meta+z');
  await page.keyboard.press('Control+z');
  await sleep(200);
  check('Undo restores it', (await countNotes()) === before + 1, String(await countNotes()));

  // Live play still works alongside, and everything else from before
  await page.keyboard.press('Home');
  await page.keyboard.down('a');
  const live = Math.max(...(await sampleLevels(300)).map((x) => x[1]));
  await page.keyboard.up('a');
  check('live keyboard still plays', live > 0.05, `peak ${live.toFixed(3)}`);

  await page.screenshot({ path: `${SP}/hillsing-midi.png`, fullPage: true });
  check('no console/page errors', errors.length === 0, errors.join(' | '));
  await browser.close();
  console.log(failures ? `\n${failures} FAILED` : '\nALL PASSED');
  process.exit(failures ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
