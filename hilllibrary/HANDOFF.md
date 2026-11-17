# Hill Library: handoff to a local session

Written at the end of a cloud session. **This is the redacted, committed copy**: the user's domain, registrar
and email details were removed on purpose. The user holds the full version (`HANDOFF.private.md`, git-ignored).
Read this first, then `hilllibrary/README.md`.

## Where things stand

- **Repo / branch:** `Headstone-Hills/hill-apps`, branch `claude/blissful-knuth-ie3n69`
  (pushed, clean, HEAD `6230380`). No PR was opened (the user hasn't asked for one).
- **What was built:** `hilllibrary/`, a self-hosted home-library web app: barcode scanning,
  automatic metadata (Open Library + Google Books), a weighted genre/tag classifier,
  FTS5 search, installable PWA. Zero npm dependencies, Node >= 22.13 (`node:sqlite`).
  It is a new sibling of the repo's unrelated Python music apps (it only adds a mention to the root README).
- **Tests:** `cd hilllibrary && npm test` gives 88 passing. A Chromium end-to-end run exists at
  `tests/e2e/ui.e2e.mjs` (needs Playwright; not part of `npm test`) and passed, including decoding an
  independently generated EAN-13 barcode from a fake camera feed.
- **Not deployed anywhere yet.** Nothing is running; the user has not tried it on a phone.

## Added after the first handoff (2026-11-17)

- **No-match flow with cover photo.** Lookup now keeps *partial* provider data (no title but publisher/subjects/etc.):
  `/api/lookup/:isbn` returns `{found:false, partial:{...}}`; `mergeProviders` sets `incomplete:true`; `POST /api/books`
  still requires a user-supplied title for such records but uses the partial data + subjects for classification.
  New `PUT /api/books/:id/cover` takes raw image bytes (sniffed, JPEG/PNG/WebP, <= 5 MB), writes
  `data/covers/<id>-<stamp>.<ext>` (unique name defeats the service-worker cache-first covers) and deletes the old file.
  Client: `pickPhoto()` / `shrinkImage()` / `uploadCover()` in `public/app.js`; the book dialog (add and edit) has
  Take cover photo / Choose image; the scan no-match card offers "Take cover photo & add".
- **Bug fixed:** `Element.append(null)` inserted the literal text "null" on the Scan page (and More without a password).
  Pages are now assembled with a null-safe `put()`; the e2e fails if "null"/"undefined"/"NaN" text shows on any main screen.
- Tests: 78 unit/API passing; e2e extended (partial data + photo, cover replacement, stray-text guard).
- **Still unverified:** capture on real phones (the test uses a generated JPEG via Playwright's file chooser).

### Scanner changes (2026-11-17, second pass)

- **Scan from photo** (`decodeIsbnFromImage` in `public/scanner.js`, button on the Scan page): one still from the
  phone's camera app, decoded with the shared detector, retried upright/90/270/±6/±12 degrees at a few sizes. Added
  because live scanning was reported to stop background video/audio; it avoids `getUserMedia` entirely. e2e asserts it
  never calls `getUserMedia`. **The background-playback problem is NOT confirmed fixed**: the cause is unknown and
  nothing in the page's code touches audio. Ask the user what stops (other app vs. other tab), which phone/browser,
  and whether it still stops with Scan from photo. That one answer says whether it's the live stream or the OS camera.
- **Scanner bug fixed:** `MultiFormatReader.decode()` silently resets hints, so the "EAN-13 only / try harder" hints were
  ignored and every frame also ran the QR/Aztec/DataMatrix/PDF417 readers. Now uses `Z.EAN13Reader` directly
  (also avoids a per-frame `console.warn` the library emits on every "no barcode" frame). Accuracy equal or better,
  hard cases faster. e2e fails on any console warning/error, which is how this was found.
- Synthetic photo fixtures (not committed; need PIL + bwip-js): 12 MP, tilt, blur, noise, dim, sideways. Single-factor
  sweep all decoded. Never tested: real phone photos.

### Scan-speed work (2026-11-17, third pass)

Reported: lookups slow down after several scans in a row. **Measured first; the app's own code was not the cause.** Server
(240 consecutive scans, also with 3,000 books) and browser (14 distinct barcodes from a fake camera feed) were flat: lookup
~ provider latency + 2 ms, add ~ cover time + 7 ms, no growth, no long tasks. What was left is time spent *waiting on
external services in series*, and a throttling provider makes that grow. Reproduced by simulation (providers degrade after
6 requests): scans 1-6 ~230 ms, scan 7 onward ~4.6 s each. **The real Open Library / Google Books could not be reached from
the sandbox, so real-world throttling is a likely explanation, not a confirmed one.**
Fixes (`server/metadata.js`, `server/app.js`): answer on the first complete provider (0.8 s grace; late data upgrades the cache);
per-provider circuit breaker (429 pauses at once, honours Retry-After; 2 consecutive failures pause with backoff; one probe after);
in-flight de-duplication; cache capped at 500 entries; **cover download moved off the add** into a 2-at-a-time background queue
that re-checks the book is still the same one (SQLite reuses ids) and has no cover before saving; graceful shutdown waits
<= 5 s for pending covers. Simulated result: steady ~127 ms/scan (4-scan transient ~0.93 s), add 107 -> ~4 ms. 88 tests.
Behaviour change: `POST /api/books` returns `has_cover:false`; the cover attaches moments later (tests use `handler.idle()`).
Benchmarks lived in the session scratchpad (not committed); they need only Node (+ Playwright and ffmpeg for the browser one).

## What the user wants

Personal-use home library app, reachable from all their devices as a web app: barcode scanning, automatic
metadata, a robust automatic tag/sort system **primarily by genre**, and simple search.

**Preferences to honour:** they like cited sources and detailed explanations, and they do *not* like offers
to write their work for them (do the task asked; don't end with "want me to write X?"). Use they/them if
you need a pronoun. Don't send their personal details to outside services.

## Current goal: host it on their own domain

- The user owns a domain (name withheld here; use `<their-domain>`) and already runs Jellyfin on a subdomain of it,
  on **a computer at home** (the user confirmed). This app would go at its own subdomain, e.g.
  `library.<their-domain>`. It must be a subdomain, not a subfolder, because the app uses root-absolute paths.
- **Known from the user's email in the cloud session:** the domain's **DNS is on Cloudflare**, so records are edited
  in the Cloudflare dashboard. Nothing there says how Jellyfin is exposed (tunnel vs. port-forward/proxy); that lives
  on the home computer. Registrar and account details are in the user's own copy, not this file.

### The one unanswered question (do this first, on the local machine)
Find out how Jellyfin is exposed today. It decides the whole setup:

1. Cloudflare dashboard -> `<their-domain>` -> DNS, look at the `jelly` record:
   `*.cfargotunnel.com` CNAME = **Cloudflare Tunnel**; `A` record to a home IP = **port forward + reverse proxy**.
2. On the machine: Linux `sudo ss -ltnp | grep -E ':(80|443|8096)'` and `systemctl status cloudflared`;
   Windows `netstat -ano | findstr ":443"` and check Services for `cloudflared`.

### Then (per case)
- **Tunnel:** add public hostname `library.<their-domain>` -> `http://localhost:3000` in Cloudflare Zero Trust.
- **Port forward + proxy (Caddy/nginx/NPM/Traefik):** add a `library` DNS record matching `jelly`, add a
  second site proxying to `127.0.0.1:3000`; the proxy must pass `Host` and `X-Forwarded-Proto: https`.
  Direct port-forward with no proxy means adding one (two apps can't share 80/443).
- **App side:** `cp .env.example .env`; set a long unique `LIBRARY_PASSWORD`, `HOST=127.0.0.1`
  (only reachable via the proxy/tunnel), `PORT=3000`. Run it as a service that starts on boot
  (systemd, Windows Task Scheduler, or pm2). If saves fail with "Cross-origin request blocked",
  add the full public URL to `ALLOWED_ORIGINS`.
- **Why a real domain matters:** phone cameras only work on HTTPS/localhost; the domain's certificate fixes that.
  (Tailscale `serve` was the alternative suggested; the user preferred their own domain.)

## Known limitations to fix or decide

1. **Login lockout is shared behind a proxy/tunnel.** `server/app.js` rate-limits by
   `req.socket.remoteAddress`, which will be the proxy's address for everyone. 10 failed logins from
   anyone lock out everyone (including the owner) for 15 minutes. Fix: trust a proxy-supplied client IP
   (e.g. `X-Forwarded-For` / `CF-Connecting-IP`), only when an env flag like `TRUST_PROXY` is set. **Not done.**
2. **Public-internet exposure.** Auth is one shared password. Recommend a second gate, such as Cloudflare
   Access (Zero Trust) in front of the hostname, or keep it reachable only over a VPN. Plan limits for
   Cloudflare Access were not checked (the cloud session couldn't open vendor docs).
3. **Backups:** `data/` (`library.db`, `covers/`, `.secret`) is the whole library. Nothing automates a backup.
4. Browser-side caches the app shell via a service worker (`public/sw.js`, cache name `hl-shell-v1`);
   bump the name if you ship breaking front-end changes and users see stale files.

## What was verified and what was NOT (be honest with the user about this)

Verified by automated tests: ISBN math, classifier on 20 fixture books, DB/FTS search/sorting, every API
route, auth/CSRF/path traversal, and a Chromium e2e run.

**Never verified (the cloud sandbox blocked or limited these), so the local session is the first place to try:**

1. **Live Open Library and Google Books responses.** Open Library was blocked (403) and Google returned 429, so
   the parsers in `server/metadata.js` were written from memory of the documented formats and tested only against
   fixtures in that shape. They ignore missing/wrong-typed fields, so a mismatch should mean *less metadata*,
   not a crash. **First real test: scan or look up one real ISBN and compare.** (Docs: openlibrary.org/dev/docs/api/books,
   developers.google.com/books/docs/v1/using. These could not be fetched, so no claim about them is source-checked.)
2. **The browser's native `BarcodeDetector` path.** The test Chromium lacks it, so only the bundled ZXing fallback ran
   (it decoded clean and rotated/noisy/dim synthetic barcodes). Native path = Chrome/Android; untested.
3. **Real scanning on real phones** (glare, curved covers, iOS Safari).
4. **Genre accuracy on the user's real books.** Fixtures were modelled from memory of how books are catalogued.
   Expect some wrong shelves; the UI has *needs review*, per-book genre/tag overrides (which lock them), and
   *More -> Re-run genre detection*. Rules are plain data at the top of `server/genres.js`.
5. Home-screen install, HTTPS via the domain, the proxy/tunnel config.
6. Tailscale doc links in `README.md` were written from memory and not opened.

## Orientation for the code

```
server/index.js      entry; refuses non-localhost listening without a password; password >= 8 chars
server/app.js        routes, auth (HMAC session cookie), CSRF/origin check, CSP, static files, cover saving
server/db.js         node:sqlite schema, FTS5 index (kept in sync by #reindex), list/facets queries
server/metadata.js   providers, merge, in-memory 10-min lookup cache, cover download (host allow-list)
server/genres.js     TAXONOMY (36 genres, rules are data), classify(), buildTags(), Dewey hints
public/              vanilla JS SPA (app.js), scanner.js, style.css, sw.js, vendor/zxing.min.js
tests/               node:test suites (*.test.js) and tests/e2e/ui.e2e.mjs
```

Behaviours worth knowing before you change anything:
- **Locks:** `genre_locked` / `tags_locked` mean the user set them; reclassify and refresh never overwrite them.
  PATCH `genre:"auto"` or `autoTags:true` hands control back. `meta_raw` stores the original subjects/categories/
  Dewey so reclassifying works offline.
- **Dewey fiction numbers (813/823…) only flag "fiction"; they deliberately don't vote for a genre.** An earlier
  version did, and every genre novel with a Dewey number became Literary Fiction (found by the e2e run; regression
  test exists).
- **`node:sqlite` cannot bind JS booleans**; `LibraryDb.update()` converts them to 0/1.
- **Strict CSP, no inline scripts/styles.** Dynamic styling uses CSSOM (`el.style.setProperty`). `img-src` is
  widened only for Open Library / archive.org / Google cover hosts (scan-preview images).
- **Never use `innerHTML` with metadata.** It's third-party content; the UI builds DOM with `h()` + text nodes.
- The Library view must not trust cached facets; `route()` refetches them on every navigation (a stale-count bug was fixed).

## Re-running the browser e2e (optional, needs Playwright + ffmpeg + a barcode)

`node --disable-warning=ExperimentalWarning tests/e2e/ui.e2e.mjs [fake-camera.y4m] [screenshot-dir]`
The y4m should show the EAN-13 for ISBN 9780547928227 (The Hobbit). It was generated with `bwip-js` (`ean13`) and
`ffmpeg` (barcode PNG overlaid on a 640x480 clip, `-pix_fmt yuv420p`) and run with Chromium flags
`--use-fake-device-for-media-stream --use-file-for-fake-video-capture=<file>`. Omit the video to skip the camera step.

## Suggested order of work

1. Pull the branch, `npm test`, run it locally (`npm start`), add one real book by ISBN. Confirm metadata, cover, genre.
2. Decide/fix the proxy-aware rate limit (limitation 1) *before* exposing it publicly.
3. Work out the tunnel/proxy question above, then deploy to `library.<their-domain>`.
4. Try scanning on the user's actual phone(s); report what the camera does on real covers.
5. Add a second access gate (Cloudflare Access) and a backup routine for `data/`.
6. Review wrong genres from the first batch of real books and tune `genres.js` rules (add fixtures to
   `tests/genres.test.js` for each correction).
