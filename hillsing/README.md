# HillSing

Type lyrics, play them on a MIDI keyboard. A browser-based singing instrument built on a
real-time **formant synthesizer** (old-school TTS style): each key press sings the next syllable
of your text at the pitch you play, holds the vowel as long as the key is down, and releases
into the syllable's final consonants.

**v0.3** does three things: **live keyboard singing**, **MIDI file playback** (load a melody, sing
your lyrics over it, one syllable per note, with melismas), and a **Voice Lab** with six voices and
listening phrases for judging quality. Not built yet: the piano-roll editor with per-note tuning,
live-performance recording, WAV export and the Electron (Apple Silicon) wrapper. See
[Roadmap](#roadmap).

> Unlike the other apps in this repo (Python/pygame for the RG35XXSP handheld), HillSing is
> a TypeScript web app. It shares no code or build steps with them.

## Run it

Requires Node 20+.

```
cd hillsing
npm install
npm run dev        # http://localhost:5173
npm test           # unit + DSP tests (Vitest)
npm run build      # typecheck + production build into dist/
```

1. Click **Start audio** (browsers require a click before sound).
2. Type lyrics. Words are split into syllables; the highlighted one is sung by the next note.
3. Play: a MIDI keyboard (Chrome/Edge), or the computer keyboard (`A S D F G H J K L` white
   keys, `W E T Y U O P` black keys, `Z`/`X` octave).

Controls: hold one key and press another for **legato**: the pitch glides and the vowel changes
without a gap, but **no consonants are lost**. The previous word's final consonants and the new
syllable's opening consonants are sung between the two vowels. Mod wheel = vibrato, pitch wheel = bend (±2 semitones), sustain pedal holds
the vowel. Arrow keys / Home (or bound MIDI keys, see below) move through the lyrics.

### Voice lab (the listening round)

Pick a **voice** (Classic, Deep, Female, Child, Robot, Breathy), then play a **listening phrase**.
Each phrase isolates one thing to listen for, so feedback can be specific:

| Phrase | Listen for |
|---|---|
| Vowels | Ten vowels and three diphthongs on one pitch: distinct? equally loud? do *ai / ow / ay* glide smoothly? |
| Opening consonants | 21 consonants + "ah": which are weak, harsh, or confused with another? |
| Final consonants | "ah" + each ending: can you hear the word ending? |
| Legato words | Overlapping notes: is every consonant still heard between the vowels? |
| Sentence | "The quick brown fox...": intelligible? |
| Melisma | One syllable held across several notes; the word ends only on the last. |

A live **spectrogram** (0–8 kHz) shows what is being produced: vowels as horizontal formant
bands, /s/ as high-frequency noise, stops as a gap followed by a burst. Extra controls: *Consonant
level*, *Breathiness* and *Brightness* (a high shelf, about ±8 dB). **Copy settings** gives the voice
and every slider value as text to paste into a message ("female, brightness +0.4, consonant level
1.5: the /s/ is still too soft").

The voices are parameter sets for one synthesis engine (vocal-tract size, glottal pulse, bandwidths,
breathiness, drift), not different engines, so expect a family resemblance. The female and child
vowel formants follow the pattern of the Peterson & Barney (1952) averages but were written from
memory and **not checked against the paper**.

### Melisma (holding a vowel across notes)

Write `~` (or `_`) after a word for each extra note: `love ~ ~` is one syllable over three notes.
The first note carries the opening consonants, the last note carries the word ending, and the notes
in between are the held vowel. A diphthong holds its first vowel and glides only on the last note
(`fly ~ ~` sustains "ah" and closes to "ee" at the end), as singers do. In a MIDI file, melisma
notes join the previous note without a release when they touch (or are within 120 ms of it); after
a real rest the vowel re-attacks. The upcoming piano-roll editor will build on this: melismas are
already part of the lyric data model, so the editor needs only a UI for creating them.

### MIDI file playback

1. Type lyrics, then choose a `.mid` file under **MIDI file**. The busiest pitched track is picked
   as the melody (change it with *Melody track*; percussion-only tracks are not offered).
2. **Play / Pause / Stop** (or `Space`), *Loop*, and a *Tempo* slider (50-150%). Click the preview
   to jump anywhere. Each note shows the syllable it will sing.
3. Live playing still works at the same time.

How notes become syllables:

- **One syllable per note, in order.** Extra notes stay silent (the panel says how many); turn on
  *Repeat lyrics* to cycle the lyrics instead. If the melody is shorter than the lyrics, unused
  syllables are reported.
- **Chords** are reduced to their top note.
- **Overlapping notes are legato**, exactly like holding one key while pressing the next: pitch
  glides, but the previous word's ending and the next word's start are still sung between the
  vowels (they are pre-rolled so the new vowel still lands on the beat). Notes that merely touch are
  re-articulated, except melisma notes (above), which join.
- **Vowel on the beat** (on by default): a syllable's onset consonants start *before* the note so
  the vowel lands on the beat, and its final consonants finish *inside* the note. Without it,
  consonants begin at the note start like live playing, so vowels arrive late. The pre-roll uses
  the *Consonant length* setting, so changing that slider re-times the sequence.
- Velocity is used; pitch bend, controllers and lyric meta-events in the file are ignored.
  Format 0 and 1 files, tempo changes and SMPTE timing are supported; format 2 is not.

Playback is scheduled **inside the AudioWorklet, sample-accurately**, not by main-thread timers, so
it does not jitter when the page is busy. Measured end to end in headless Chromium, notes written
1.000 s apart come out 1.000 s apart, and 50% tempo gives 2.0 s.

### Lyric navigation

Step back / skip forward by syllable or by word, reset to start, and optional loop at end.
Buttons, arrow keys (`Shift` = word, `Home` = reset), or **MIDI learn**: click *Learn* next to an
action, then press a spare key, pad or CC button. Bound notes are never sung. Bindings persist in
`localStorage`.

## How it works

```
text ──▶ tokenize ──▶ CMUdict (ARPAbet) ──▶ syllabify ──▶ LyricPointer
                         └─ unknown word: letter-to-sound guess
MIDI / computer keys ──▶ Performance (mono, legato, pedal, bend)
                              │ note-on(syllable, f0, velocity, legato) / note-off / setF0
                              ▼
                  SingerEngine ──postMessage──▶ AudioWorklet ──▶ KlattVoice
```

| Path | Role |
|---|---|
| `src/text/` | `cmudict.ts` dictionary + possessives, `syllabify.ts`, `g2p.ts` fallback, `lyrics.ts`, `pointer.ts` |
| `src/synth/klatt.ts` | The synthesizer (pure TS, no Web Audio dependency, testable in Node) |
| `src/synth/phonemes.ts` | Phoneme → timed parameter segments (formant tables, consonant recipes) |
| `src/synth/worklet.ts`, `engine.ts` | AudioWorklet wrapper and main-thread handle |
| `src/engine/performance.ts` | Note handling: mono, last-note priority, legato, sustain, bend |
| `src/midi/` | Web MIDI input, message parser, MIDI-learn bindings, `smf.ts` MIDI file reader |
| `src/engine/sequence.ts` | Melody + lyrics -> timed voice commands (mono reduction, legato, vowel-on-the-beat timing) |
| `src/synth/sequence.ts` | Sample-accurate sequence player running inside the worklet (pure TS, testable in Node) |
| `src/ui/` | DOM UI (no framework): `app.ts`, `midifile.ts` (file panel), `preview.ts` (note preview canvas) |

### Note-driven playback

The key design point is that the engine is driven by **note-on / note-off events, not clip
lengths**, because a live player's note length is unknown at key-down. A syllable is:

1. **Onset consonants**: fixed-length segments (scaled by *Consonant length*).
2. **Vowel**: held until note-off (diphthongs glide between two targets), minimum 60 ms so even
   a tap is intelligible.
3. **Coda consonants + fade**: queued at note-off. So final consonants sound *after* you release.

Because there is no recorded audio, there is no time-stretching: the synth generates the syllable
at the requested pitch and duration. MIDI file playback (later) will feed the same note events.

### The synthesizer

A simplified cascade/parallel formant synthesizer after Klatt (1980): a glottal pulse train
(Rosenberg-style pulse, differentiated for lip radiation, with a brightness filter tied to
velocity) plus aspiration noise, through a nasal notch and five cascaded formant resonators;
a parallel resonated-noise branch makes frication and stop bursts. Control parameters are
interpolated at 1/16 of the sample rate. Pitch glides in the log domain; a small random drift and
optional vibrato keep the tone from sounding static.

Loudness is **measured, not guessed**. Vowel loudness depends on where the harmonics fall relative
to the formants (it varied by up to ±8 dB across the singing range with a fixed gain table), so at
every note-on the synth briefly renders a probe of the target vowel at that exact pitch, measures
its RMS and sets the gain, bringing every vowel in every voice to the same level. Consonant noise
levels were solved numerically against targets (fricatives about 8–19 dB below the vowel; bursts
6–10 dB below) and are guarded by `tests/levels.test.ts`.

Two defects found by looking at the output (offline spectrograms and band-energy measurements)
are worth recording, since they are exactly what a listener would have heard as "muffled" and "hissy":

- The glottal pulse closed over 16% of the period (1.2 ms at 130 Hz), which acts as a −12 dB/octave
  low-pass from a few hundred Hz. Vowels were about 25 dB too dark at 4–6 kHz and nearly silent above
  that. Closure is now an absolute time (0.3 ms for the classic voice) and fixed upper poles are
  added (the usual "higher-pole correction"). Vowels now fall to roughly −45 dB at 5 kHz, a speech-like slope.
- Frication used a DC-normalised resonator, which passes everything below its centre at full gain,
  so /s/ was broadband noise. It is now a true band-pass, normalised for noise bandwidth.

### Syllabification

Maximal-onset principle with a table of legal English onsets, plus one correction: a single
consonant between a stressed lax vowel and an unstressed vowel stays with the first syllable
(`lem·on`, `lit·tle`; tense vowels keep `pa·per`). Syllable boundaries are genuinely ambiguous
(ambisyllabic consonants), so this is a deterministic, documented convention, not ground truth.
The syllable label on each chip is a cosmetic split of the spelling; the **sung** syllables
come from the phonemes. Words missing from the dictionary get a crude letter-to-sound guess and are
shown with a dashed underline.

## Tests

`npm test` runs 127 tests, including checks on the actual audio: formant peak positions (spectral
analysis), pitch accuracy at 110/220/440 Hz, no clipping, legato continuity versus articulated
notes, coda-after-note-off, silence after release, and that every phoneme in the dictionary has a
synth recipe. The syllabifier is run over all 126,052 dictionary words. The MIDI reader is tested
on synthetic files (tempo maps, running status, SMPTE, truncated and garbage input), and the
sequencer is rendered offline to measure that vowels land on the beat and that timing is identical
however the audio callback is chunked. These verify that the signal is *correct*, not that it
sounds *good*: that needs listening.

`e2e/` has two browser checks (headless Chromium, using a fake MIDI device and a generated `.mid`
file) that exercise the real app: audio starts, keys and MIDI make sound, MIDI-learn, file loading,
play/pause/stop/seek, tempo, and measured onset timing. They need Playwright, which is deliberately
not a project dependency; see the header of each script. `e2e/voice-lab.cjs` covers the lab:
every phrase with every voice (audible, unclipped), voice persistence, sliders, settings text and the
live spectrogram.

## Known limitations (v0.1)

- **Voice quality is the main risk and is not yet tuned by ear.** The formant frequencies are
  approximate adult-male averages (in the style of Peterson & Barney 1952 as commonly tabulated);
  consonant loci, burst frequencies, bandwidths and durations are hand-chosen starting points.
  Expect to adjust `phonemes.ts` by ear. Robotic is the aesthetic; muffled or harsh consonants
  are bugs to tune.
- Consonants delay the vowel after key-down (default 0.7× natural length; lower *Consonant length*
  to get the vowel closer to the key press).
- Legato adds the connecting consonants *between* the vowels, so a legato note with a long
  consonant cluster arrives later than the key press (live) or needs the pre-roll (files).
- The synth is hand-built: expect robotic and sometimes harsh results. The Voice Lab exists so
  that the remaining quality gaps can be found by ear and reported specifically.
- Monophonic only. English only. Files with several vocal parts need one track chosen.
- MIDI files: lyrics embedded in the file are not read, and the melody is one syllable per note
  (no melismas yet). Very short notes may not leave room for long consonant clusters.
- Web MIDI: Chrome/Edge work. Firefox gates it behind a permission prompt. Safari/iOS lacked
  support when this was written, so use the computer keyboard there. Check
  [caniuse](https://caniuse.com/midi).
- Latency: the browser reports its own output latency (shown in the header). Wired output on
  desktop Chrome is typically best; Bluetooth adds a lot.
- Numbers are read digit by digit ("12" → "one two").

## Licensing

Everything here is permissive; no GPL code (this is why eSpeak NG, which is GPL-3.0, is not used).

- **CMU Pronouncing Dictionary** (`public/cmudict.txt`, built by `npm run build:dict` from
  [cmusphinx/cmudict](https://github.com/cmusphinx/cmudict)): Copyright 1993-2015 Carnegie Mellon
  University, BSD-style two-clause license. The notice must accompany copies of the data, so
  [`public/cmudict-LICENSE.txt`](public/cmudict-LICENSE.txt) ships alongside it.
  Only the first pronunciation of each word is kept (126,052 entries).
- Dev dependencies only (Vite, Vitest, TypeScript, `@types/node`), all MIT/Apache-2.0. There are
  no runtime dependencies.

## References

- Klatt, D. H. (1980). Software for a cascade/parallel formant synthesizer. *J. Acoust. Soc. Am.*
  67(3), 971–995.
- Rosenberg, A. E. (1971). Effect of glottal pulse shape on the quality of natural vowels.
  *J. Acoust. Soc. Am.* 49(2B), 583–590.
- Peterson, G. E., & Barney, H. L. (1952). Control methods used in a study of the vowels.
  *J. Acoust. Soc. Am.* 24(2), 175–184.
- Web MIDI API: <https://developer.mozilla.org/en-US/docs/Web/API/Web_MIDI_API>;
  AudioWorklet: <https://developer.mozilla.org/en-US/docs/Web/API/Web_Audio_API/Using_AudioWorklet>

The formant values and gains in this project were chosen from commonly tabulated figures and then
tuned against measurements; they were not copied from a specific table, so check the papers above
before citing any individual number.

## Effects

Seven effects apply to every voice (formant, live samples, sample melodies, Voice lab) through one shared audio path
(`src/audio/bus.ts`): **bitcrush -> distortion -> EQ -> compressor -> chorus -> delay -> reverb**, in that order. The *Effects*
section has a card per effect (on/off + sliders), presets (small room, big hall, wide chorus, lo-fi tape, old radio, robot choir)
and a reset; settings are saved in the browser. Everything is off by default. Meter and spectrogram show the
processed sound. Bitcrush runs in its own AudioWorklet (`src/audio/bitcrush.worklet.ts`); if a browser blocks
worklets the card says so and the other five still work. Chorus is two delay lines modulated by one low-frequency oscillator in opposite directions, panned left and right. Reverb uses a generated impulse response (decaying
noise that darkens with time; rebuilt shortly after you stop moving its sliders). Pure maths and the settings
model are in `src/audio/fxparams.ts` (tested); the Web Audio wiring is `src/audio/effects.ts`.

## Hold word

*Hold word* (button, backtick key, or a MIDI-learned pad) stops the lyric pointer: new notes keep singing the current
syllable, e.g. to give one word several notes, until you release the hold. The position label reads "holding on".

## Piano-roll editor

The melody panel is an editor, not just a preview. Load a MIDI file (or press **New melody**) and:

- **Add** a note by double-clicking empty space; **move** by dragging (time and pitch), **resize** by dragging
  either edge, **delete** with Delete/Backspace. Shift-click adds to the selection, Cmd/Ctrl+A selects all.
- **Keys** (with the roll focused): arrows transpose (Up/Down, Shift = octave) and nudge (Left/Right, Shift = x4),
  Q quantizes the selection (or everything), M toggles a melisma, +/- zoom, Cmd/Ctrl+Z undo, Shift+Cmd/Ctrl+Z redo.
- **Snap** is a fraction of a beat at the file's first tempo. **Fit** and the scroll bar navigate; Cmd/Ctrl+scroll
  zooms. Click the ruler (or empty space) to move the playhead.
- **The word lane** under the notes shows the syllable each note sings, in order; a red dash means the note has no
  syllable. Double-click a word to change it (it rewrites that word in the Lyrics box). **Hold vowel ~** (or M) makes
  the selected note continue the previous word's vowel: it inserts a `~` before the word, exactly as typing it would.
- **Undo** covers note edits and the lyric changes made from the roll. Edits are made on a copy of the notes; the
  file itself is never changed. Times are in the file's own tempo; the Tempo slider only changes playback speed.

Code: `src/editor/notes.ts` (note operations and undo history, pure), `src/editor/lyricEdit.ts` (lyric-text
edits, pure), `src/ui/pianoroll.ts` (the canvas), `src/ui/midifile.ts` (wiring to playback). Tests:
`tests/editor.test.ts`. The canvas only covers the visible window, so long songs do not hit canvas size limits.

## UTAU voicebanks

A second voice source next to the formant synth: recorded UTAU samples, pitch-shifted by a TD-PSOLA
sampler. Load a bank in the *UTAU voicebank* panel, then choose it in the **Voice** chooser next to the lyrics (it lists the
generated formant voices and, in its own group, the sample-based voicebank; picking it with none loaded opens the
folder picker): Play renders the melody (from a MIDI file or the piano roll, with your English lyrics) into audio with the
bank's samples. **Live keys (computer keyboard or MIDI) play the voicebank too** (`src/utau/live.ts`,
`src/utau/livenote.ts`): a key press builds the syllable's consonants and the start of its vowel from the bank
(1-4 ms of work), then a seamless loop of the steady vowel sustains it until release; release fades the loop under
the syllable's ending samples (`E d`, or `E -` for a plain vowel). Holding one key and pressing another is legato
(the previous vowel closes into the new consonant); pitch bend re-tunes what is sounding. All samples are decoded
when the bank loads (about 0.3 s for the English Teto bank, roughly 190 MB of memory) so the first key press never
waits. The Voice lab and its sliders stay on the formant synth, and the vowel is not on the beat live: it follows its
consonant, as when a singer starts a word.

How the live sound is kept clean (each point was measured on the English Teto bank): to keep the delay after a key
press at 5-20 ms, silence at the start of a sample (the closure before a "t" or "k") is trimmed, consonants play 1.5x
faster live (UTAU's consonant velocity), and the sustain is built after the attack has started. The held vowel is a
seven-second sweep across the *middle half of the sample's steadiest stretch* (found from its spectrum, so a diphthong
like "ow" is held on its first element instead of sliding back and forth every 1.2 s); grains are widened up to 1.5x
when the pitch is lowered (two-period grains stop overlapping below about half the recorded pitch, and each cycle
becomes a burst with silence after it: 82 dB gating and peaks 1.7x the recording at 131 Hz); and buffers are capped at
0.95. Releases start just before the ending sample's own fade and are level-matched to the sustain, and a tap always
plays the consonant, the vowel start and 100 ms of sustained vowel.

**How English becomes samples.** Each dictionary syllable (ARPAbet) is mapped to X-SAMPA and to a chain of
aliases (`src/utau/planner.ts`), e.g. *hello* = `- h@`, then `@ l` + `- loU`, ending `oU -`: a start from silence,
the vowel closing into the next consonant, a fresh start, a release. Every choice falls back to an alias that
exists, so banks with other alias sets still sing, less smoothly. `src/utau/render.ts` places each alias using its
oto.ini timing (preutterance puts the vowel on the note's beat; overlap sets the crossfade with the previous
sound), shifts it to the note's pitch and crossfades the chain. The melody panel's *Aliases used* list shows what
each word used, which is the thing to check when a word sounds wrong. Joins are rendered at the previous note's
pitch; the bank is assumed to be an X-SAMPA English bank (the English Kasane Teto bank is).

- **Loading:** *UTAU voicebank* panel -> pick the folder that contains `oto.ini` (or a parent of it). Files are
  read in place by the browser; nothing is uploaded, copied or bundled. **Voicebanks are copyrighted and their
  terms usually forbid redistribution, so none may ever be added to this repository or the app.**
- **Formats:** `oto.ini` (Shift-JIS or UTF-8), 8/16/24/32-bit PCM or float WAV, and UTAU `.frq` pitch files
  (`FREQ0003`). Cutoff follows UTAU: negative = usable length after the offset, positive = trimmed from the end.
- **Sampler:** `src/utau/psola.ts`. The consonant part (the oto *consonant* length) plays at natural speed;
  the rest is stretched to the requested length; voiced parts are re-spaced at the target pitch from the bank's
  own pitch track, unvoiced parts are copied unshifted. Good for moderate shifts (about a fourth either way).
- **Tests:** `tests/utau.test.ts` uses synthetic data only. To also check a real bank on your machine:
  `HILLSING_UTAU_BANK=/path/to/bank/folder npx vitest run tests/utau.test.ts`.

Checked against the English "Kasane Teto" bank (X-SAMPA phonemes, VCV + CV aliases such as `- @`, `@ h@`, `h@`):
2,681 entries, no missing WAVs, and a rendered sample's measured pitch is within 0.5% of the request from
180 to 330 Hz.

**Zip voicebanks (phones).** The UTAU panel also takes a **.zip** of the bank folder (`src/utau/zip.ts`): the archive's
index is parsed and each file is sliced out and inflated only when used, so nothing is extracted or uploaded. Folder
names written in Shift-JIS are decoded. On touch devices samples are decoded when first used rather than all at once.
Zip64 and password-protected archives are rejected with a message. Check a real zip with
`HILLSING_UTAU_ZIP=/path/bank.zip npx vitest run tests/utau.test.ts`. A published artifact copy of the app carries no
voicebank: load your own zip from the phone.

## Roadmap

Agreed scope, in order: (1) **v0.1 live singing**, done; (2) **MIDI file import and playback**
through the same engine, done in v0.2; (2b) **voice lab, multiple voices, legato that keeps
consonants, melisma data model**, done in v0.3; (3) piano-roll editor with a syllable row, done in v0.4 (see "Piano-roll editor"); (4) per-note tuning: pitch
offset/curve, vibrato, formant shift, consonant/vowel timing, with global → selection → note
overrides; (5) record live performances into the editor as notes + syllables; (6) Electron wrapper
for Apple Silicon Mac, **done** (`electron/main.cjs`; `npm run app:build` writes
`release/HillSing-darwin-arm64/HillSing.app`; Electron rather than Tauri, since Chromium keeps Web MIDI
behaviour identical); (7) WAV export through the same engine in an `OfflineAudioContext`.

## Perform tab

The **Perform** tab in the header puts the voice picker, the lyrics box, the syllable parser (with Hold word) and all seven effects on one screen, and hides the MIDI file, UTAU and lab panels. Everything is shared with **Studio**, so switching tabs changes nothing but the layout. The choice is remembered, and `#perform` / `#studio` in the address open a tab directly.

## Melisma (hold Shift)

While you hold **Shift** (or press and hold the **〰 Melisma** button, which is the way to do it on a phone), notes you play while a word is sounding glide to the new pitch on the vowel being sung, with no new syllable and no new onset. If no key is down when you press Shift, the next note simply sings the next syllable. Release Shift and the next note sings the next syllable. This is the live version of writing `~` after a word.

## Sample voice tuning

The **Sample voice tuning** panel (in both Studio and Perform) holds the knobs a UTAU editor gives you, for live keys and for melodies sung by the voicebank: consonant speed (velocity), consonant level, consonant tone (a low-pass that rounds off harsh K/T/S bursts), gender (formant colour, pitch unchanged), vibrato depth and rate, glide (portamento between legato keys, live only), fine tune and volume. Changes are saved, and the next Play of a melody renders with them. Per-note tuning in the piano roll is not built yet.

## Scope, recording and test text (Perform tab)

The **Scope** shows the last few seconds of everything you hear: a waveform (read before the output limiter, so a peak past full scale shows red) over a spectrogram (0-8 kHz), on one time axis, with a gold line for every syllable and every key release. **Freeze** stops it so you can hover and read off the moment (time, peak, loudest band, the syllable); **Freeze on clip** freezes it by itself the instant the signal reaches full scale. **PNG** and **Copy** export the picture with the voice, tuning and effects settings printed at the top.

**Record** keeps everything the scope sees (up to three minutes). Stop it and the whole recording is saved as one tall PNG, in rows of about eleven seconds, with the syllable marks, a time axis and a count of clipped columns. After a recording, **PNG** and **Copy** export it; **Clear** goes back to the live window.

The **Test text** buttons next to the lyrics box load a text that uses every sound of English once (**All sounds**) or an ordinary sentence (**Sentence**).

**Flagging artifacts:** press **Q** while a syllable is playing (or within a couple of seconds after it) to flag it as having a strange sound artifact. The syllable's chip gets a red outline, a red ⚠ line goes on the scope (and into a recording's PNG), and it is added to a saved list. **⚑ n** copies the list as text (syllable, its sounds, note, voice, time); **✕** clears it.

**Finding which part of a note has an artifact:** the **Hear only** tick boxes in the tuning row mute everything except the parts you tick: **start** (the consonant and the opening of the vowel), **held vowel**, **ending**. **still vowel** holds the vowel steady instead of letting it drift slowly. They apply to live sample-voice notes and are not saved.

The Scope can be folded away with the ▾ button in its heading (the choice is remembered). A recording keeps running while it is folded, and the heading shows a red ● REC.

**Locking and highlighting:** **Caps Lock** locks on the current syllable (the same as the Hold button) until you switch it off; if you turn it on while a syllable is sounding, that syllable is the one locked. While a key is held, the syllable being sung stays highlighted (solid gold) and the highlight moves to the next one only when the key comes up. The arrow keys always step by syllable; Alt/Option+arrow jumps a whole word.

**Loading a voicebank from any tab:** under the Voice menu there is a **📂 Load voicebank (.zip)** button (and **📁 Folder** on desktop browsers), so a voicebank can be loaded from the Perform tab and from a phone, with its progress and any error shown right beneath. Choosing "Load a UTAU voicebank…" in the menu opens the same picker (a folder on desktop, a .zip on touch screens).

**Tab and MIDI controllers:** **Tab** steps back one syllable (the arrow keys step back and forward; Tab leaves a text box's own tabbing alone). On a MIDI keyboard the **pitch wheel/knob** bends the sounding note on either voice and follows at once (not at the glide time), and the keyboard's own pitch-bend-range setting (RPN 0) is honoured, otherwise the range is ±2 semitones. The **mod wheel/knob** (CC 1) adds vibrato: on the formant voices as before, and on the sample voice up to 80 cents on top of the tuning panel's own vibrato, changing live while a note sounds. The sustain pedal (CC 64) holds the note.
