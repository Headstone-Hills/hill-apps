# HillSing

Type lyrics, play them on a MIDI keyboard. A browser-based singing instrument built on a
real-time **formant synthesizer** (old-school TTS style): each key press sings the next syllable
of your text at the pitch you play, holds the vowel as long as the key is down, and releases
into the syllable's final consonants.

This is **v0.1: live keyboard singing**. Not built yet: MIDI file playback, the piano-roll
editor with per-note tuning, live-performance recording, and the Electron (Apple Silicon)
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
| `src/midi/` | Web MIDI input, message parser, MIDI-learn navigation bindings |
| `src/ui/` | DOM UI (no framework) |

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

`npm test` runs 51 tests, including checks on the actual audio: formant peak positions (spectral
analysis), pitch accuracy at 110/220/440 Hz, no clipping, legato continuity versus articulated
notes, coda-after-note-off, silence after release, and that every phoneme in the dictionary has a
synth recipe. The syllabifier is run over all 126,052 dictionary words. These verify that the
signal is *correct*, not that it sounds *good*: that needs listening.

## Known limitations (v0.1)

- **Voice quality is the main risk and is not yet tuned by ear.** The formant frequencies are
  approximate adult-male averages (in the style of Peterson & Barney 1952 as commonly tabulated);
  consonant loci, burst frequencies, bandwidths and durations are hand-chosen starting points.
  Expect to adjust `phonemes.ts` by ear. Robotic is the aesthetic; muffled or harsh consonants
  are bugs to tune.
- Consonants delay the vowel after key-down (default 0.7× natural length; lower *Consonant length*
  to get the vowel closer to the key press).
- Legato drops the new syllable's onset consonant and the old syllable's coda by design.
- Monophonic only. English only.
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

Agreed scope, in order: (1) **v0.1 live singing**, this release; (2) MIDI file import and playback
through the same engine; (3) piano-roll editor with a syllable row; (4) per-note tuning: pitch
offset/curve, vibrato, formant shift, consonant/vowel timing, with global → selection → note
overrides; (5) record live performances into the editor as notes + syllables; (6) Electron wrapper
for Apple Silicon Mac (Electron rather than Tauri, since Chromium keeps Web MIDI behaviour
identical); (7) WAV export through the same engine in an `OfflineAudioContext`.
