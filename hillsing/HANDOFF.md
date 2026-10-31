# HillSing: handoff

For a new Claude Code session running locally. Read this first, then `README.md` (architecture and
design rationale). Everything here was true at commit `2720a9c` (v0.3), branch
`claude/friendly-thompson-wuq1qh`.

## 1. Run the current build (do this first)

Requirements: Node 20+ (built and tested on Node 22), npm, **Chrome or Edge** (Web MIDI works there;
Safari/iOS probably lacks it, Firefox gates it behind a permission prompt).

```bash
git clone https://github.com/Headstone-Hills/hill-apps.git     # or: git fetch in an existing clone
cd hill-apps
git checkout claude/friendly-thompson-wuq1qh
cd hillsing
npm install
npm run build          # typechecks, then builds into dist/
npm run preview        # serves the production build at http://localhost:4173
```

Open <http://localhost:4173>.

**Use the production build (`build` + `preview`) for testing.** That is the only path I verified end
to end. `npm run dev` (Vite dev server, <http://localhost:5173>) should work but was **never run** in
this project: the AudioWorklet is loaded through Vite's `?worker&url` import (`src/synth/engine.ts`),
which is the first thing to suspect if dev mode misbehaves.

### Smoke test (2 minutes)

1. Click **Start audio** (browsers block sound until a click). Status should read "Audio on (… Hz)".
2. **Play without MIDI:** computer keys `A S D F G H J K L` are white keys, `W E T Y U O P` black
   keys, `Z`/`X` change octave. Each new key sings the next syllable of the lyrics. Hold one key and
   press another for **legato** (consonants must still be heard).
3. **MIDI keyboard:** plug it in before loading the page. Status line shows the device name. Mod
   wheel = vibrato, pitch wheel = bend, sustain pedal holds the vowel.
4. **Voice lab:** pick a voice, press the six listening phrases (Vowels, Opening consonants, Final
   consonants, Legato words, Sentence, Melisma). Watch the spectrogram.
5. **MIDI file:** choose any `.mid`, pick the melody track, press Play. Try *Vowel on the beat*,
   *Repeat lyrics*, tempo, click the preview to seek.
6. **Melisma:** replace the lyrics with `love ~ ~ hold ~ ~` and play.

### Automated checks

```bash
npm test               # 127 unit + DSP tests (Vitest). Should all pass.
npm run typecheck
```

Browser end-to-end checks (`e2e/live.cjs`, `e2e/midi-file.cjs`, `e2e/voice-lab.cjs`) drive headless
Chromium. They need Playwright, which is **deliberately not a project dependency**:

```bash
npm install --no-save playwright && npx playwright install chromium
npm run build && npm run preview &        # keep running on :4173 (or set HILLSING_URL)
node e2e/live.cjs && node e2e/midi-file.cjs && node e2e/voice-lab.cjs
```

All three passed at this commit. They measure real audio output (peak level, onset timing), so they
need a working audio stack; headless Chromium in the cloud sandbox was fine.

## 2. What this project is

A browser-based **singing instrument built on a hand-written real-time formant synthesizer** (an
"old-school TTS" in the Klatt style). You type lyrics; each note you play sings the next syllable at
that pitch. The original brief: choose a TTS voice, type/paste/import text, split into syllables,
play or import MIDI, one syllable per note with pitch/duration following the MIDI, and Vocaloid-like
per-note tuning.

**Repo context:** `hill-apps` is otherwise a suite of Python/pygame apps for an RG35XXSP handheld.
`hillsing/` is the first TypeScript/web app in it and shares no code or build steps with the others.
The top-level `README.md` lists it.

## 3. Decisions the user already made (do not re-litigate)

| Decision | Choice |
|---|---|
| Voice engine | **Formant / old-school only** (no neural TTS clips, so no time-stretching) |
| Synth source | **Write our own** Klatt-style synth |
| Licensing | **Permissive only.** No GPL, so **eSpeak NG is out**. CMUdict is BSD-style (notice ships in `public/cmudict-LICENSE.txt`) |
| Platform | Browser app **and** native Apple Silicon Mac build via **Electron** (not Tauri: WKWebView likely lacks Web MIDI; unverified) |
| Live play | **Monophonic with legato** |
| Stack | TypeScript + Vite, **no UI framework** |
| v1 tuning params | Pitch offset/curve, vibrato, formant/gender shift, consonant & vowel timing |
| Recording | Record live performances into the editor as notes + syllables |
| Melismas | **Wanted in the editor** (data model done, UI pending) |
| Language | English only for v1 |
| Voice quality bar | "Robotic but intelligible" is acceptable; quality is judged by the user's ears |
| Project name | **HillSing**, folder `hillsing/` |

User preferences: they like **cited sources and detailed explanations**, and do **not** like offers to
write their work for them. Keep suggestions concrete, ask before big scope jumps, and do not open a
pull request unless asked.

## 4. Current state

Done:

- **v0.1** live keyboard singing (MIDI + computer keys), syllabification, lyric pointer with
  step/skip/reset, MIDI-learn bindings for navigation.
- **v0.2** MIDI file playback: own SMF parser, mono reduction, legato, "vowel on the beat" pre-roll,
  sample-accurate sequencer inside the AudioWorklet, preview canvas, tempo, loop, seek.
- **v0.3** six voices, Voice Lab (6 listening phrases, live spectrogram, consonant level /
  breathiness / brightness, "Copy settings"), **legato that keeps consonants**, melisma (`~` in
  lyrics), per-note loudness equalisation, large synth-quality fixes (section 6).

Not built yet (agreed roadmap, in order): (3) **piano-roll editor** with a syllable row (current
preview is read-only), (4) **per-note tuning** with global → selection → note overrides,
(5) record live performances into the editor, (6) **Electron wrapper** for Apple Silicon,
(7) **WAV export** via `OfflineAudioContext` (the sequencer and synth are pure TS and already render
offline in tests).

**Immediate next step:** the user was about to do the **listening round** with the Voice Lab. No one
has *heard* v0.3. Ask for specific feedback per phrase and voice ("Copy settings" produces pasteable
text), then tune `src/synth/phonemes.ts` / `voices.ts`. After that, start the piano-roll editor.

## 5. Architecture (short; see README for detail)

```
text ─▶ tokenize ─▶ CMUdict ─▶ syllabify ─▶ LyricPointer          (src/text/)
MIDI/keys ─▶ Performance (mono, legato, pedal, bend)              (src/engine/performance.ts)
MIDI file ─▶ smf.ts ─▶ engine/sequence.ts (timed commands)        (src/midi/, src/engine/)
        both ─▶ SingerEngine ─postMessage─▶ AudioWorklet ─▶ KlattVoice (+ SequenceRunner)
                                                                  (src/synth/)
```

- `src/synth/klatt.ts`: the synth (pure TS, no Web Audio, runs in Node for tests). Glottal pulse →
  nasal notch → 5 cascaded formants + 2 fixed upper poles → parallel band-pass frication.
- `src/synth/phonemes.ts`: phoneme → timed parameter segments. Consonant noise levels are **solved
  numerically** against targets in `tests/levels.test.ts`; do not hand-tweak without rerunning it.
- `src/synth/voices.ts`: voice profiles (data only).
- `src/synth/sequence.ts`: sample-accurate player (worklet side). `src/engine/sequence.ts`: maps
  melody + lyrics to commands (main thread). `src/engine/audition.ts`: Voice Lab phrases.
- The engine is **note-on / note-off driven**, not clip-length driven (live note length is unknown at
  key-down): onset consonants → held vowel → coda queued at note-off.
- Only runtime dependencies: none. Dev: Vite, Vitest, TypeScript, `@types/node`.

## 6. Things learned the hard way (read before changing the synth)

- **Measure, don't guess; no one on the project can hear in the cloud.** The earlier sessions found
  real defects only by measuring output (spectra, band energy, RMS): vowels ~25 dB too dark above
  4 kHz (glottal closure was a fraction of the period; now an absolute `closeMs`), /s/ was
  broadband (frication used a DC-normalised resonator; now a true band-pass), and loudness varied
  ±8 dB across pitch (now probed per note in `vowelGain()`).
- **Vowel loudness is computed per note** by rendering a short probe (`klatt.ts`, `vowelGain`,
  cached). Voiced-consonant amplitudes scale with the syllable's vowel gain (`V()` in `phonemes.ts`).
  Do not reintroduce a fixed per-vowel table.
- **Legato must keep consonants.** The synth plays the previous syllable's coda and the new onset
  between the two vowels (`klatt.ts` `noteOn`, `connecting`). The mapper adds matching pre-roll
  (`engine/sequence.ts`). Tests mutation-checked: restoring the old behaviour fails three tests.
- **Control tick must advance by the real chunk length** (`tick(n)`), else timing drifts when the
  sequencer splits a render at command boundaries (a test covers it).
- **Several of my own tests were vacuous at first** (`|| true`, merged onset detectors, metrics
  dominated by the vowel). Prefer A/B comparisons and mutation checks; do not add assertions you
  have not seen fail.
- **Cheap detectors:** "was a consonant sung" = energy above 4 kHz in a 46 ms window (fricative/burst
  are 40+ dB above vowels there); a fricative differs from a burst by *duration*.
- **Pitch bug history:** a zero-mean-noise mistake once made pitch 8% flat; the pitch-accuracy tests
  (110/220/440 Hz) exist because of it.
- **Tooling gotchas:** `pkill -f "vite preview"` can match and kill its own shell. Kill by port
  instead. Scratch tests named `tests/_*.test.ts` are gitignored (used for offline spectrogram PNGs and
  measurement; recreate as needed, they are not part of the suite).
- Web MIDI needs a secure context (`localhost` counts) and, in Chrome, may show a permission prompt.
  AudioContext needs a user click (hence **Start audio**, and the lab/file Play buttons start it too).

## 7. Known limitations and unverified items

- **Voice quality has never been judged by ear.** Expect robotic and possibly harsh results.
  Consonant loci, burst frequencies, bandwidths and durations are hand-chosen starting points.
- **Female/child vowel formants were written from memory** of Peterson & Barney (1952), not checked
  against the paper. Klatt (1980), Rosenberg (1971) and P&B (1952) are the cited references.
- Voices are parameter sets of one engine, not distinct engines.
- Child/female voices at high pitch: harmonics are sparse relative to formants (inherent to the
  method); loudness is equalised but timbre can vary across the range.
- Not tested: `npm run dev`, a real MIDI keyboard (only a fake Web MIDI device in e2e), Safari,
  Firefox, mobile, Bluetooth audio latency. Latency in the header is the browser's own estimate.
- MIDI files: lyric meta-events, pitch bend and CCs are ignored; format 2 is rejected; one syllable
  per note unless `~` is used.
- Monophonic only; English only; numbers are read digit by digit.

## 8. Open questions for the user

1. Listening-round results: which phrase/voice is worst? (Use the lab; ask for "Copy settings".)
2. Start the piano-roll editor now, or another quality pass first?
3. Editor scope check: melisma creation UI, per-note tuning overrides model (global → selection →
   note), and how recorded live takes map syllables to notes.
4. Latency target (I assumed under ~20 ms key-to-sound; the browser may not allow it everywhere).

## 9. Git workflow

- Work on `claude/friendly-thompson-wuq1qh`. Commits so far: `f5f42db` (v0.1), `1bd8bc9` (v0.2),
  `2720a9c` (v0.3), plus this handoff.
- No pull request has been opened; do not open one unless the user asks.
- Commit messages in this repo end with the attribution lines the session provides.
- Keep `README.md` in sync when behaviour changes (it documents rules, limitations and tests).
