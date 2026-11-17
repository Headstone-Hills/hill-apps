# Hill Library

A personal home-library web app: scan a book's barcode, get its metadata automatically,
have it shelved by genre with auto-generated tags, and find anything with one search box.
It runs on a machine at home (a Raspberry Pi, an old laptop, a NAS) and you use it from
any phone, tablet or computer with a browser. Personal use, one library, no accounts.

- **Zero npm dependencies.** Node 22.13+ only. Storage is the SQLite built into Node
  (`node:sqlite`) with FTS5 full-text search.
- **Installable** to a phone home screen (PWA).
- **Private by default:** listens on localhost only unless you set a password.

## Quick start

```bash
cd hilllibrary
cp .env.example .env        # set LIBRARY_PASSWORD (8+ characters)
npm start                   # http://localhost:3000
npm test                    # 78 unit/API tests
```

Your data lives in `./data/` (`library.db` plus `covers/`). **Back that folder up.**
The *More* tab also exports CSV/JSON of the catalogue.

## Using it from your phone (read this: cameras need HTTPS)

Browsers only allow camera access on **secure contexts**: HTTPS, or `localhost`. Opening
`http://192.168.1.20:3000` on a phone will load the app but the camera is blocked
(the app tells you so, and manual ISBN entry still works). Options, easiest first:

1. **Tailscale** (private, free for personal use). Install it on the server and your
   devices, then publish the app over HTTPS with `tailscale serve` (it provisions a real
   certificate for your `*.ts.net` name). Only your own devices can reach it.
   Docs: <https://tailscale.com/kb/1312/serve> and <https://tailscale.com/kb/1153/enabling-https>
2. **A reverse proxy with HTTPS** (Caddy, nginx) in front of the app. The app honours
   `X-Forwarded-Proto` for secure cookies. If requests are refused with
   "Cross-origin request blocked", add your public URL to `ALLOWED_ORIGINS`.
3. **Direct HTTPS**: set `TLS_CERT` and `TLS_KEY` in `.env`.

> ⚠️ I wrote the Tailscale pointers from memory and could not open those pages while
> building this (the sandbox blocks outside sites). Check the linked docs for current steps.

Keep `LIBRARY_PASSWORD` set whenever the server is reachable beyond your own machine.
If you expose it to the public internet, put it behind the proxy/VPN first; the built-in
login is a single shared password with rate limiting, not a hardened public-facing system.

## How it works

### Scanning
`public/scanner.js` uses the browser's native **`BarcodeDetector`** when it supports
EAN-13 (Chrome/Edge on Android). Otherwise it falls back to a vendored copy of
**ZXing** ([`@zxing/library`](https://github.com/zxing-js/library) 0.23.0, Apache-2.0,
`public/vendor/`) which decodes video frames on a canvas (Safari, Firefox, desktop).
Book barcodes are EAN-13 encodings of the ISBN-13; only codes with a valid ISBN-13
checksum and a 978/979 prefix are accepted, so price add-ons and other products are ignored.

**Scan from photo** is an alternative that takes one photo with your phone's own camera app and reads the barcode
from it. It never opens a live video stream (no `getUserMedia`), so it also works over plain HTTP, where the live
camera is blocked. It tries the photo upright, sideways and slightly tilted. Tested on synthetic 12-megapixel
photos (tilt up to 15°, blur, noise, dimming, a small barcode, sideways); not yet on real phone photos.

Scan tips: good light (there is a torch button where the device allows it), hold steady
about 15–25 cm away, keep the whole barcode inside the box. Tick **Add automatically** and
set a **location** to zip through a whole shelf; each add has an Undo.

### When a barcode has no match (cover photo)
If the databases can't find a title, you aren't stuck:

- Whatever they *did* know (author, publisher, date, page count, subjects, Dewey number) is kept and
  pre-fills the add form, and the subjects still feed genre/tag detection.
- **Take cover photo & add** opens your phone's camera app (a normal full-resolution photo, not a video
  frame). The photo is shrunk to a ~1400 px JPEG, shown in the form, and saved as the book's cover once you
  type the title and tap *Add book*. **Add without photo** skips it.
- Any book's details also have **Take cover photo / Choose image** to add or replace a cover later (e.g.
  when a provider's cover is missing or wrong). Photos are stored in `data/covers/` like any other cover.

The title still has to be typed: the app does not read text from the photo.

### Metadata
Given an ISBN the server asks two free sources in parallel and merges them field by field:

| Source | Used for | Docs |
|---|---|---|
| Open Library | title, authors, publisher, subjects, Dewey number, cover | <https://openlibrary.org/dev/docs/api/books> |
| Google Books | description, category paths, fallback for the rest | <https://developers.google.com/books/docs/v1/using> |

If one is down or doesn't know the book, the other is used. "Not found" (offer manual
entry) is distinguished from "couldn't reach the internet" (retry). Covers are downloaded
once to `data/covers/`, from an allow-list of hosts only. Google Books works without a key
but may rate-limit; a free key in `GOOGLE_BOOKS_API_KEY` avoids that.

### Genres and tags (`server/genres.js`)
Library catalogue data is messy: Open Library gives free-form subject headings
("Fantasy fiction", "Description and travel"); Google gives paths like
"Juvenile Fiction / Fantasy & Magic". Neither maps cleanly onto a shelf, so the classifier
**scores every genre from every signal** instead of trusting one field:

1. Each subject, category, the title and the description is a *signal*, weighted by
   how much it's trusted (category 1.2, subject 1.0, title 0.6, description 0.15).
2. About 35 genres, each with weighted patterns and optional subgenres
   (Science Fiction › Cyberpunk, History › Military & War, …). Per signal, a genre
   takes its strongest matching rule; scores sum across signals.
3. A **fiction / non-fiction check** damps the wrong side, so "science fiction" never
   lands in Science and "History -- Fiction" goes to Historical Fiction.
4. **Dewey Decimal** numbers add a coarse non-fiction signal. (A fiction number like 813
   only flags "this is fiction"; it deliberately doesn't vote for a genre. An earlier
   version let it, and every genre novel with a Dewey number got shelved as Literary Fiction.)
5. The winner is the **genre**; the best-scoring rule inside it is the **subgenre**; other
   strong genres become **secondary tags** (e.g. a children's fantasy gets "Fantasy" as its
   shelf and "children's books" as a tag).
6. A **confidence** level is recorded. Low confidence or no match marks the book
   *needs review*; Browse shows how many, one tap away.

**You always have the last word.** Choosing a genre, or editing tags, in a book's
details *locks* it so automatic detection never overwrites it; *Auto* hands control
back. **More → Re-run genre detection** re-classifies everything from the saved raw
metadata (offline) while skipping locked fields, so improving the rules later is safe.

The default library view is a **shelf**: Genre → Subgenre → Author surname → Title
(leading "The/A/An" ignored), with Uncategorized last. Other sorts: title, author,
recently added, publication date.

### Search
One box. Every word is a prefix match, all words must match (`frank her` finds Herbert),
across title, subtitle, author, tags, genre, description, notes and ISBN. Case and accents
are ignored (`miserables` finds *Les Misérables*). Title and author matches rank above
description matches. Input is turned into a quoted FTS5 query, so it can't inject syntax.
Reference: <https://www.sqlite.org/fts5.html>

## Configuration

See `.env.example`. `LIBRARY_PASSWORD`, `PORT`, `HOST`, `DATA_DIR`, `GOOGLE_BOOKS_API_KEY`,
`TLS_CERT`/`TLS_KEY`, `ALLOWED_ORIGINS`.

## Security notes

- Login is a shared password; session cookie is `HttpOnly`, `SameSite=Lax`, `Secure` over HTTPS,
  signed with a random secret stored in `data/.secret`; 10 failed logins per 15 min per IP are locked out.
- Changes must be JSON and same-origin (CSRF); strict Content-Security-Policy; no inline scripts.
- Book metadata comes from third parties and is only ever rendered as text, never as HTML.
- CSV export neutralises spreadsheet formula injection.
- Cover uploads are checked by their actual bytes (JPEG/PNG/WebP only; SVG and GIF are refused), limited to 5 MB,
  need a login, and are stored under a fresh filename each time so old covers never linger in caches.

## Verification status: what has and hasn't been tested

Tested (automated): ISBN math, classifier on 20 fixture books, database/search/sorting,
all API routes, auth, CSRF, path traversal, and a Chromium end-to-end run
(`node tests/e2e/ui.e2e.mjs [fake-camera.y4m] [screenshot-dir]`, needs Playwright)
that adds books, searches, edits, and **decodes an independently generated EAN-13
barcode from a fake camera feed**, including a rotated/noisy/dim variant, via the ZXing fallback.

**Not verified, because of the build sandbox. Please check on your own devices:**

1. **Live Open Library and Google Books responses.** Those hosts were unreachable
   (Open Library blocked; Google rate-limited), so the response parsing follows the
   documented format from memory and is tested against fixtures in that shape. It
   ignores fields that are missing or the wrong type, so a format difference should
   degrade to "less metadata", not a crash. Scan one real book first and compare.
2. **The native `BarcodeDetector` path.** The test browser doesn't have it, so only
   the ZXing path ran. (Native is plain feature-detection code, but it's untested.)
3. **Real-world scanning** (glare, curved covers, cheap cameras, iOS Safari). The synthetic
   test is encouraging, not proof.
4. **Genre accuracy on your real books.** The fixtures are books I modelled from memory
   of how they're catalogued. Expect some wrong shelves at first; fix them in the
   book's details and use *needs review*. Mismatches are cheap to correct and the
   rules in `genres.js` are plain data.
5. Installing to the home screen, and the HTTPS setups above.
7. **Live scanning interrupting other apps.** Reported: video/audio playing in the background stops while the
   live camera scanner runs. The app never requests the microphone, audio focus or a media session (the only media
   call is `getUserMedia({ audio: false, video })`), and a web search found no documented cause, so this looks like
   the phone/browser's handling of an active camera rather than something the page does. That is unconfirmed.
   **Scan from photo** avoids the live stream entirely, so it doubles as the test: if playback survives it, the live
   stream is the trigger; if playback still stops, it's the phone's camera itself and no web page can prevent it.
6. **The cover-photo capture on real phones.** The test drove the file picker with a generated JPEG; it did not
   use an actual phone camera. The `capture` hint that opens the camera app is standard but behaves a little
   differently across Android/iOS browsers (some offer a choice, some go straight to the camera).

## Layout

```
server/index.js      entry point + safe-by-default config
server/app.js        HTTP routes, auth, security headers
server/db.js         SQLite schema, FTS5 search, queries
server/metadata.js   Open Library + Google Books lookup, merge, covers
server/genres.js     genre/tag classifier (rules are data at the top)
server/isbn.js       ISBN-10/13 validation and conversion
public/              the app (vanilla JS, no build step) + service worker
tests/               node:test suites; tests/e2e is the browser run
```
