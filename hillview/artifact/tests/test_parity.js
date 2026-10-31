const { chromium } = require('playwright'); const fs = require('fs');
const S = process.argv[2]; const G = JSON.parse(fs.readFileSync(S + '/golden.json'));
let fails = 0; const ok = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) fails++; };
(async () => {
  const b = await chromium.launch({ args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist', '--autoplay-policy=no-user-gesture-required'] });
  const p = await b.newPage({ viewport: { width: 1400, height: 850 } });
  const errs = []; p.on('pageerror', e => errs.push('pageerror: ' + e.message)); p.on('console', m => { if (m.type() === 'error') errs.push('console: ' + m.text()); });
  await p.goto('file://' + S + '/index.html');
  await p.waitForFunction(() => window.hillview && window.hillview.source() && window.hillview.format, null, { timeout: 20000 });
  console.log('GL format:', await p.evaluate(() => hillview.format), '| source:', JSON.stringify(await p.evaluate(() => hillview.source())));
  // --- modulation parity (bit-exact hash, 1e-12 for sources)
  const hres = await p.evaluate(h => h.map(([n, s]) => hillview.hash01(n, s)), G.hash.map(x => [x[0], x[1]]));
  ok(G.hash.every((g, i) => hres[i] === g[2]), `hash01 identical to Python on ${G.hash.length} cases (incl. negative n, 2^40, big seeds)`);
  const mres = await p.evaluate(m => m.map(([src, t]) => hillview.sourceValue(src, t, {}, 0)), G.mod.map(x => [x[0], x[1]]));
  const worst = Math.max(...G.mod.map((g, i) => Math.abs(g[2] - mres[i])));
  ok(worst < 1e-12, `lfo/noise/steps sources match Python (worst diff ${worst.toExponential(1)} over ${G.mod.length} cases)`);
  const lres = await p.evaluate(o => o.map(([c]) => hillview.rgbToOklab(c)), G.oklab);
  const lw = Math.max(...G.oklab.flatMap((g, i) => g[1].map((v, j) => Math.abs(v - lres[i][j]))));
  ok(lw < 1e-5, `OKLab conversion matches Python (worst diff ${lw.toExponential(1)})`);
  const rt = await p.evaluate(() => { const c = [0.1, 0.7, 0.3]; return hillview.oklabToRgb(hillview.rgbToOklab(c)).map((v, i) => Math.abs(v - c[i])); });
  ok(Math.max(...rt) < 1e-6, 'OKLab round trip');

  // --- shader parity on a still image: posterize + 2 remap rules + hue shift (no blur)
  const I = G.img;
  await p.evaluate(async I => {
    const id = new ImageData(new Uint8ClampedArray(I.w * I.h * 4), I.w, I.h);
    for (let i = 0; i < I.w * I.h; i++) { id.data[i * 4] = I.rgb[i * 3]; id.data[i * 4 + 1] = I.rgb[i * 3 + 1]; id.data[i * 4 + 2] = I.rgb[i * 3 + 2]; id.data[i * 4 + 3] = 255; }
    hillview.setWorkingWidth(I.w); hillview.openBitmap(await createImageBitmap(id), 'test.png');
  }, I);
  const still = async cfg => { await p.evaluate(c => hillview.loadConfig(c), cfg); const r = await p.evaluate(() => hillview.stillRGBA()); const out = new Uint8Array(r.w * r.h * 3); for (let y = 0; y < r.h; y++) for (let x = 0; x < r.w; x++) for (let c = 0; c < 3; c++) out[(y * r.w + x) * 3 + c] = r.data[(((r.h - 1 - y) * r.w) + x) * 4 + c]; return { w: r.w, h: r.h, out }; };
  const cmp = (got, want, w, h, margin = 0) => { let mx = 0, sum = 0, n = 0; for (let y = margin; y < h - margin; y++) for (let x = margin; x < w - margin; x++) for (let c = 0; c < 3; c++) { const d = Math.abs(got[(y * w + x) * 3 + c] - want[(y * w + x) * 3 + c]); mx = Math.max(mx, d); sum += d; n++; } return { mx, mean: sum / n }; };
  let A = await still({ quantize: { enabled: true, mode: 'posterize', levels: 4 }, remap: { rules: I.rules, threshold: 0.05, amount: 0.9, hue_shift: 0.8 } });
  let c = cmp(A.out, I.A, A.w, A.h); ok(A.w === I.w && A.h === I.h, `still rendered at ${A.w}x${A.h}`);
  ok(c.mx <= 3, `posterize + remap (2 rules, hue shift, threshold/amount offsets) matches numpy: max ${c.mx} levels, mean ${c.mean.toFixed(3)}`);
  let B = await still({ quantize: { enabled: true, mode: 'posterize', levels: 4 }, smooth: { sigma: 2 } });
  c = cmp(B.out, I.B, B.w, B.h, 8); ok(c.mx <= 3, `posterize + 3-pass box blur (interior) matches numpy: max ${c.mx} levels, mean ${c.mean.toFixed(3)}`);
  let C = await still({ quantize: { enabled: true, mode: 'kmeans', colors: 5 }, seed: 3 });
  const uniq = new Set(); for (let i = 0; i < C.out.length; i += 3) uniq.add(C.out[i] + ',' + C.out[i + 1] + ',' + C.out[i + 2]);
  ok(uniq.size >= 2 && uniq.size <= 5, `k-means palette yields ${uniq.size} colours (asked for 5)`);
  let Z = await still({}); c = cmp(Z.out, I.rgb, Z.w, Z.h); ok(c.mx <= 1, `everything off is a pass-through (max ${c.mx} level)`);
  let big = await p.evaluate(() => { hillview.loadConfig({ smooth: { sigma: 40 }, blur: { sigma: 40 } }); const t = performance.now(); hillview.stillRGBA(); return performance.now() - t; });
  ok(big > 0, `max-radius blur renders without error (${big.toFixed(0)} ms on software GL)`);

  // --- eyedropper reads the ORIGINAL pixel
  await p.evaluate(() => hillview.loadConfig({ remap: { rules: [{ from: '#808080', to: '#ff0000', threshold: 0.2, softness: 0.1, amount: 1 }] }, quantize: { enabled: true, mode: 'posterize', levels: 2 } }));
  await p.click('#swatches .pk'); const box = await p.locator('#out').boundingBox();
  const ar = I.w / I.h; let w = box.width, h = box.height, ox = 0, oy = 0; if (w / h > ar) { const nw = h * ar; ox = (w - nw) / 2; w = nw; } else { const nh = w / ar; oy = (h - nh) / 2; h = nh; }
  await p.mouse.click(box.x + ox + w * 0.25, box.y + oy + h * 0.15);
  const picked = await p.evaluate(() => hillview.getState().remap.rules[0].from);
  ok(picked === '#c81e28', `eyedropper picked the unprocessed colour under the cursor: ${picked} (expected #c81e28)`);

  ok(errs.length === 0, 'no console/page errors' + (errs.length ? ': ' + errs.join(' | ') : ''));
  await b.close(); process.exit(fails ? 1 : 0);
})().catch(e => { console.error('CRASH', e); process.exit(2); });
