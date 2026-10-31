// Builds public/cmudict.txt (first pronunciation per word, lowercase) from the
// upstream cmudict.dict. Usage: node scripts/build-dict.mjs <path-to-cmudict.dict>
import { readFileSync, writeFileSync } from 'node:fs';

const src = process.argv[2];
if (!src) {
  console.error('usage: node scripts/build-dict.mjs <cmudict.dict>');
  process.exit(1);
}
const out = [];
for (const raw of readFileSync(src, 'utf8').split('\n')) {
  const line = raw.split('#')[0].trim();
  if (!line) continue;
  const sp = line.indexOf(' ');
  const word = line.slice(0, sp);
  if (/\(\d+\)$/.test(word)) continue; // keep primary pronunciation only
  out.push(`${word} ${line.slice(sp + 1)}`);
}
writeFileSync(new URL('../public/cmudict.txt', import.meta.url), out.join('\n') + '\n');
console.log(`wrote ${out.length} entries`);
