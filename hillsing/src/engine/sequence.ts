// Turns a MIDI melody + lyrics into timed voice commands (the "score" for the synth).
//
// Rules (see README "MIDI file playback"):
//  - Melody is made monophonic: of notes starting together (a chord), the highest is sung.
//  - One syllable per note, in order. Extra notes are silent unless lyrics repeat.
//  - A note that overlaps the previous one is legato (same as live playing): the pitch glides
//    and the vowel changes without a gap, but consonants are NOT lost. The previous syllable's
//    final consonants and the new syllable's opening consonants are sung between the vowels.
//    Touching or separated notes re-articulate fully.
//  - Melisma: a "~" in the lyrics holds the previous vowel across another note. Melisma
//    notes join the previous note without a release when they are touching (or overlapping).
//  - "Vowel on the beat": a syllable's onset consonants start *before* the note so the vowel
//    lands on it, and its coda consonants finish *inside* the note. Without it, consonants
//    begin at the note start, like live playing.

import type { MidiNote } from '../midi/smf';
import { syllableTiming } from '../synth/phonemes';
import type { SeqCommand, SyllablePlan } from '../synth/types';
import type { LyricSyllable } from '../text/lyrics';

export interface MapOptions {
  /** Playback speed multiplier (1 = the file's tempo, 2 = twice as fast). */
  rate: number;
  vowelOnBeat: boolean;
  repeatLyrics: boolean;
  /** Must equal the voice's consonant-length setting (it determines pre-roll). */
  consonantScale: number;
}

export interface SungNote {
  /** Score seconds after applying `rate` (0 = the first note's file time 0). */
  start: number;
  end: number;
  note: number;
  velocity: number;
  /** Index into the lyric syllables. */
  syllable: number;
  /** Overlaps the previous note: glides, no new onset. */
  legato: boolean;
}

export interface Sequence {
  commands: SeqCommand[];
  /** Engine time at which score time 0 occurs (room for consonants before the first beat). */
  leadInSec: number;
  /** Engine time at which playback is over (last sound has faded). */
  lengthSec: number;
  notes: SungNote[];
  stats: { melodyNotes: number; sungNotes: number; unsungNotes: number; syllables: number; unusedSyllables: number };
}

const CHORD_WINDOW_SEC = 0.03;
/** A melisma note starting within this long after the previous note ended still joins it. */
const MELISMA_JOIN_SEC = 0.12;
const OVERLAP_EPS_SEC = 0.01;
const END_TAIL_SEC = 0.6;
const MIN_GAP_SEC = 0.001;

const noteToHz = (note: number): number => 440 * Math.pow(2, (note - 69) / 12);

/** Keep the highest note of every group of notes that start (nearly) together. */
export function reduceToMonophonic(notes: MidiNote[], windowSec = CHORD_WINDOW_SEC): MidiNote[] {
  const sorted = [...notes].sort((a, b) => a.start - b.start || b.note - a.note);
  const out: MidiNote[] = [];
  let i = 0;
  while (i < sorted.length) {
    let best = sorted[i];
    let j = i + 1;
    while (j < sorted.length && sorted[j].start - sorted[i].start <= windowSec) {
      if (sorted[j].note > best.note) best = sorted[j];
      j++;
    }
    out.push(best);
    i = j;
  }
  return out;
}

const planOf = (s: LyricSyllable): SyllablePlan => ({
  onset: s.onset,
  nucleus: s.nucleus,
  coda: s.coda,
  holdFirst: s.holdFirst,
});

export function buildSequence(melody: MidiNote[], syllables: LyricSyllable[], opts: MapOptions): Sequence {
  const rate = Math.max(0.1, opts.rate);
  const mono = reduceToMonophonic(melody);
  const t0 = mono.length ? mono[0].start : 0;

  // Assign syllables and detect legato overlaps (previous note is cut where the next begins).
  const sung: SungNote[] = [];
  for (let i = 0; i < mono.length; i++) {
    let syllable = i;
    if (syllables.length === 0) break;
    if (syllable >= syllables.length) {
      if (!opts.repeatLyrics) break;
      syllable = i % syllables.length;
    }
    const m = mono[i];
    const start = (m.start - t0) / rate;
    const end = Math.max(start + 0.02, (m.end - t0) / rate);
    const prev = sung[sung.length - 1];
    let legato = false;
    if (prev) {
      if (prev.end - start > OVERLAP_EPS_SEC) legato = true;
      else if (syllables[syllable].melisma && start - prev.end <= MELISMA_JOIN_SEC) legato = true;
      if (prev.end > start) prev.end = start;
    }
    sung.push({ start, end, note: m.note, velocity: m.velocity, syllable, legato });
  }

  const commands: SeqCommand[] = [];
  const timings = sung.map((s) =>
    opts.vowelOnBeat
      ? syllableTiming(planOf(syllables[s.syllable]), opts.consonantScale)
      : { onsetSec: 0, codaSec: 0, minVowelSec: 0 },
  );
  let lastT = -Infinity;
  for (let k = 0; k < sung.length; k++) {
    const s = sung[k];
    const tm = timings[k];
    const nextLegato = k + 1 < sung.length && sung[k + 1].legato;
    const nextFresh = k + 1 < sung.length && !sung[k + 1].legato;

    // Pre-roll: the onset consonants, plus (when legato) the previous syllable's final
    // consonants, which are sung now because there is no release between the two notes.
    const preRoll = tm.onsetSec + (s.legato && k > 0 ? timings[k - 1].codaSec : 0);
    let onT = s.start - preRoll;
    onT = Math.max(onT, lastT + MIN_GAP_SEC); // never earlier than the previous command
    commands.push({
      t: onT,
      kind: 'on',
      plan: planOf(syllables[s.syllable]),
      f0: noteToHz(s.note),
      velocity: s.velocity,
      legato: s.legato,
    });
    lastT = onT;

    if (nextLegato) continue; // the next note takes over; no release in between
    let offT = Math.max(s.end - tm.codaSec, s.start + tm.minVowelSec);
    if (nextFresh && opts.vowelOnBeat) {
      // Let the coda finish before the next onset begins.
      offT = Math.min(offT, sung[k + 1].start - timings[k + 1].onsetSec - tm.codaSec);
    }
    offT = Math.max(offT, onT + MIN_GAP_SEC * 2);
    commands.push({ t: offT, kind: 'off' });
    lastT = offT;
  }

  // Shift so the earliest command (a consonant before the first beat) is at engine time 0.
  const leadIn = commands.length ? Math.max(0, -commands[0].t) : 0;
  if (leadIn > 0) for (const c of commands) c.t += leadIn;
  const lastCmd = commands.length ? commands[commands.length - 1].t : 0;

  return {
    commands,
    leadInSec: leadIn,
    lengthSec: lastCmd + END_TAIL_SEC,
    notes: sung,
    stats: {
      melodyNotes: mono.length,
      sungNotes: sung.length,
      unsungNotes: mono.length - sung.length,
      syllables: syllables.length,
      unusedSyllables: Math.max(0, syllables.length - sung.length),
    },
  };
}
