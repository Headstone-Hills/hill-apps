import { describe, expect, it } from 'vitest';
import { parseMidiFile, singableTracks } from '../src/midi/smf';
import { noteOff, noteOn, smf, tempo, track, trackName } from './smf-builder';

describe('parseMidiFile', () => {
  it('converts ticks to seconds at the default 120 BPM', () => {
    const f = parseMidiFile(smf({ tracks: [track([noteOn(0, 60, 127), noteOff(480, 60)])] }));
    expect(f.tracks[0].notes).toEqual([{ start: 0, end: 0.5, note: 60, velocity: 1, channel: 0 }]);
    expect(f.durationSec).toBeCloseTo(0.5, 9);
    expect(f.initialBpm).toBe(120);
    expect(f.tempoEvents).toBe(0);
  });

  it('follows tempo changes (tempo track + note track, format 1)', () => {
    const f = parseMidiFile(
      smf({
        tracks: [
          track([tempo(0, 500000), tempo(480, 1000000)]), // 120 BPM then 60 BPM at beat 2
          track([noteOn(0, 60), noteOff(480, 60), noteOn(0, 62), noteOff(480, 62)]),
        ],
      }),
    );
    const [a, b] = f.tracks[1].notes;
    expect(a.start).toBeCloseTo(0, 9);
    expect(a.end).toBeCloseTo(0.5, 9);
    expect(b.start).toBeCloseTo(0.5, 9);
    expect(b.end).toBeCloseTo(1.5, 9); // slower: a quarter now lasts a full second
    expect(f.tempoEvents).toBe(2);
  });

  it('handles running status and note-on with velocity 0 as note-off', () => {
    const bytes = track([
      [0, 0x90, 60, 100],
      [240, 62, 100], // running status: note-on 62
      [240, 60, 0], // running status: note-on vel 0 = off for 60
      [240, 62, 0],
    ]);
    const f = parseMidiFile(smf({ tracks: [bytes] }));
    expect(f.tracks[0].notes.map((n) => [n.note, n.start, n.end])).toEqual([
      [60, 0, 0.5],
      [62, 0.25, 0.75],
    ]);
  });

  it('pairs repeated same-pitch notes first-in-first-out and closes unterminated notes at track end', () => {
    const f = parseMidiFile(
      smf({ tracks: [track([noteOn(0, 60), noteOn(240, 60), noteOff(240, 60), noteOff(240, 60), noteOn(0, 64), [480, 0xb0, 7, 100]])] }),
    );
    const n = f.tracks[0].notes;
    expect(n.map((x) => [x.note, x.start, x.end])).toEqual([
      [60, 0, 0.5],
      [60, 0.25, 0.75],
      [64, 0.75, 1.25], // never turned off: ends with the track
    ]);
  });

  it('reads SMPTE division', () => {
    // 25 fps (0xE7 = -25) x 40 ticks/frame = 1000 ticks per second
    const f = parseMidiFile(smf({ division: 0xe728, tracks: [track([noteOn(500, 60), noteOff(1000, 60)])] }));
    expect(f.tracks[0].notes[0].start).toBeCloseTo(0.5, 9);
    expect(f.tracks[0].notes[0].end).toBeCloseTo(1.5, 9);
  });

  it('skips sysex and unknown metas, reads names and channels', () => {
    const f = parseMidiFile(
      smf({
        tracks: [
          track([
            trackName('Lead Vox'),
            [0, 0xf0, 3, 0x7e, 0x7f, 0xf7],
            [0, 0xff, 0x7f, 2, 1, 2],
            [0, 0xc0, 5], // program change: 1 data byte
            noteOn(0, 72, 90, 2),
            noteOff(100, 72, 2),
          ]),
          track([noteOn(0, 36, 100, 9), noteOff(50, 36, 9)]),
        ],
      }),
    );
    expect(f.tracks[0].name).toBe('Lead Vox');
    expect(f.tracks[0].channels).toEqual([2]);
    expect(singableTracks(f).map((t) => t.index)).toEqual([0]); // channel-10-only track is percussion
  });

  it('keeps notes parsed before a truncated or damaged track ends', () => {
    const good = track([noteOn(0, 60), noteOff(480, 60), noteOn(0, 62)], false);
    const cut = good.slice(0, good.length - 1); // chop the last byte
    const f = parseMidiFile(Uint8Array.from([...smf({ tracks: [cut] })]));
    expect(f.tracks[0].notes[0]).toMatchObject({ note: 60, start: 0, end: 0.5 });
  });

  it('rejects things that are not usable MIDI files', () => {
    expect(() => parseMidiFile(new Uint8Array([1, 2, 3]))).toThrow(/Not a MIDI file/);
    expect(() => parseMidiFile(new TextEncoder().encode('RIFF....WAVEfmt this is not midi at all'))).toThrow(/Not a MIDI file/);
    expect(() => parseMidiFile(smf({ format: 2, tracks: [track([])] }))).toThrow(/format 2/);
    expect(() => parseMidiFile(smf({ tracks: [] }))).toThrow(/no tracks/);
  });

  it('survives random garbage after a valid header without hanging', () => {
    const header = smf({ tracks: [] });
    for (let seed = 1; seed <= 200; seed++) {
      let x = seed * 2654435761;
      const junk = Array.from({ length: 64 }, () => ((x = (x * 1103515245 + 12345) >>> 0) >> 8) & 255);
      try {
        parseMidiFile(Uint8Array.from([...header, 0x4d, 0x54, 0x72, 0x6b, 0, 0, 0, 64, ...junk]));
      } catch (e) {
        expect(e).toBeInstanceOf(Error); // an Error is fine; a hang or crash is not
      }
    }
  });
});
