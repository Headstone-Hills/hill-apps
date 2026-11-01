import { describe, expect, it } from 'vitest';
import { Performance, type VoiceSink } from '../src/engine/performance';
import { NavBindings, type NavAction } from '../src/midi/bindings';
import { parseMidiMessage, type MidiEvent } from '../src/midi/midi';
import { buildLyrics } from '../src/text/lyrics';
import { LyricPointer } from '../src/text/pointer';
import { realLookup } from './helpers';

function setup(text = 'hello big world') {
  const calls: any[] = [];
  const sink: VoiceSink = {
    noteOn: (plan, f0, velocity, legato) => calls.push({ t: 'on', nucleus: plan.nucleus, onset: plan.onset, f0, velocity, legato }),
    noteOff: () => calls.push({ t: 'off' }),
    setF0: (f0) => calls.push({ t: 'f0', f0 }),
  };
  const pointer = new LyricPointer();
  pointer.setLyrics(buildLyrics(text, realLookup()));
  const shown: number[] = [];
  const perf = new Performance(sink, pointer, { onSyllable: (i) => shown.push(i) });
  return { calls, pointer, perf, shown };
}
const hz = (note: number) => 440 * Math.pow(2, (note - 69) / 12);

describe('Performance (mono legato)', () => {
  it('consumes one syllable per fresh note and tunes to A4=440', () => {
    const { calls, perf, pointer, shown } = setup();
    perf.noteOn(69, 0.5);
    perf.noteOff(69);
    perf.noteOn(60, 1);
    expect(calls.map((c) => c.t)).toEqual(['on', 'off', 'on']);
    expect(calls[0].f0).toBeCloseTo(440, 6);
    expect(calls[0].legato).toBe(false);
    expect(calls[2].legato).toBe(false);
    expect(pointer.index).toBe(2);
    expect(shown).toEqual([0, 1]);
  });

  it('melisma glides further notes on the vowel being sung, without consuming syllables', () => {
    const { calls, perf, pointer, shown } = setup('hello big world');
    perf.noteOn(60, 1);
    perf.melisma = true;
    perf.noteOn(64, 1);
    perf.noteOn(67, 1);
    expect(calls.map((c) => c.t)).toEqual(['on', 'f0', 'f0']);
    expect(calls[2].f0).toBeCloseTo(hz(67), 6);
    expect(pointer.index).toBe(1);
    expect(shown).toEqual([0]);
    perf.melisma = false;
    perf.noteOn(69, 1); // back to normal: the next syllable, legato
    expect(calls[3]).toMatchObject({ t: 'on', legato: true });
    expect(pointer.index).toBe(2);
  });

  it('melisma with nothing sounding (the first key already came up) still sings the next syllable', () => {
    const { calls, perf, pointer } = setup('hello big world');
    perf.noteOn(60, 1);
    perf.noteOff(60);
    perf.melisma = true;
    perf.noteOn(64, 1);
    expect(calls.map((c) => c.t)).toEqual(['on', 'off', 'on']);
    expect(calls[2].legato).toBe(false);
    expect(calls[2].onset.length + (calls[2].nucleus ? 1 : 0)).toBeGreaterThan(0);
    expect(calls[2].nucleus).not.toBe(calls[0].nucleus); // the next syllable ("lo"), not the first one again
    expect(pointer.index).toBe(2);
  });

  it('reports when the voice starts and stops sounding, for the highlight that follows the played syllable', () => {
    const gates: boolean[] = [];
    const sink: VoiceSink = { noteOn: () => {}, noteOff: () => {}, setF0: () => {} };
    const pointer = new LyricPointer();
    pointer.setLyrics(buildLyrics('hello big world', realLookup()));
    const perf = new Performance(sink, pointer, { onGate: (open) => gates.push(open) });
    perf.noteOn(60, 1);
    perf.noteOn(64, 1); // legato: still one stretch of sound
    expect(gates).toEqual([true]);
    perf.noteOff(60);
    expect(gates).toEqual([true]); // another key is still down
    perf.noteOff(64);
    expect(gates).toEqual([true, false]);
  });

  it('melisma does nothing special at the very first note', () => {
    const { calls, perf, pointer } = setup('hello big world');
    perf.melisma = true;
    perf.noteOn(60, 1);
    expect(calls[0]).toMatchObject({ t: 'on', legato: false });
    expect(pointer.index).toBe(1);
  });

  it('hold keeps singing the current syllable without moving the pointer, and releasing it resumes', () => {
    const { calls, perf, pointer, shown } = setup('hello big world');
    perf.noteOn(60, 1);
    perf.noteOff(60);
    expect(pointer.index).toBe(1); // "hel" sung, "lo" is next
    perf.hold = true;
    const next = pointer.peek()!.nucleus;
    for (const n of [62, 64, 65]) {
      perf.noteOn(n, 1);
      perf.noteOff(n);
    }
    expect(pointer.index).toBe(1); // did not move
    expect(shown).toEqual([0, 1, 1, 1]); // the same syllable each time
    expect(calls.filter((c) => c.t === 'on').map((c) => c.nucleus).slice(1)).toEqual([next, next, next]);
    perf.hold = false;
    perf.noteOn(67, 1);
    expect(pointer.index).toBe(2); // moves on again
  });

  it('overlapping notes are legato and still consume a syllable', () => {
    const { calls, perf, pointer } = setup();
    perf.noteOn(60, 0.8);
    perf.noteOn(64, 0.8);
    expect(calls[1]).toMatchObject({ t: 'on', legato: true });
    expect(pointer.index).toBe(2);
  });

  it('releasing the top key glides back to the held key without consuming a syllable', () => {
    const { calls, perf, pointer } = setup();
    perf.noteOn(60, 0.8);
    perf.noteOn(64, 0.8);
    perf.noteOff(64);
    expect(calls[2]).toEqual({ t: 'f0', f0: hz(60) });
    expect(pointer.index).toBe(2);
    perf.noteOff(60);
    expect(calls[3]).toEqual({ t: 'off' });
  });

  it('releasing a non-top key changes nothing audible', () => {
    const { calls, perf } = setup();
    perf.noteOn(60, 0.8);
    perf.noteOn(64, 0.8);
    perf.noteOff(60);
    expect(calls).toHaveLength(2);
    perf.noteOff(64);
    expect(calls[2]).toEqual({ t: 'off' });
  });

  it('sustain pedal keeps the voice open until pedal-up, and next note is legato', () => {
    const { calls, perf } = setup();
    perf.sustain(true);
    perf.noteOn(60, 0.8);
    perf.noteOff(60);
    expect(calls.map((c) => c.t)).toEqual(['on']);
    perf.noteOn(62, 0.8);
    expect(calls[1]).toMatchObject({ t: 'on', legato: true });
    perf.noteOff(62);
    perf.sustain(false);
    expect(calls[2]).toEqual({ t: 'off' });
  });

  it('is silent at end of text for fresh notes, but follows pitch when legato', () => {
    const { calls, perf, pointer } = setup('hi');
    pointer.setIndex(pointer.length);
    perf.noteOn(60, 0.8);
    expect(calls).toHaveLength(0);
    perf.noteOff(60);
    expect(calls).toHaveLength(0);

    pointer.setIndex(pointer.length - 1);
    perf.noteOn(60, 0.8); // last syllable
    perf.noteOn(67, 0.8); // legato at end of text: only a pitch change
    expect(calls.map((c) => c.t)).toEqual(['on', 'f0']);
    expect(calls[1].f0).toBeCloseTo(hz(67), 6);
  });

  it('re-pressing a held note does not duplicate it in the stack', () => {
    const { calls, perf } = setup();
    perf.noteOn(60, 0.8);
    perf.noteOn(60, 0.8);
    perf.noteOff(60);
    expect(calls.filter((c) => c.t === 'off')).toHaveLength(1);
  });

  it('pitch bend retunes the sounding note by +/- bendRange semitones', () => {
    const { calls, perf } = setup();
    perf.noteOn(60, 0.8);
    perf.pitchBend(1);
    expect(calls[1].f0).toBeCloseTo(hz(62), 6);
    perf.pitchBend(-1);
    expect(calls[2].f0).toBeCloseTo(hz(58), 6);
    perf.noteOn(64, 0.8); // new note keeps the current bend
    expect(calls[3].f0).toBeCloseTo(hz(62), 6);
  });

  it('pitch bend while silent does nothing; allNotesOff closes the gate', () => {
    const { calls, perf } = setup();
    perf.pitchBend(0.5);
    expect(calls).toHaveLength(0);
    perf.noteOn(60, 0.8);
    perf.allNotesOff();
    expect(calls.at(-1)).toEqual({ t: 'off' });
    expect(perf.isGated).toBe(false);
  });
});

describe('parseMidiMessage', () => {
  it('parses notes on any channel, with velocity-0 note-on as note-off', () => {
    expect(parseMidiMessage([0x93, 60, 127])).toEqual({ type: 'noteOn', note: 60, velocity: 1 });
    expect(parseMidiMessage([0x90, 60, 0])).toEqual({ type: 'noteOff', note: 60 });
    expect(parseMidiMessage([0x80, 61, 40])).toEqual({ type: 'noteOff', note: 61 });
  });
  it('parses CC, pitch bend and all-notes-off; ignores the rest', () => {
    expect(parseMidiMessage([0xb0, 1, 64])).toEqual({ type: 'cc', controller: 1, value: 64 });
    expect(parseMidiMessage([0xb0, 123, 0])).toEqual({ type: 'allNotesOff' });
    expect(parseMidiMessage([0xe0, 0, 64])).toEqual({ type: 'pitchBend', value: 0 });
    expect((parseMidiMessage([0xe0, 127, 127]) as any).value).toBeCloseTo(1, 3);
    expect((parseMidiMessage([0xe0, 0, 0]) as any).value).toBe(-1);
    expect(parseMidiMessage([0xf8])).toBeNull();
    expect(parseMidiMessage([0xc0, 5])).toBeNull();
  });
});

describe('NavBindings', () => {
  const memory = () => {
    const m = new Map<string, string>();
    return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
  };
  const on = (note: number): MidiEvent => ({ type: 'noteOn', note, velocity: 1 });
  const off = (note: number): MidiEvent => ({ type: 'noteOff', note });

  it('learns a note, then fires and swallows both its note-on and note-off', () => {
    const b = new NavBindings(memory());
    const fired: NavAction[] = [];
    b.learn('forward');
    expect(b.handle(on(36), (a) => fired.push(a))).toBe(true); // learned
    expect(fired).toEqual([]);
    expect(b.handle(off(36), (a) => fired.push(a))).toBe(true);
    expect(b.handle(on(36), (a) => fired.push(a))).toBe(true);
    expect(b.handle(off(36), (a) => fired.push(a))).toBe(true);
    expect(fired).toEqual(['forward']);
    expect(b.handle(on(60), (a) => fired.push(a))).toBe(false); // unrelated notes play
    expect(b.handle(off(60), (a) => fired.push(a))).toBe(false);
  });

  it('CC buttons fire once per press', () => {
    const b = new NavBindings(memory());
    const fired: NavAction[] = [];
    b.learn('reset');
    b.handle({ type: 'cc', controller: 20, value: 127 }, (a) => fired.push(a));
    b.handle({ type: 'cc', controller: 20, value: 127 }, (a) => fired.push(a)); // repeat while held
    expect(fired).toEqual([]);
    b.handle({ type: 'cc', controller: 20, value: 0 }, (a) => fired.push(a));
    b.handle({ type: 'cc', controller: 20, value: 127 }, (a) => fired.push(a));
    b.handle({ type: 'cc', controller: 20, value: 127 }, (a) => fired.push(a));
    expect(fired).toEqual(['reset']);
  });

  it('never captures mod wheel / sustain presses while learning, and never swallows releases', () => {
    const b = new NavBindings(memory());
    b.learn('back');
    expect(b.handle({ type: 'cc', controller: 64, value: 127 }, () => {})).toBe(false);
    expect(b.handle({ type: 'cc', controller: 64, value: 0 }, () => {})).toBe(false);
    expect(b.handle({ type: 'cc', controller: 1, value: 100 }, () => {})).toBe(false);
    expect(b.armedAction).toBe('back');
  });

  it('rebinding a note moves it from the old action, and bindings persist', () => {
    const store = memory();
    const b = new NavBindings(store);
    b.learn('back'); b.handle(on(30), () => {});
    b.learn('forward'); b.handle(on(30), () => {});
    expect(b.bindingFor('back')).toBeUndefined();
    expect(b.bindingFor('forward')).toEqual({ kind: 'note', number: 30 });
    const again = new NavBindings(store);
    expect(again.bindingFor('forward')).toEqual({ kind: 'note', number: 30 });
    again.clear('forward');
    expect(new NavBindings(store).bindingFor('forward')).toBeUndefined();
  });

  it('survives corrupt storage', () => {
    const b = new NavBindings({ getItem: () => '{not json', setItem: () => { throw new Error('quota'); } });
    b.learn('back');
    expect(() => b.handle(on(40), () => {})).not.toThrow();
  });
});
