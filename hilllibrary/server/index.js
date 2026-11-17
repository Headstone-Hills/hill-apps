// Entry point. Configuration comes from environment variables (see .env.example).

import { createServer as createHttp } from 'node:http';
import { createServer as createHttps } from 'node:https';
import { readFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import { LibraryDb } from './db.js';
import { createDiscLookup, createTmdbClient } from './discs.js';
import { createJellyfin } from './jellyfin.js';
import { createVisionReader, createTesseractReader, createClaudeReader } from './cover.js';
import { createLookup, createSearch } from './metadata.js';
import { createApp } from './app.js';

const here = dirname(fileURLToPath(import.meta.url));
const env = process.env;

const password = env.LIBRARY_PASSWORD ?? '';
const port = Number(env.PORT ?? 3000);
const loopback = (h) => ['127.0.0.1', '::1', 'localhost'].includes(h);
const host = env.HOST || (password ? '0.0.0.0' : '127.0.0.1');
const dataDir = resolve(env.DATA_DIR || join(here, '..', 'data'));

if (!password && !loopback(host)) {
  console.error(`Refusing to listen on ${host} without a password: anyone on the network could read and edit your library.\nSet LIBRARY_PASSWORD (see .env.example), or leave HOST unset to stay on localhost.`);
  process.exit(1);
}
if (password && password.length < 8) {
  console.error('LIBRARY_PASSWORD must be at least 8 characters.');
  process.exit(1);
}

mkdirSync(dataDir, { recursive: true });
const db = new LibraryDb(join(dataDir, 'library.db'));
const googleApiKey = env.GOOGLE_BOOKS_API_KEY ?? '';
const lookup = createLookup({ googleApiKey });
const search = createSearch({ googleApiKey, breaker: lookup.breaker }); // one breaker: a provider that throttles lookups is skipped by search too
const tmdbKey = (env.TMDB_API_KEY ?? '').trim();
const lookupDisc = createDiscLookup({ tmdbKey });
const tmdb = createTmdbClient({ key: tmdbKey });
const jellyfinKey = (env.JELLYFIN_API_KEY ?? '').trim();
const jellyfin = createJellyfin({
  url: jellyfinKey ? (env.JELLYFIN_URL || 'http://127.0.0.1:8096') : '', apiKey: jellyfinKey, publicUrl: env.JELLYFIN_PUBLIC_URL ?? '', library: env.JELLYFIN_MOVIE_LIBRARY ?? '',
});
// Cover readers, best first: Apple Vision (macOS, local), then Claude (only if you set a key: the photo is then
// sent to Anthropic, but only when the local reader found nothing), then Tesseract if installed.
const anthropicKey = (env.ANTHROPIC_API_KEY ?? '').trim();
const coverReaders = env.COVER_SCAN === 'off' ? [] : [
  createVisionReader({ binDir: join(dataDir, 'bin') }),
  ...(anthropicKey ? [createClaudeReader({ apiKey: anthropicKey, model: env.COVER_VISION_MODEL || undefined })] : []),
  createTesseractReader({ bin: env.TESSERACT_PATH || 'tesseract' }),
];
coverReaders[0]?.warm?.();
const app = createApp({
  db, lookup, search, lookupDisc, tmdb, jellyfin, coverReaders, googleApiKey: env.GOOGLE_BOOKS_API_KEY ?? '', dataDir, publicDir: join(here, '..', 'public'), password,
  trustProxy: /^(1|true|yes)$/i.test(env.TRUST_PROXY ?? ''),
  allowedOrigins: (env.ALLOWED_ORIGINS ?? '').split(',').map((s) => s.trim()).filter(Boolean),
});

const tls = env.TLS_CERT && env.TLS_KEY ? { cert: readFileSync(env.TLS_CERT), key: readFileSync(env.TLS_KEY) } : null;
const server = tls ? createHttps(tls, app) : createHttp(app);

server.listen(port, host, () => {
  const scheme = tls ? 'https' : 'http';
  console.log(`Hill Library running at ${scheme}://${host === '0.0.0.0' ? 'localhost' : host}:${port}`);
  console.log(`  data: ${dataDir}  |  books: ${db.count()}  |  auth: ${password ? 'password' : 'none (localhost only)'}  |  movie genres: ${env.TMDB_API_KEY ? 'TMDB' : 'keyword guess (set TMDB_API_KEY for accurate genres)'}  |  jellyfin: ${jellyfin.enabled ? 'connected' : 'off (set JELLYFIN_API_KEY)'}  |  cover scan: ${coverReaders.map((r) => r.name).join(' > ') || 'off'}`);
  if (!tls) console.log('  note: phone/tablet cameras only work over HTTPS (or localhost). See README "Using it from your phone".');
});

// On shutdown, stop taking requests but give in-flight cover downloads a few seconds to finish first,
// instead of closing the database underneath them.
let stopping = false;
for (const sig of ['SIGINT', 'SIGTERM']) {
  process.on(sig, async () => {
    if (stopping) return;
    stopping = true;
    server.close();
    await Promise.race([app.idle(), new Promise((r) => setTimeout(r, 5000))]);
    db.close();
    process.exit(0);
  });
}
