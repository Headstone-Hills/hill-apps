# Hill Library

A personal home-library web app: scan a book's (or DVD's / Blu-ray's) barcode, get its metadata automatically,
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
npm test                    # 103 unit/API tests
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

### Finding a book by title or author
No barcode handy? On the **Scan** page, under the ISBN box, use **Or find it by title or author** (also reachable
from the **Find a book** button on the Library page, and from **Search online for “…”** when a search of your own library
finds nothing). Type words from the title and/or author (e.g. `dune herbert`) and press Search.

- The server asks Open Library and Google Books at once and merges the lists: duplicates (same ISBN, or same title and
  first author) become one row, gaps are filled from the other copy, and results are ranked by how well the title
  matches what you typed (a leading "The/A/An" is ignored, so `great gatsby` finds *The Great Gatsby* first).
- **Preview** (the result has an ISBN) runs the normal ISBN lookup, so you get full metadata, genre and tags, and
  the usual Add / Edit first. **Add details** (no ISBN) opens the add form pre-filled with what the search knew, including
  the cover. Books you already own are marked **In your library** and open instead of being added twice.
- Search shares the throttling protection described under Metadata, and answers within moments of the first provider.
  If only one provider answers you get a note that the list may be incomplete.
- Results from Open Library are works, not editions, so the ISBN offered is just one edition of several; if it is not
  the edition on your shelf, scan or type that edition's ISBN instead, or edit the details after adding.

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
Given an ISBN the server asks two free sources in parallel and merges them field by field. To keep rapid
scanning fast it **answers as soon as one source has a complete record**, waiting at most 0.8 s for the other to
add detail (a source that arrives late upgrades the cached record, so the add that follows still gets it). A source
that is **throttling you (HTTP 429, honouring `Retry-After`) or failing twice in a row is paused** for a while, instead
of every scan waiting on it again. A pause only ever skips a provider *while another one can answer*: if nothing usable came
back, paused providers are asked anyway, so a pause can never black out lookups. When a lookup fails, the screen and the
server terminal say which provider failed and why (e.g. `Open Library: HTTP 429`). Covers download **in the background** after the book is saved (two at a time), so an add
never waits on an image host; a cover can appear a second or two after the book does.

| Source | Used for | Docs |
|---|---|---|
| Open Library | title, authors, publisher, subjects, Dewey number, cover | <https://openlibrary.org/dev/docs/api/books> |
| Google Books | description, category paths, fallback for the rest | <https://developers.google.com/books/docs/v1/using> |

If one is down or doesn't know the book, the other is used. "Not found" (offer manual
entry) is distinguished from "couldn't reach the internet" (retry). Covers are downloaded
once to `data/covers/`, from an allow-list of hosts only. Google Books works without a key
but may rate-limit; a free key in `GOOGLE_BOOKS_API_KEY` avoids that.

### Covers without a barcode (`server/cover.js`)
On the Scan page, **No barcode? Photograph the cover** opens the phone camera (or photo picker). The
photo is shrunk in the browser to 1800 px, sent to the server, and turned into a short list of likely books:

1. **Read** the text on the cover with the best reader available, tried in order until one finds a good match:
   - **Apple Vision** (macOS, on-device, free). Far better than classic OCR on stylised covers. A small Swift
     helper (`server/native/cover-ocr.swift`) is compiled once into `data/bin/` at first start; needs the Xcode
     command line tools (`xcode-select --install`).
   - **Claude** (optional, `ANTHROPIC_API_KEY`): used only when the local reader finds no decent match. The
     photo is then sent to Anthropic. Off unless you set a key.
   - **Tesseract** (`brew install tesseract`), the fallback on non-Mac servers. Noticeably weaker.
2. **Search** Open Library and Google Books with the biggest words on the cover (single words and pairs, so one
   misread word can't sink the search).
3. **Rank** each hit by how much of its title and author surname appears in the cover text (tolerating one OCR slip),
   weighting big type over small blurbs, demoting books titled with just the author's name, and breaking ties by
   how widely published the work is. The ISBN comes from the edition whose cover Open Library shows.

You always pick from the list; nothing is added until you tap a book and confirm the normal preview. A printed
ISBN on the cover (checksum-validated) is offered directly. Set `COVER_SCAN=off` to disable the feature.

**Measured** on 18 well-known covers (catalogue images from Open Library, not phone photos): Apple Vision put the
right book first for 16 and in the top three for 17; Tesseract alone managed 9 and 12. The misses were a cover
with no readable text and one where only the author was legible. Expect worse on glare, curved spines, busy
photos and non-English covers (the readers are set to English).

### DVDs and Blu-rays (`server/discs.js`, `server/moviegenres.js`)
Scan a disc's barcode (UPC-A / EAN-13) the same way. The server tells a book ISBN from a
disc barcode by checksum and prefix, then:

| Step | Source | Gives | Docs |
|---|---|---|---|
| 1 | UPCitemdb free "trial" API, no key | retail title, studio/brand, description | <https://www.upcitemdb.com/wp/docs/main/development/getting-started/> |
| 2 | TMDB, **optional** (`TMDB_API_KEY`, free) | genres, director, runtime, year, overview, poster | <https://developer.themoviedb.org/docs/getting-started> |

- The free UPCitemdb tier allows roughly **100 lookups a day** per IP, and its database is
  patchy: if a code isn't a recognisable disc it is treated as not found (add it by hand).
- **Genres come from TMDB.** Without a key, discs still scan, but the genre is only a keyword
  guess on the title/description and is flagged *needs review*.
- Listings often don't say DVD vs Blu-ray, so the scan preview shows a format dropdown
  (with a "please check" note when it's a guess). 4K/UHD is treated as Blu-ray plus a `4k` tag.
- Movies have their own genre shelves (Action & Adventure, Animation, Comedy, Horror, ...)
  and are never mixed with book genres. Director is stored as the "author", runtime in minutes.
- **Fixing a wrong or "needs review" title:** open the movie, and the *Find this title on TMDB* box
  (needs `TMDB_API_KEY`) searches by title and optional year. Pick the right result and the form fills in
  director, studio, year, runtime, description, genre and tags; the poster is fetched when you save.
  Your DVD/Blu-ray choice, location, notes, status, and any genre you set yourself are kept. The server
  re-fetches the chosen title from TMDB on save rather than trusting what the browser sends.
- Existing libraries upgrade automatically on start (new columns are added; nothing is lost).

### Jellyfin and the Movies tab (`server/jellyfin.js`)
The **Movies** tab lists every disc you own *and* every movie on your Jellyfin server as one
list, each marked *Disc*, *Server* or both, with filters for disc-only / server-only. Tapping a film
shows where you have it, lets you edit the disc, open it in Jellyfin, or (for a server-only film) add
the disc you own with the details pre-filled.

- Only your Jellyfin **Movies** library is read (every library of type "Movies", or the one named in
  `JELLYFIN_MOVIE_LIBRARY`), so home-video, concert, extras and music-video folders never appear as films.
- Configure with `JELLYFIN_API_KEY` (create one in Jellyfin: Dashboard > Advanced > API Keys),
  `JELLYFIN_URL` (default `http://127.0.0.1:8096`) and optionally `JELLYFIN_PUBLIC_URL` for
  "Open in Jellyfin" links. Read-only: the app never changes anything on Jellyfin.
- A disc and a server movie are the same film when their TMDB ids match, otherwise when the titles
  match (ignoring "(Criterion Collection)", "The", punctuation, case) and the years are within one.
  Pick a disc's TMDB match in the editor to make the link exact.
- The key stays on the server; posters are fetched through the app (`/api/jellyfin/image/...`), so
  the browser never sees it. If Jellyfin is down or the key is wrong, the tab still lists your
  discs and says why the server side is missing. The server list is cached for 5 minutes
  (*Refresh server* forces it). Only movies for now, not TV series.

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

### Filtering by genre and tag
Both the **Library** and **Movies** tabs have *Genre* and *Tag* dropdowns (plus *Subgenre* in the Library once a genre
is picked). Options only list what exists for the current view and show how many results each would leave: tag counts
follow the chosen genre and genre counts follow the chosen tag, and the Library's lists follow the media filter
(books, movies, DVDs, Blu-rays). Filters live in the address (`#/library?media=book&genre=Horror&tag=classic`,
`#/movies?show=server&genre=Comedy`), so a refresh or a bookmark keeps them. Server-only films get tags from
Jellyfin's genres plus their decade, so they filter the same way as discs.

### Search
One box. Every word is a prefix match, all words must match (`frank her` finds Herbert),
across title, subtitle, author, tags, genre, description, notes and ISBN. Case and accents
are ignored (`miserables` finds *Les Misérables*). Title and author matches rank above
description matches. Input is turned into a quoted FTS5 query, so it can't inject syntax.
Reference: <https://www.sqlite.org/fts5.html>

## Configuration

See `.env.example`. `LIBRARY_PASSWORD`, `PORT`, `HOST`, `DATA_DIR`, `GOOGLE_BOOKS_API_KEY`,
`TLS_CERT`/`TLS_KEY`, `ALLOWED_ORIGINS`, `TRUST_PROXY`, `TMDB_API_KEY`, `JELLYFIN_API_KEY`, `JELLYFIN_URL`, `JELLYFIN_PUBLIC_URL`, `JELLYFIN_MOVIE_LIBRARY`, `ANTHROPIC_API_KEY`, `COVER_VISION_MODEL`, `COVER_SCAN`, `TESSERACT_PATH`.

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
11. **Cover scanning:** the pipeline ran end to end in a browser and on your live server with real Open Library
   searches, but only on catalogue images. Not yet tried with the phone camera (iOS Safari / Android Chrome file
   capture), real-world glare and angles, or the Claude reader against the real API (tested with a fake only).
10. **Jellyfin:** the integration is tested against a fake Jellyfin shaped like its documented API
   (<https://api.jellyfin.org/>), not yet against a real server. Matching quality on your real library is unknown.
9. **DVD/Blu-ray support:** UPCitemdb and TMDB responses (movie and TV) were each checked live once
   and the full barcode-to-genre path worked for the Spider-Man DVD. Accuracy across many real discs,
   format guesses, and TV box sets is untested.
8. **Title search against the live services.** `/api/search` parses Open Library's `search.json` and Google Books'
   `volumes?q=` responses as I remember their documented shapes, and was tested only against fixtures of that shape (the
   hosts were unreachable). Malformed or missing fields are ignored, so a mismatch should show as fewer/emptier results,
   not an error. Check one real search first. Ranking and de-duplication quality on real catalogue data is unproven.
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
server/barcode.js    UPC/EAN validation for discs
server/discs.js      DVD/Blu-ray lookup (UPCitemdb + TMDB), title cleanup
server/moviegenres.js movie genre shelves + classifier
server/jellyfin.js   Jellyfin client + disc/server movie matching
server/cover.js      cover photo -> text readers, search, ranking
server/native/       Swift helper for Apple Vision text recognition
public/              the app (vanilla JS, no build step) + service worker
tests/               node:test suites; tests/e2e is the browser run
```
