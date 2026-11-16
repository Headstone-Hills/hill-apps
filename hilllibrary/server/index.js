// Entry point. Configuration comes from environment variables (see .env.example).

import { createServer as createHttp } from 'node:http';
import { createServer as createHttps } from 'node:https';
import { readFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import { LibraryDb } from './db.js';
import { createLookup } from './metadata.js';
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
const lookup = createLookup({ googleApiKey: env.GOOGLE_BOOKS_API_KEY ?? '' });
const app = createApp({
  db, lookup, dataDir, publicDir: join(here, '..', 'public'), password,
  allowedOrigins: (env.ALLOWED_ORIGINS ?? '').split(',').map((s) => s.trim()).filter(Boolean),
});

const tls = env.TLS_CERT && env.TLS_KEY ? { cert: readFileSync(env.TLS_CERT), key: readFileSync(env.TLS_KEY) } : null;
const server = tls ? createHttps(tls, app) : createHttp(app);

server.listen(port, host, () => {
  const scheme = tls ? 'https' : 'http';
  console.log(`Hill Library running at ${scheme}://${host === '0.0.0.0' ? 'localhost' : host}:${port}`);
  console.log(`  data: ${dataDir}  |  books: ${db.count()}  |  auth: ${password ? 'password' : 'none (localhost only)'}`);
  if (!tls) console.log('  note: phone/tablet cameras only work over HTTPS (or localhost). See README "Using it from your phone".');
});

for (const sig of ['SIGINT', 'SIGTERM']) {
  process.on(sig, () => { server.close(); db.close(); process.exit(0); });
}
