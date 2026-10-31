// Standard MIDI File (SMF) reader: formats 0 and 1, tempo map, running status, SMPTE
// division. Only what singing needs is extracted: notes (in seconds), track names and
// channels. Spec: MIDI 1.0 Standard MIDI Files, https://www.midi.org/specifications

export interface MidiNote {
  /** Seconds from the start of the file (at the file's own tempo). */
  start: number;
  end: number;
  note: number;
  /** 0..1 */
  velocity: number;
  /** 0-based MIDI channel. */
  channel: number;
}

export interface MidiTrack {
  index: number;
  name: string;
  /** Channels that carry notes in this track. Channel index 9 (MIDI channel 10) is percussion. */
  channels: number[];
  notes: MidiNote[];
}

export interface MidiFile {
  format: 0 | 1;
  tracks: MidiTrack[];
  durationSec: number;
  /** Number of tempo events (1 = constant tempo). */
  tempoEvents: number;
  /** First tempo in BPM (120 if the file has none). */
  initialBpm: number;
}

interface RawNote {
  startTick: number;
  endTick: number;
  note: number;
  velocity: number;
  channel: number;
}

class Reader {
  pos = 0;
  constructor(
    readonly d: Uint8Array,
    public end = d.length,
  ) {}
  get eof(): boolean {
    return this.pos >= this.end;
  }
  u8(): number {
    if (this.pos >= this.end) throw new Error('Unexpected end of MIDI data');
    return this.d[this.pos++];
  }
  u16(): number {
    return (this.u8() << 8) | this.u8();
  }
  u32(): number {
    return ((this.u8() << 24) | (this.u8() << 16) | (this.u8() << 8) | this.u8()) >>> 0;
  }
  vlq(): number {
    let v = 0;
    for (let i = 0; i < 4; i++) {
      const b = this.u8();
      v = (v << 7) | (b & 0x7f);
      if (!(b & 0x80)) return v;
    }
    throw new Error('Invalid variable-length quantity');
  }
  skip(n: number): void {
    if (this.pos + n > this.end) throw new Error('Unexpected end of MIDI data');
    this.pos += n;
  }
  text(n: number): string {
    const s = String.fromCharCode(...this.d.subarray(this.pos, Math.min(this.end, this.pos + n)));
    this.skip(Math.min(n, this.end - this.pos));
    return s;
  }
}

const tag = (r: Reader): string => String.fromCharCode(r.u8(), r.u8(), r.u8(), r.u8());

interface TempoPoint {
  tick: number;
  sec: number;
  usPerQuarter: number;
}

export function parseMidiFile(data: Uint8Array): MidiFile {
  const r = new Reader(data);
  if (data.length < 14 || tag(r) !== 'MThd') throw new Error('Not a MIDI file (missing MThd header)');
  const headerLen = r.u32();
  if (headerLen < 6) throw new Error('Corrupt MIDI header');
  const format = r.u16();
  const trackCount = r.u16();
  const division = r.u16();
  r.skip(headerLen - 6);
  if (format > 1) throw new Error('MIDI format 2 (independent sequences) is not supported');

  // Division: ticks per quarter note, or (negative) SMPTE frames x ticks per frame.
  const smpte = (division & 0x8000) !== 0;
  let ticksPerQuarter = division;
  let smpteTicksPerSec = 0;
  if (smpte) {
    const fps = 256 - (division >> 8); // two's complement of the high byte
    const tpf = division & 0xff;
    smpteTicksPerSec = fps * tpf;
    if (smpteTicksPerSec <= 0) throw new Error('Invalid SMPTE division');
  } else if (ticksPerQuarter <= 0) {
    throw new Error('Invalid ticks-per-quarter division');
  }

  const trackData: Array<{ name: string; notes: RawNote[]; lastTick: number }> = [];
  const tempos: Array<{ tick: number; usPerQuarter: number }> = [];

  while (!r.eof && trackData.length < trackCount) {
    if (r.end - r.pos < 8) break;
    const id = tag(r);
    const len = r.u32();
    const chunkEnd = Math.min(r.end, r.pos + len); // tolerate a truncated last chunk
    if (id !== 'MTrk') {
      r.pos = chunkEnd;
      continue;
    }
    const tr = new Reader(data, chunkEnd);
    tr.pos = r.pos;
    r.pos = chunkEnd;

    const notes: RawNote[] = [];
    const open = new Map<number, Array<{ tick: number; velocity: number }>>(); // channel*128+note
    let name = '';
    let tick = 0;
    let status = 0;
    try {
      while (!tr.eof) {
        tick += tr.vlq();
        let b = tr.u8();
        if (b < 0x80) {
          if (!status) throw new Error('Running status with no previous status');
          tr.pos -= 1; // data byte belongs to the previous status
          b = status;
        } else if (b < 0xf0) {
          status = b;
        }
        if (b === 0xff) {
          const type = tr.u8();
          const mlen = tr.vlq();
          if (type === 0x2f) break; // end of track
          if (type === 0x51 && mlen === 3) {
            tempos.push({ tick, usPerQuarter: (tr.u8() << 16) | (tr.u8() << 8) | tr.u8() });
          } else if (type === 0x03 && !name) {
            name = tr.text(mlen);
          } else {
            tr.skip(mlen);
          }
        } else if (b === 0xf0 || b === 0xf7) {
          tr.skip(tr.vlq());
        } else {
          const kind = b & 0xf0;
          const channel = b & 0x0f;
          const d1 = tr.u8();
          const d2 = kind === 0xc0 || kind === 0xd0 ? 0 : tr.u8();
          if (kind === 0x90 && d2 > 0) {
            const key = channel * 128 + d1;
            const stack = open.get(key) ?? [];
            stack.push({ tick, velocity: d2 / 127 });
            open.set(key, stack);
          } else if (kind === 0x80 || (kind === 0x90 && d2 === 0)) {
            const stack = open.get(channel * 128 + d1);
            const on = stack?.shift(); // first-in-first-out for repeated same-pitch notes
            if (on) notes.push({ startTick: on.tick, endTick: tick, note: d1, velocity: on.velocity, channel });
          }
        }
      }
    } catch (e) {
      // A damaged track keeps whatever was parsed so far (close open notes below).
      if (!(e instanceof Error) || !/end of MIDI data|variable-length|Running status/.test(e.message)) throw e;
    }
    // Notes never turned off end with the track.
    for (const [key, stack] of open) {
      for (const on of stack) {
        notes.push({ startTick: on.tick, endTick: Math.max(tick, on.tick), note: key % 128, velocity: on.velocity, channel: Math.floor(key / 128) });
      }
    }
    trackData.push({ name, notes, lastTick: tick });
  }

  if (trackData.length === 0) throw new Error('MIDI file contains no tracks');

  // Tempo map (ticks -> seconds). Default 120 BPM until the first tempo event.
  const points: TempoPoint[] = [{ tick: 0, sec: 0, usPerQuarter: 500000 }];
  if (!smpte) {
    tempos.sort((a, b) => a.tick - b.tick);
    for (const t of tempos) {
      const last = points[points.length - 1];
      const sec = last.sec + ((t.tick - last.tick) * last.usPerQuarter) / 1e6 / ticksPerQuarter;
      if (t.tick === last.tick) last.usPerQuarter = t.usPerQuarter;
      else points.push({ tick: t.tick, sec, usPerQuarter: t.usPerQuarter });
    }
  }
  const toSec = (tick: number): number => {
    if (smpte) return tick / smpteTicksPerSec;
    let lo = 0;
    let hi = points.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (points[mid].tick <= tick) lo = mid;
      else hi = mid - 1;
    }
    const p = points[lo];
    return p.sec + ((tick - p.tick) * p.usPerQuarter) / 1e6 / ticksPerQuarter;
  };

  let duration = 0;
  const tracks: MidiTrack[] = trackData.map((t, index) => {
    const notes: MidiNote[] = t.notes
      .map((n) => ({
        start: toSec(n.startTick),
        end: toSec(n.endTick),
        note: n.note,
        velocity: n.velocity,
        channel: n.channel,
      }))
      .sort((a, b) => a.start - b.start || a.note - b.note);
    for (const n of notes) duration = Math.max(duration, n.end);
    return { index, name: t.name, channels: [...new Set(notes.map((n) => n.channel))].sort((a, b) => a - b), notes };
  });

  return {
    format: format as 0 | 1,
    tracks,
    durationSec: duration,
    tempoEvents: tempos.length,
    initialBpm: tempos.length && tempos[0].tick === 0 ? 6e7 / tempos[0].usPerQuarter : 120,
  };
}

/** Tracks that look like a melody to sing: have notes and are not purely percussion. */
export function singableTracks(file: MidiFile): MidiTrack[] {
  return file.tracks.filter((t) => t.notes.length > 0 && !t.channels.every((c) => c === 9));
}
