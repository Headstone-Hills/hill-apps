# HillSing

Type lyrics, play them on a MIDI keyboard. A browser-based singing instrument built on a
real-time **formant synthesizer** (old-school TTS style): each key press sings the next syllable
of your text at the pitch you play, holds the vowel as long as the key is down, and releases
into the syllable's final consonants.

**v0.2** does two things: **live keyboard singing**, and **MIDI file playback** (load a melody,
sing your lyrics over it, one syllable per note). Not built yet: the piano-roll editor with
per-note tuning, live-performance recording, WAV export and the Electron (Apple Silicon)
wrapper. See [Roadmap](#roadmap).

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

Controls: hold one key and press another for **legato** (glides into the next syllable's vowel,
no new consonant). Mod wheel = vibrato, pitch wheel = bend (±2 semitones), sustain pedal holds
the vowel. Arrow keys / Home (or bound MIDI keys, see below) move through the lyrics.

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
- **Overlapping notes are legato** (glide into the next vowel, no new consonant), exactly like
  holding one key while pressing the next. Notes that merely touch are re-articulated.
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

Per-vowel loudness compensation and consonant levels are **measured, not guessed**: each phoneme
was rendered offline and the constants in `phonemes.ts` equalise sustained-vowel RMS (within
0.2 dB) and set fricatives roughly 8–19 dB below the vowel.

### Syllabification

Maximal-onset principle with a table of legal English onsets, plus one correction: a single
consonant between a stressed lax vowel and an unstressed vowel stays with the first syllable
(`lem·on`, `lit·tle`; tense vowels keep `pa·per`). Syllable boundaries are genuinely ambiguous
(ambisyllabic consonants), so this is a deterministic, documented convention, not ground truth.
The syllable label on each chip is a cosmetic split of the spelling; the **sung** syllables
come from the phonemes. Words missing from the dictionary get a crude letter-to-sound guess and are
shown with a dashed underline.

## Tests

`npm test` runs 78 tests, including checks on the actual audio: formant peak positions (spectral
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
not a project dependency; see the header of each script.

## Known limitations (v0.1)

- **Voice quality is the main risk and is not yet tuned by ear.** The formant frequencies are
  approximate adult-male averages (in the style of Peterson & Barney 1952 as commonly tabulated);
  consonant loci, burst frequencies, bandwidths and durations are hand-chosen starting points.
  Expect to adjust `phonemes.ts` by ear. Robotic is the aesthetic; muffled or harsh consonants
  are bugs to tune.
- Consonants delay the vowel after key-down (default 0.7× natural length; lower *Consonant length*
  to get the vowel closer to the key press).
- Legato drops the new syllable's onset consonant and the old syllable's coda by design.
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

## Roadmap

Agreed scope, in order: (1) **v0.1 live singing**, done; (2) **MIDI file import and playback**
through the same engine, done in v0.2; (3) piano-roll editor with a syllable row (the current preview is read-only); (4) per-note tuning: pitch
offset/curve, vibrato, formant shift, consonant/vowel timing, with global → selection → note
overrides; (5) record live performances into the editor as notes + syllables; (6) Electron wrapper
for Apple Silicon Mac (Electron rather than Tauri, since Chromium keeps Web MIDI behaviour
identical); (7) WAV export through the same engine in an `OfflineAudioContext`.
