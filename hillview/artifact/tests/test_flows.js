const { chromium } = require('playwright');
const S = process.argv[2]; let fails = 0; const ok = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) fails++; };
const mad = (a, b) => { let s = 0, n = 0; for (let i = 0; i < a.length; i++) if (i % 4 < 3) { s += Math.abs(a[i] - b[i]); n++; } return s / n; };
(async () => {
  const b = await chromium.launch({ args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist', '--autoplay-policy=no-user-gesture-required'] });
  const ctx = await b.newContext({ viewport: { width: 1400, height: 850 } });
  await ctx.addInitScript(() => {
    window.__saved = [];
    window.claude = { use: async n => n === 'downloads' ? { save: async r => {
      let size, head = '', text = null;
      if (r.data instanceof Blob) { size = r.data.size; head = Array.from(new Uint8Array(await r.data.slice(0, 8).arrayBuffer())).join(','); } else { size = r.data.length; head = r.data.slice(0, 8); text = r.data; }
      window.__saved.push({ filename: r.filename, size, head, text }); return { status: 'saved' };
    } } : null };
  });
  const p = await ctx.newPage(); const errs = [];
  p.on('pageerror', e => errs.push('pageerror: ' + e.message)); p.on('console', m => { if (m.type() === 'error') errs.push('console: ' + m.text()); });
  await p.goto('file://' + S + '/index.html');
  await p.waitForFunction(() => window.hillview && hillview.source() && hillview.format, null, { timeout: 20000 });

  // ---- first frame is a real, non-blank, animated picture before any interaction
  const snap = () => p.evaluate(() => hillview.readCanvas());
  const f1 = await snap(); await p.waitForTimeout(700); const f2 = await snap();
  const distinct = new Set(); for (let i = 0; i < f1.length; i += 4 * 97) distinct.add(f1[i] + ',' + f1[i + 1] + ',' + f1[i + 2]);
  ok(distinct.size > 4, `opening state is a rich picture (${distinct.size} sampled colours)`);
  ok(mad(f1, f2) > 0.5, `demo animates by itself (mean change ${mad(f1, f2).toFixed(2)} levels in 0.7 s)`);
  const m1 = await p.evaluate(() => [...document.querySelectorAll('.meter i')].map(i => +i.style.getPropertyValue('--v')));
  await p.waitForTimeout(250); const m2 = await p.evaluate(() => [...document.querySelectorAll('.meter i')].map(i => +i.style.getPropertyValue('--v')));
  ok(m1.some((v, i) => Math.abs(v - m2[i]) > 0.02), 'audio meters move with the demo beat before any sound is enabled');
  await p.screenshot({ path: S + '/shot_desktop.png' });

  // ---- frame sampling: interpolation is a real cross-fade of the two keyframes, hold is a hard hold
  const cfg = interp => ({ quantize: { enabled: true, mode: 'posterize', levels: 5 }, smooth: { sigma: 2 }, sampling: { step: 4, interp } });
  await p.evaluate(c => { hillview.loadConfig(c); }, cfg('blend'));
  const at = async idx => { await p.evaluate(i => hillview.pauseAt(i / 30 + 0.001), idx); return snap(); };
  await at(16); const O20 = await at(20), O22 = await at(22), O23 = await at(23), O24 = await at(24);
  const mid = O20.map((v, i) => (v + O24[i]) / 2);
  ok(mad(O22, mid) < 1.6, `step 4 blend: frame 2 of 4 is the midpoint of its neighbours (error ${mad(O22, mid).toFixed(2)} levels)`);
  ok(mad(O20, O24) > 3 && mad(O22, O20) > 1 && mad(O23, O22) > 0.5, 'blend frames differ from each other (not frozen)');
  await p.evaluate(c => hillview.loadConfig(c), cfg('hold'));
  await at(16); const H20 = await at(20), H22 = await at(22), H23 = await at(23);
  ok(mad(H20, H22) === 0 && mad(H22, H23) === 0, 'step 4 hold: the three in-between frames are identical');
  // changing a control while paused mid-interval must not leave a cross-fade with stale settings
  await p.evaluate(c => hillview.loadConfig(c), cfg('blend'));
  await at(16); await at(20); const before22 = await at(22);
  await p.evaluate(() => hillview.loadConfig({ quantize: { enabled: true, mode: 'posterize', levels: 2 }, smooth: { sigma: 0 }, sampling: { step: 4, interp: 'blend' } }));
  const after22 = await snap(), after20 = await at(20);
  ok(mad(before22, after22) > 3 && mad(after22, after20) === 0, 'paused control change shows the new settings fully (no stale cross-fade)');

  // ---- UI smoke: rules, routes, presets, view toggle
  await p.click('summary:has-text("Advanced controls")');
  await p.click('#addRule'); await p.click('#addMod');
  await p.selectOption('#mods .card:last-child .ty', 'audio'); await p.selectOption('#mods .card:last-child .tg', 'remap.threshold');
  ok(await p.evaluate(() => { const m = hillview.getState().modulation.at(-1); return m.source.type === 'audio' && m.target === 'remap.threshold'; }), 'new modulation route can be retargeted');
  await p.click('summary:has-text("Preset JSON")');
  await p.selectOption('#preset', 'Melt'); ok(await p.evaluate(() => hillview.getState().smooth.sigma) === 6, 'preset loads');
  await p.click('#vOrig'); const orig = await snap(); await p.click('#vFx'); const fx = await snap();
  ok(mad(orig, fx) > 1, 'Original / Processed toggle shows different pictures');
  await p.evaluate(() => { const t = document.querySelector('#presetJson'); t.value = '{"modulation":[{"source":{"type":"lfo"},"target":"nope"}]}'; }); await p.click('#applyJson');
  ok((await p.textContent('#presetErr')).includes('unknown modulation target'), 'bad preset JSON is reported, not applied');
  await p.selectOption('#preset', 'Glitch pop');
  await p.waitForTimeout(400);

  // ---- video file, music, recording, saving
  await p.setInputFiles('#fileIn', S + '/clip.webm');
  await p.waitForFunction(() => hillview.source().kind === 'video', null, { timeout: 20000 });
  ok(await p.evaluate(() => { const s = hillview.source(); return s.w === 640 && s.h === 360; }), 'video opens at its native size');
  await p.waitForTimeout(1500);
  const v1 = await snap(); await p.waitForTimeout(500); const v2 = await snap();
  ok(mad(v1, v2) > 0.3, 'video plays through the pipeline (picture changes)');
  const fpsEst = await p.evaluate(() => hillview.source().fps); ok(Math.abs(fpsEst - 30) < 0.1, `frame rate estimated from playback: ${fpsEst}`);
  await p.click('#play'); await p.evaluate(() => { const s = document.querySelector('#scrub'); s.value = 1.0; s.dispatchEvent(new Event('input')); }); await p.waitForTimeout(400);
  ok(await p.evaluate(() => Math.abs(document.querySelector('#vid').currentTime - 1.0) < 0.1), 'scrubber seeks the video (paused)'); await p.click('#play');
  await p.setInputFiles('#musicIn', S + '/music.wav'); await p.waitForTimeout(300);
  ok(await p.evaluate(() => !document.querySelector('#musicOff').disabled), 'music file attaches'); await p.click('#musicOff');
  ok(await p.evaluate(() => document.querySelector('#musicOff').disabled), 'music can be removed');
  await p.setInputFiles('#fileIn', S + '/clip.webm'); await p.waitForFunction(() => hillview.source().kind === 'video'); await p.waitForTimeout(500);
  await p.click('#rec'); await p.waitForSelector('#recresult:not([hidden]) video', { timeout: 30000 });
  const recInfo = await p.textContent('#recresult .note'); console.log('   ', recInfo);
  ok(/Recorded [\d.]+ MB \(WEBM\)/.test(recInfo), 'recording of the video finished by itself at the end of the clip');
  await p.click('#recresult button.primary');
  await p.waitForFunction(() => window.__saved.some(s => s.filename.endsWith('.webm')), null, { timeout: 5000 });
  const sv = await p.evaluate(() => window.__saved.find(s => s.filename.endsWith('.webm')));
  ok(sv.size > 20000 && sv.head.startsWith('26,69,223,163'), `saved a real WebM via the downloads capability (${sv.filename}, ${sv.size} bytes, EBML header ok)`);
  const dims = await p.evaluate(() => new Promise(r => { const v = document.querySelector('#recresult video'); const done = () => r([v.videoWidth, v.videoHeight, v.error && v.error.code]); v.readyState >= 1 ? done() : v.addEventListener('loadedmetadata', done, { once: true }); setTimeout(done, 5000); }));
  ok(dims[0] === 960 && dims[1] === 540, `recorded file decodes at the working size ${dims[0]}x${dims[1]}`);
  await p.click('#still'); await p.waitForFunction(() => window.__saved.some(s => s.filename.endsWith('.png')), null, { timeout: 5000 });
  ok((await p.evaluate(() => window.__saved.find(s => s.filename.endsWith('.png')))).head === '137,80,78,71,13,10,26,10', 'saved frame is a PNG');
  await p.click('#saveJson'); await p.waitForFunction(() => window.__saved.some(s => s.filename.endsWith('.json')));
  ok(await p.evaluate(() => { const s = window.__saved.find(x => x.filename.endsWith('.json')); const j = JSON.parse(s.text); return j.sampling && Array.isArray(j.modulation) && j.remap.rules.length >= 1; }), 'preset JSON saves in the desktop-app format');

  // ---- demo recording stopped by hand
  await p.click('#useDemo'); await p.waitForTimeout(500); await p.click('#rec'); await p.waitForTimeout(2200); await p.click('#rec');
  await p.waitForFunction(() => /Recorded/.test(document.querySelector('#recresult .note').textContent) && !!document.querySelector('#recresult button.primary'), null, { timeout: 15000 });
  ok(true, 'demo recording can be stopped by hand and offers a save');

  // ---- sound toggle builds the audio graph without errors; phone layout has no sideways scroll
  await p.check('#sound'); await p.waitForTimeout(400);
  await p.setViewportSize({ width: 390, height: 844 }); await p.waitForTimeout(400);
  const sw = await p.evaluate(() => [document.documentElement.scrollWidth, innerWidth]); ok(sw[0] <= sw[1], `no horizontal scroll at phone width (${sw[0]} <= ${sw[1]})`);
  await p.screenshot({ path: S + '/shot_phone.png', fullPage: false });
  ok(errs.length === 0, 'no console/page errors' + (errs.length ? ': ' + errs.slice(0, 4).join(' | ') : ''));
  await b.close(); process.exit(fails ? 1 : 0);
})().catch(e => { console.error('CRASH', e); process.exit(2); });
