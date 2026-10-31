import { describe, expect, it } from 'vitest';
import { buildSequence, reduceToMonophonic, type MapOptions } from '../src/engine/sequence';
import type { MidiNote } from '../src/midi/smf';
import { KlattVoice } from '../src/synth/klatt';
import { syllableTiming } from '../src/synth/phonemes';
import { SequenceRunner } from '../src/synth/sequence';
import { buildLyrics } from '../src/text/lyrics';
import { FS, realLookup, rms } from './helpers';

const note = (start: number, end: number, n = 60, velocity = 0.8): MidiNote => ({ start, end, note: n, velocity, channel: 0 });
const syllables = (text: string) => buildLyrics(text, realLookup()).syllables;
const OPTS: MapOptions = { rate: 1, vowelOnBeat: true, repeatLyrics: false, consonantScale: 0.7 };

describe('reduceToMonophonic', () => {
  it('keeps the highest note of a chord and leaves a melody alone', () => {
    const chord = [note(0, 1, 60), note(0.005, 1, 64), note(0.01, 1, 67), note(1, 2, 62)];
    expect(reduceToMonophonic(chord).map((n) => n.note)).toEqual([67, 62]);
    expect(reduceToMonophonic([note(0, 1, 60), note(1, 2, 62), note(2, 3, 64)]).map((n) => n.note)).toEqual([60, 62, 64]);
    expect(reduceToMonophonic([])).toEqual([]);
  });
});

describe('buildSequence: mapping', () => {
  it('assigns one syllable per note in order; extra notes are silent', () => {
    const syl = syllables('hello'); // hel-lo
    const seq = buildSequence([note(0, 0.4, 60), note(0.5, 0.9, 62), note(1, 1.4, 64)], syl, OPTS);
    expect(seq.notes.map((n) => n.syllable)).toEqual([0, 1]);
    expect(seq.stats).toMatchObject({ melodyNotes: 3, sungNotes: 2, unsungNotes: 1, unusedSyllables: 0 });
  });

  it('repeats the lyrics when asked', () => {
    const seq = buildSequence([0, 1, 2, 3, 4].map((i) => note(i, i + 0.5)), syllables('hello'), { ...OPTS, repeatLyrics: true });
    expect(seq.notes.map((n) => n.syllable)).toEqual([0, 1, 0, 1, 0]);
  });

  it('reports unused syllables and handles empty input', () => {
    expect(buildSequence([note(0, 1)], syllables('hello world'), OPTS).stats.unusedSyllables).toBe(2);
    expect(buildSequence([], syllables('hello'), OPTS).commands).toEqual([]);
    expect(buildSequence([note(0, 1)], [], OPTS).commands).toEqual([]);
  });

  it('tunes notes (MIDI 69 = 440 Hz) and carries velocity', () => {
    const seq = buildSequence([note(0, 1, 69, 0.5)], syllables('hi'), OPTS);
    const on = seq.commands.find((c) => c.kind === 'on');
    expect(on).toMatchObject({ f0: 440, velocity: 0.5, legato: false });
  });

  it('marks overlapping notes legato, cuts the previous note, and sends no release between them', () => {
    const seq = buildSequence([note(0, 1.0, 60), note(0.8, 1.6, 64), note(2, 2.5, 62)], syllables('one two three'), OPTS);
    expect(seq.notes.map((n) => n.legato)).toEqual([false, true, false]);
    expect(seq.notes[0].end).toBeCloseTo(0.8, 9);
    const kinds = seq.commands.map((c) => (c.kind === 'on' ? (c.legato ? 'L' : 'N') : 'x'));
    expect(kinds).toEqual(['N', 'L', 'x', 'N', 'x']);
  });

  it('touching notes are re-articulated (not legato)', () => {
    const seq = buildSequence([note(0, 1, 60), note(1, 2, 62)], syllables('one two'), OPTS);
    expect(seq.notes.map((n) => n.legato)).toEqual([false, false]);
  });

  it('scales time by the playback rate', () => {
    const slow = buildSequence([note(0, 1), note(2, 3)], syllables('one two'), { ...OPTS, rate: 1 });
    const fast = buildSequence([note(0, 1), note(2, 3)], syllables('one two'), { ...OPTS, rate: 2 });
    expect(fast.notes[1].start).toBeCloseTo(slow.notes[1].start / 2, 9);
  });
});

describe('buildSequence: timing', () => {
  it('starts onsets early and ends codas inside the note when "vowel on the beat"', () => {
    const syl = syllables('stand'); // S T AE1 N D: onset + coda
    const tm = syllableTiming({ onset: syl[0].onset, nucleus: syl[0].nucleus, coda: syl[0].coda }, 0.7);
    expect(tm.onsetSec).toBeGreaterThan(0.05);
    expect(tm.codaSec).toBeGreaterThan(0.05);
    const seq = buildSequence([note(1, 2)], syl, OPTS);
    const [on, off] = seq.commands;
    // Engine time = score time + lead-in; score time 0 is the first note's start.
    expect(seq.leadInSec).toBeCloseTo(tm.onsetSec, 9);
    expect(on.t).toBeCloseTo(0, 9);
    expect(off.t - seq.leadInSec).toBeCloseTo(1 - tm.codaSec, 9); // note is 1 s long, starting at score time 0
    expect(seq.lengthSec).toBeGreaterThan(off.t);
  });

  it('keeps the plain timing (no pre-roll, release at note end) when switched off', () => {
    const seq = buildSequence([note(1, 2)], syllables('stand'), { ...OPTS, vowelOnBeat: false });
    expect(seq.leadInSec).toBe(0);
    expect(seq.commands.map((c) => +c.t.toFixed(6))).toEqual([0, 1]);
  });

  it('lets a coda finish before the next onset starts, and keeps commands in order', () => {
    const syl = syllables('stand strong'); // two consonant-heavy syllables
    const seq = buildSequence([note(0, 0.5), note(0.5, 1.0)], syl, OPTS);
    const ts = seq.commands.map((c) => c.t);
    expect([...ts].sort((a, b) => a - b)).toEqual(ts);
    const tm0 = syllableTiming({ onset: syl[0].onset, nucleus: syl[0].nucleus, coda: syl[0].coda }, 0.7);
    const tm1 = syllableTiming({ onset: syl[1].onset, nucleus: syl[1].nucleus, coda: syl[1].coda }, 0.7);
    const off0 = seq.commands[1].t;
    const on1 = seq.commands[2].t;
    expect(seq.commands[1].kind).toBe('off');
    expect(on1).toBeGreaterThanOrEqual(off0);
    // Normal case: coda fits between the release and the next onset.
    expect(on1 - off0).toBeGreaterThanOrEqual(tm0.codaSec - 1e-6);
    expect(on1).toBeCloseTo(seq.leadInSec + 0.5 - tm1.onsetSec, 6);
  });

  it('never produces negative or out-of-order times for very short, dense notes', () => {
    const notes = Array.from({ length: 40 }, (_, i) => note(i * 0.06, i * 0.06 + 0.05, 60 + (i % 7)));
    const seq = buildSequence(notes, syllables('the quick brown fox jumps over the lazy dog and keeps on running far away'), OPTS);
    const ts = seq.commands.map((c) => c.t);
    expect(ts.every((t) => t >= 0)).toBe(true);
    expect([...ts].sort((a, b) => a - b)).toEqual(ts);
    // on/off alternate properly (legato 'on's may follow an 'on')
    expect(seq.commands.filter((c) => c.kind === 'off').length).toBeLessThanOrEqual(seq.commands.filter((c) => c.kind === 'on').length);
  });
});

describe('buildSequence: legato keeps consonants', () => {
  it('pre-rolls the previous syllable\'s final consonants and the new onset for a legato note', () => {
    const syl = syllables('stand strong');
    const tm0 = syllableTiming({ onset: syl[0].onset, nucleus: syl[0].nucleus, coda: syl[0].coda }, 0.7);
    const tm1 = syllableTiming({ onset: syl[1].onset, nucleus: syl[1].nucleus, coda: syl[1].coda }, 0.7);
    expect(tm0.codaSec).toBeGreaterThan(0.05);
    expect(tm1.onsetSec).toBeGreaterThan(0.05);
    const seq = buildSequence([note(0, 1.0), note(0.9, 1.8)], syl, OPTS); // overlap => legato
    expect(seq.notes[1].legato).toBe(true);
    const second = seq.commands.find((c, i) => c.kind === 'on' && i > 0)!;
    expect(second.kind === 'on' && second.legato).toBe(true);
    // Score time of the 2nd note is 0.9; consonants start this long before it.
    expect(second.t - seq.leadInSec).toBeCloseTo(0.9 - (tm0.codaSec + tm1.onsetSec), 6);
  });

  it('plain timing (vowel-on-beat off) starts legato notes at the note start', () => {
    const seq = buildSequence([note(0, 1.0), note(0.9, 1.8)], syllables('stand strong'), { ...OPTS, vowelOnBeat: false });
    expect(seq.commands.map((c) => +c.t.toFixed(6))).toEqual([0, 0.9, 1.8]);
  });
});

describe('buildSequence: melisma', () => {
  it('melisma notes join the previous note with no release when touching', () => {
    const syl = syllables('hold ~ ~');
    const seq = buildSequence([note(0, 0.5), note(0.5, 1.0, 64), note(1.0, 1.5, 62)], syl, OPTS);
    expect(seq.notes.map((n) => n.legato)).toEqual([false, true, true]);
    const kinds = seq.commands.map((c) => (c.kind === 'on' ? (c.legato ? 'L' : 'N') : 'x'));
    expect(kinds).toEqual(['N', 'L', 'L', 'x']); // one release, after the last note
    // The final consonants (L D) belong to the last note only.
    const plans = seq.commands.filter((c) => c.kind === 'on').map((c) => (c.kind === 'on' ? c.plan.coda : []));
    expect(plans).toEqual([[], [], ['L', 'D']]);
  });

  it('joins across a short gap but re-attacks after a real rest', () => {
    const syl = syllables('hold ~');
    const short = buildSequence([note(0, 0.5), note(0.55, 1.0)], syl, OPTS);
    expect(short.notes[1].legato).toBe(true);
    const rest = buildSequence([note(0, 0.5), note(1.0, 1.5)], syl, OPTS);
    expect(rest.notes[1].legato).toBe(false);
  });

  it('ordinary (non-melisma) touching notes are still re-articulated', () => {
    const seq = buildSequence([note(0, 0.5), note(0.5, 1.0)], syllables('one two'), OPTS);
    expect(seq.notes[1].legato).toBe(false);
  });

  it('passes the hold-first-vowel flag to the synth', () => {
    const seq = buildSequence([note(0, 0.5), note(0.5, 1.0)], syllables('fly ~'), OPTS);
    const ons = seq.commands.filter((c) => c.kind === 'on');
    expect(ons.map((c) => (c.kind === 'on' ? c.plan.holdFirst ?? false : null))).toEqual([true, false]);
  });
});

// ---- Offline rendering through the real runner + voice ----
function renderSequence(seq: ReturnType<typeof buildSequence>, chunk = 128, setup?: (r: SequenceRunner, v: KlattVoice) => void) {
  const voice = new KlattVoice(FS);
  const runner = new SequenceRunner(voice, FS);
  runner.load(seq.commands, seq.lengthSec, 0);
  setup?.(runner, voice);
  runner.play();
  const total = Math.ceil(seq.lengthSec * FS) + 2000;
  const out = new Float32Array(total);
  let ended = false;
  runner.onEnded = () => (ended = true);
  for (let i = 0; i < total; i += chunk) runner.render(out.subarray(i, Math.min(total, i + chunk)));
  return { out, runner, ended };
}
const firstAbove = (a: Float32Array, from: number, level: number) => {
  for (let t = from; t < a.length / FS - 0.01; t += 0.002) if (rms(a, t, t + 0.01) > level) return t;
  return Infinity;
};

describe('SequenceRunner (offline render)', () => {
  const melody = [note(0.5, 1.0, 60), note(1.5, 2.0, 64)];
  const syl = syllables('stand tea'); // consonant onset on both syllables

  it('lands vowels on the beat with "vowel on the beat", but late without it', () => {
    const on = buildSequence(melody, syl, OPTS);
    const off = buildSequence(melody, syl, { ...OPTS, vowelOnBeat: false });
    const a = renderSequence(on).out;
    const b = renderSequence(off).out;
    const beat = (seq: typeof on, i: number) => seq.leadInSec + seq.notes[i].start;
    const lateOn = firstAbove(a, beat(on, 0) - 0.3, 0.1) - beat(on, 0);
    const lateOff = firstAbove(b, beat(off, 0) - 0.3, 0.1) - beat(off, 0);
    expect(Math.abs(lateOn)).toBeLessThan(0.06);
    expect(lateOff).toBeGreaterThan(Math.abs(lateOn) + 0.04);
  });

  it('is audible during notes, silent in gaps, and finishes', () => {
    const seq = buildSequence(melody, syl, OPTS);
    const { out, ended, runner } = renderSequence(seq);
    // Score time 0 is the first note's start: notes span 0-0.5 s and 1.0-1.5 s.
    const mid1 = seq.leadInSec + 0.25;
    const mid2 = seq.leadInSec + 1.25;
    expect(rms(out, mid1, mid1 + 0.05)).toBeGreaterThan(0.08);
    expect(rms(out, mid2, mid2 + 0.05)).toBeGreaterThan(0.08);
    expect(rms(out, seq.leadInSec + 0.75, seq.leadInSec + 0.8)).toBeLessThan(0.01); // gap between notes
    expect(ended).toBe(true);
    expect(runner.isPlaying).toBe(false);
  });

  it('gives the same timing no matter how the audio callback is chunked', () => {
    const seq = buildSequence(melody, syl, OPTS);
    const a = firstAbove(renderSequence(seq, 128).out, 0, 0.1);
    for (const chunk of [32, 37, 100, 441]) {
      const b = firstAbove(renderSequence(seq, chunk).out, 0, 0.1);
      expect(Math.abs(a - b), `chunk ${chunk}`).toBeLessThan(0.006);
    }
  });

  it('pause silences the voice and play resumes from the same position', () => {
    const seq = buildSequence(melody, syl, OPTS);
    const voice = new KlattVoice(FS);
    const runner = new SequenceRunner(voice, FS);
    runner.load(seq.commands, seq.lengthSec, 0);
    runner.play();
    const buf = new Float32Array(128);
    const t0 = seq.leadInSec + 0.7;
    while (runner.positionSec < t0) runner.render(buf);
    runner.pause();
    const pausedAt = runner.positionSec;
    let tailEnergy = 0;
    for (let i = 0; i < 400; i++) { runner.render(buf); if (i > 300) tailEnergy += buf.reduce((s, x) => s + x * x, 0); }
    expect(tailEnergy).toBeLessThan(1e-6);
    expect(runner.positionSec).toBe(pausedAt);
    runner.play();
    runner.render(buf);
    expect(runner.positionSec).toBeGreaterThan(pausedAt);
  });

  it('seek skips earlier commands; stop rewinds; loop wraps around', () => {
    const seq = buildSequence(melody, syl, OPTS);
    const voice = new KlattVoice(FS);
    const runner = new SequenceRunner(voice, FS);
    runner.load(seq.commands, seq.lengthSec, 0);
    runner.seek(seq.leadInSec + 1.4);
    expect(runner.positionSec).toBeCloseTo(seq.leadInSec + 1.4, 3);
    runner.stop();
    expect(runner.positionSec).toBe(0);

    runner.loop = true;
    runner.play();
    const buf = new Float32Array(128);
    let wrapped = false;
    let last = 0;
    for (let i = 0; i < Math.ceil((seq.lengthSec * FS) / 128) + 200; i++) {
      runner.render(buf);
      if (runner.positionSec < last) wrapped = true;
      last = runner.positionSec;
    }
    expect(wrapped).toBe(true);
    expect(runner.isPlaying).toBe(true);
  });

  it('live voice output still works when no sequence is playing', () => {
    const voice = new KlattVoice(FS);
    const runner = new SequenceRunner(voice, FS);
    voice.noteOn({ onset: [], nucleus: 'AA1', coda: [] }, 130, 0.8, false);
    const buf = new Float32Array(2048);
    runner.render(buf);
    expect(rms(buf, 0.02, 0.04)).toBeGreaterThan(0.05);
  });
});
