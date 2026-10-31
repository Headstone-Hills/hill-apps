// Simple controls: stage switches, effect amounts, movement macros, two-way sync with Advanced, export fidelity, Surprise me.
const { chromium } = require('playwright');
const S = process.argv[2]; let fails = 0; const ok = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) fails++; };
(async () => {
  const b = await chromium.launch({ args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'] });
  const ctx = await b.newContext({ viewport: { width: 1400, height: 1000 } });
  await ctx.addInitScript(() => { window.__saved = []; window.claude = { use: async n => n === 'downloads' ? { save: async r => { window.__saved.push({ filename: r.filename, text: typeof r.data === 'string' ? r.data : null }); return { status: 'saved' }; } } : null }; });
  const p = await ctx.newPage(); const errs = [];
  p.on('pageerror', e => errs.push('pageerror: ' + e.message)); p.on('console', m => { if (m.type() === 'error') errs.push('console: ' + m.text()); });
  await p.goto('file://' + S + '/index.html');
  await p.waitForFunction(() => window.hillview && hillview.source() && hillview.format, null, { timeout: 20000 });
  const st = () => p.evaluate(() => JSON.parse(JSON.stringify(hillview.getState())));
  const ui = () => p.evaluate(() => hillview.getUI());
  const setRange = (sel, v) => p.evaluate(([s, v]) => { const e = document.querySelector(s); e.value = v; e.dispatchEvent(new Event('input', { bubbles: true })); }, [sel, v]);
  const setColor = (sel, v) => p.evaluate(([s, v]) => { const e = document.querySelector(s); e.value = v; e.dispatchEvent(new Event('input', { bubbles: true })); }, [sel, v]);
  const sw = id => p.evaluate(id => document.querySelector('#t_' + id).checked, id);
  const rowOff = id => p.evaluate(id => document.querySelector('.fx[data-id=' + id + ']').classList.contains('off'), id);
  const outText = id => p.evaluate(id => document.querySelector('.fx[data-id=' + id + '] output').textContent, id);

  // ---- the opening look is reflected honestly by the switches
  ok(await sw('stutter') && await sw('crush') && await sw('melt') && await sw('recolour') && !(await sw('soften')), 'Glitch pop: Stutter, Crush, Melt, Recolour on; Soften off');
  ok((await outText('stutter')) === 'every 4 frames' && (await outText('crush')) === '5 colours' && (await outText('melt')) === '2 px', 'amount readouts say what they do (every 4 frames / 5 colours / 2 px)');
  ok((await outText('react')).endsWith('2 routes') && (await outText('drift')).endsWith('0 routes') && (await outText('chaos')).endsWith('2 routes'), 'movement rows count their routes: audio 2, drift 0, chaos 2');
  ok(await p.evaluate(() => document.querySelector('#t_drift').disabled), 'a macro with no routes is disabled, with the reason in its hint: ' + JSON.stringify(await p.evaluate(() => document.querySelector('.fx[data-id=drift] .fxhint').textContent)));

  // ---- switches are non-destructive
  await p.click('#t_melt'); let s = await st();
  ok(s.smooth.sigma === 0 && !(await sw('melt')) && await rowOff('melt') && await p.evaluate(() => document.querySelector('#m_melt').disabled), 'Melt off: smoothing is 0, row dims, slider disabled');
  await p.click('#t_melt'); s = await st(); ok(s.smooth.sigma === 2 && await sw('melt'), 'Melt back on restores the previous 2 px');
  await setRange('#m_melt', 9); await p.click('#t_melt'); await p.click('#t_melt'); s = await st(); ok(s.smooth.sigma === 9, 'a changed amount is remembered across off/on (9 px)');
  await p.click('#t_soften'); s = await st(); ok(s.blur.sigma === 4 && await sw('soften'), 'Soften on uses a sensible default (4 px)');
  await p.click('#t_stutter'); s = await st(); ok(s.sampling.step === 1, 'Stutter off = every frame'); await p.click('#t_stutter'); s = await st(); ok(s.sampling.step === 4, 'Stutter on restores every 4th frame');
  await p.click('#t_crush'); ok(!(await st()).quantize.enabled && await rowOff('crush'), 'Crush off'); await p.click('#t_crush');
  await p.click('#t_recolour'); s = await st(); ok(s.remap.rules.length === 0 && await p.evaluate(() => !document.querySelector('#swatches').children.length) && await rowOff('recolour'), 'Recolour off clears the rules and the swatches');
  await p.click('#t_recolour'); s = await st(); ok(s.remap.rules.length === 2 && s.remap.rules[0].to === '#ff00aa' && await p.evaluate(() => document.querySelectorAll('#swatches .swrow').length === 2), 'Recolour on brings the same two colour pairs back');

  // ---- amounts write the real settings
  await setRange('#m_stutter', 6); s = await st(); ok(s.sampling.step === 6 && (await outText('stutter')) === 'every 6 frames', 'Stutter amount sets the frame step');
  await p.uncheck('#m_fade'); ok((await st()).sampling.interp === 'hold', 'cross-fade checkbox sets hold'); await p.check('#m_fade'); ok((await st()).sampling.interp === 'blend', '…and blend');
  await p.click('.fx[data-id=crush] button[data-m=posterize]'); s = await st(); ok(s.quantize.mode === 'posterize' && (await outText('crush')).endsWith('levels'), 'Crush method switches and the readout changes to levels');
  await setRange('#m_crush', 3); ok((await st()).quantize.levels === 3 && (await outText('crush')) === '3 levels', 'Crush amount sets the levels'); await p.click('.fx[data-id=crush] button[data-m=kmeans]');
  await setRange('#m_recolour', 0.12); ok(Math.abs((await st()).remap.threshold - 0.12) < 1e-9, 'Recolour reach sets the threshold offset');
  await setRange('#m_hue', 90); ok(Math.abs((await st()).remap.hue_shift - Math.PI / 2) < 1e-9 && (await p.evaluate(() => document.querySelector('#m_hue').previousElementSibling.textContent)) === '90°', 'hue shift is set in degrees and stored in radians');
  await setRange('#m_amt', 0.5); ok((await st()).remap.amount === 0.5, 'Recolour strength sets the amount');

  // ---- colour pairs
  await p.click('#addPair'); ok((await st()).remap.rules.length === 3, 'add colour pair');
  await setColor('#swatches .swrow:last-child .f', '#123456'); await setColor('#swatches .swrow:last-child .t', '#abcdef'); s = await st();
  ok(s.remap.rules[2].from === '#123456' && s.remap.rules[2].to === '#abcdef', 'editing a swatch edits the rule');
  ok(await p.evaluate(() => { const el = document.querySelector('#swatches .swrow:last-child .f'); el.value = '#654321'; el.dispatchEvent(new Event('input', { bubbles: true })); return document.querySelector('#swatches .swrow:last-child .f') === el && el.isConnected; }), 'a swatch input is not rebuilt while you drag it (same DOM node after its own edit)');
  await p.click('#swatches .swrow:last-child .x'); ok((await st()).remap.rules.length === 2, 'remove colour pair');

  // ---- two-way sync with Advanced
  await p.click('summary:has-text("Advanced controls")');
  await setRange('#s_blur_sigma', 7); ok((await outText('soften')) === '7 px' && await p.evaluate(() => document.querySelector('#m_soften').value) === '7', 'Advanced slider moves the simple one');
  await setRange('#m_soften', 3); ok(await p.evaluate(() => document.querySelector('#s_blur_sigma').value) === '3', 'simple slider moves the Advanced one');
  await setRange('#s_blur_sigma', 0); ok(!(await sw('soften')), 'setting blur to 0 in Advanced turns the Soften switch off by itself');
  await setColor('#rules .card:first-child .f', '#fedcba'); ok(await p.evaluate(() => document.querySelector('#swatches .swrow .f').value) === '#fedcba', 'colour edited in Advanced shows in the simple swatches');
  await setColor('#swatches .swrow .t', '#00ff00'); ok(await p.evaluate(() => document.querySelector('#rules .card .t').value) === '#00ff00', 'colour edited in the swatches shows in Advanced');
  await p.click('#rules .card:first-child .x'); ok(await p.evaluate(() => document.querySelectorAll('#swatches .swrow').length) === 1, 'rule removed in Advanced disappears from the swatches');

  // ---- movement macros: exact arithmetic on the real parameter values
  await p.click('#chips .chip:has-text("Glitch pop")'); const AFa = { rms: 0, low: 1, mid: 0, high: 0, onset: 0 };
  const sig = (af = AFa, t = 0) => p.evaluate(([af, t]) => hillview.paramsAt(t, af)['smooth.sigma'], [af, t]);
  ok(Math.abs((await sig()) - 10) < 1e-9, 'Glitch pop: bass at full adds 8 to the smoothing: 2 + 8 = 10');
  await setRange('#m_react', 0.5); ok(Math.abs((await sig()) - 6) < 1e-9, 'Reactivity 50%: 2 + 4 = 6');
  await setRange('#m_react', 2); ok(Math.abs((await sig()) - 18) < 1e-9, 'Reactivity 200%: 2 + 16 = 18');
  await p.click('#t_react'); ok(Math.abs((await sig()) - 2) < 1e-9 && await rowOff('react'), 'Reactivity off: only the base 2');
  await p.click('#t_react');
  const hue = t => p.evaluate(t => hillview.paramsAt(t, { rms: 0, low: 0, mid: 0, high: 0, onset: 0 })['remap.hue_shift'], t);
  const h1 = await hue(1.37); ok(Math.abs(h1) > 0.05, 'noise route is moving the hue at t=1.37 (' + h1.toFixed(3) + ')');
  await p.click('#t_chaos'); ok(Math.abs(await hue(1.37)) < 1e-12, 'Chaos off removes the random routes'); await p.click('#t_chaos');
  await setRange('#m_chaos', 0.5); ok(Math.abs((await hue(1.37)) - h1 * 0.5) < 1e-9, 'Chaos 50% halves the random depth exactly');
  await setRange('#m_chaos', 1);
  await p.evaluate(() => hillview.loadConfig({ modulation: [{ source: { type: 'noise', hz: 1, seed: 3 }, target: 'blur.sigma', depth: 4, }] }));
  const bl = t => p.evaluate(t => hillview.paramsAt(t, {})['blur.sigma'], t); const base = await bl(0.74);
  await setRange('#m_speed', 1); ok(Math.abs((await bl(0.37)) - base) < 1e-12 && (await outText('speed')) === '×2.00', 'Speed ×2: time runs twice as fast through the random route (exact)');
  await setRange('#m_speed', -1); ok(Math.abs((await bl(1.48)) - base) < 1e-12, 'Speed ×0.5: half as fast (exact)');
  await setRange('#m_speed', 0);

  // ---- exported looks include the macros; the underlying settings stay editable
  await p.click('#chips .chip:has-text("Glitch pop")'); await setRange('#m_react', 0.5); await p.click('#t_chaos');
  await p.click('#saveJson'); await p.waitForFunction(() => window.__saved.some(x => x.filename.endsWith('.json')));
  const saved = JSON.parse(await p.evaluate(() => window.__saved.at(-1).text));
  const audio = saved.modulation.filter(m => m.source.type === 'audio');
  ok(audio.length === 2 && audio.find(m => m.target === 'smooth.sigma').depth === 4 && !saved.modulation.some(m => m.source.type === 'noise' || m.source.type === 'steps'), 'saved look: audio depth 8 → 4 at 50%, random routes gone with Chaos off');
  ok((await st()).modulation.length === 4 && (await st()).modulation.find(m => m.target === 'smooth.sigma').depth === 8, 'the underlying routes are untouched (still 4, depth 8)');
  const probe = () => p.evaluate(() => [0.3, 1.37, 5.2].map(t => hillview.paramsAt(t, { rms: 0.4, low: 0.7, mid: 0.2, high: 0.5, onset: 0.9 })));
  const before = await probe();
  await p.evaluate(j => hillview.loadConfig(j), saved); const after = await probe();
  const worst = Math.max(...before.flatMap((o, i) => Object.keys(o).map(k => Math.abs(o[k] - after[i][k]))));
  ok((await ui()).react === 1 && (await ui()).chaosOn && worst < 1e-12, `reloading a saved look gives macros back at 100% and the identical parameters (worst diff ${worst.toExponential(1)} over 3 times x 8 params)`);

  // ---- looks and Surprise me
  await setRange('#m_react', 0.25); await p.click('#chips .chip:has-text("Neon posterize")'); s = await st();
  ok(s.quantize.mode === 'posterize' && (await ui()).react === 1 && await sw('crush'), 'choosing a look resets the macros and shows the right switches');
  await p.click('#chips .chip:has-text("Clean")'); ok(!(await sw('stutter')) && !(await sw('crush')) && !(await sw('melt')) && !(await sw('soften')) && !(await sw('recolour')), 'Clean look: every switch off');
  let bad = 0, blank = 0, seen = new Set();
  for (let i = 0; i < 25; i++) {
    await p.click('#surprise'); const c = await st();
    const any = c.sampling.step > 1 || c.quantize.enabled || c.smooth.sigma > 0 || c.blur.sigma > 0 || c.remap.rules.length > 0;
    const sync = (await sw('stutter')) === c.sampling.step > 1 && (await sw('crush')) === c.quantize.enabled && (await sw('melt')) === c.smooth.sigma > 0 && (await sw('soften')) === c.blur.sigma > 0 && (await sw('recolour')) === c.remap.rules.length > 0;
    if (!any || !sync) bad++;
    seen.add(JSON.stringify([c.sampling.step, c.quantize.mode, c.remap.rules.length]));
    const px = await p.evaluate(() => hillview.readCanvas()); let nz = 0; for (let k = 0; k < px.length; k += 4 * 211) nz += px[k] + px[k + 1] + px[k + 2]; if (!nz) blank++;
  }
  ok(bad === 0, '25 × Surprise me: every result has something on and the switches match it');
  ok(blank === 0 && seen.size > 6, `…every result renders a picture, and they differ (${seen.size} distinct shapes)`);
  await p.screenshot({ path: S + '/shot_simple_desktop.png' });
  await p.setViewportSize({ width: 390, height: 844 }); await p.waitForTimeout(300);
  const sw2 = await p.evaluate(() => [document.documentElement.scrollWidth, innerWidth]); ok(sw2[0] <= sw2[1], `no horizontal scroll at phone width (${sw2[0]} <= ${sw2[1]})`);
  ok(errs.length === 0, 'no console/page errors' + (errs.length ? ': ' + errs.slice(0, 4).join(' | ') : ''));
  await b.close(); process.exit(fails ? 1 : 0);
})().catch(e => { console.error('CRASH', e); process.exit(2); });
