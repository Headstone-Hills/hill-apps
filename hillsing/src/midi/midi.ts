export type MidiEvent =
  | { type: 'noteOn'; note: number; velocity: number } // velocity 0..1
  | { type: 'noteOff'; note: number }
  | { type: 'cc'; controller: number; value: number } // value 0..127
  | { type: 'pitchBend'; value: number } // -1..1
  | { type: 'allNotesOff' };

/** Parse one raw MIDI message (any channel). Returns null for messages we ignore. */
export function parseMidiMessage(data: ArrayLike<number>): MidiEvent | null {
  if (data.length < 2) return null;
  const status = data[0] & 0xf0;
  const d1 = data[1] & 0x7f;
  const d2 = (data[2] ?? 0) & 0x7f;
  switch (status) {
    case 0x90:
      return d2 === 0
        ? { type: 'noteOff', note: d1 }
        : { type: 'noteOn', note: d1, velocity: d2 / 127 };
    case 0x80:
      return { type: 'noteOff', note: d1 };
    case 0xb0:
      if (d1 === 123 || d1 === 120) return { type: 'allNotesOff' };
      return { type: 'cc', controller: d1, value: d2 };
    case 0xe0: {
      const raw = (d2 << 7) | d1; // 0..16383, centre 8192
      return { type: 'pitchBend', value: (raw - 8192) / 8192 };
    }
    case 0xd0:
      // Channel pressure (aftertouch): treated as controller 128, so it can be learned as a pitch or mod control.
      return { type: 'cc', controller: AFTERTOUCH_CONTROLLER, value: d1 };
    default:
      return null;
  }
}

/** Channel pressure is reported as this pseudo-controller number (real controllers are 0..127). */
export const AFTERTOUCH_CONTROLLER = 128;

const STATUS_NAMES: Record<number, string> = {
  0x80: 'note off',
  0x90: 'note on',
  0xa0: 'key pressure',
  0xb0: 'controller',
  0xc0: 'program change',
  0xd0: 'channel pressure',
  0xe0: 'pitch bend',
};

/** Any raw message as hex plus what kind it is, for the monitor (including kinds the app does not use). */
export function describeRaw(data: ArrayLike<number>): string {
  const hex = Array.from({ length: data.length }, (_, i) => data[i].toString(16).toUpperCase().padStart(2, '0')).join(' ');
  const status = data[0] & 0xf0;
  const kind = data[0] >= 0xf0 ? (data[0] === 0xf0 ? 'system exclusive' : 'system message') : STATUS_NAMES[status] ?? 'unknown';
  return `${hex} · ${kind}${data[0] < 0xf0 ? ` · channel ${(data[0] & 0x0f) + 1}` : ''}`;
}

export interface MidiConnection {
  /** Names of currently connected inputs. */
  inputs(): string[];
  close(): void;
}

/**
 * Listen on every MIDI input via the Web MIDI API. Rejects if the browser lacks Web MIDI
 * or permission is denied.
 */
export async function connectMidi(
  onEvent: (ev: MidiEvent) => void,
  onInputsChanged: (names: string[]) => void,
  /** Every raw message, before it is interpreted (timing clock and active sensing are left out). */
  onRaw?: (data: ArrayLike<number>) => void,
): Promise<MidiConnection> {
  if (typeof navigator === 'undefined' || !navigator.requestMIDIAccess) {
    throw new Error('Web MIDI is not supported in this browser');
  }
  const access = await navigator.requestMIDIAccess();
  const handler = (e: MIDIMessageEvent): void => {
    if (!e.data) return;
    if (e.data[0] !== 0xf8 && e.data[0] !== 0xfe) onRaw?.(e.data);
    const ev = parseMidiMessage(e.data);
    if (ev) onEvent(ev);
  };
  const attach = (): string[] => {
    const names: string[] = [];
    access.inputs.forEach((input) => {
      input.onmidimessage = handler;
      names.push(input.name ?? 'MIDI input');
    });
    return names;
  };
  onInputsChanged(attach());
  access.onstatechange = () => onInputsChanged(attach());
  return {
    inputs: () => {
      const names: string[] = [];
      access.inputs.forEach((i) => names.push(i.name ?? 'MIDI input'));
      return names;
    },
    close: () => {
      access.inputs.forEach((i) => (i.onmidimessage = null));
      access.onstatechange = null;
    },
  };
}

/**
 * Follows the MIDI "pitch bend sensitivity" message (registered parameter 0: CC 101 = 0, CC 100 = 0, then data entry
 * CC 6 = semitones and CC 38 = cents) that some keyboards and controllers send to set how far the wheel bends.
 */
export class BendRangeTracker {
  private msb = -1;
  private lsb = -1;
  private semitones = 2;
  private cents = 0;

  /** Feed every control change; returns the new range in semitones when this message changed it, else null. */
  handle(controller: number, value: number): number | null {
    if (controller === 101) this.msb = value;
    else if (controller === 100) this.lsb = value;
    else if ((controller === 6 || controller === 38) && this.msb === 0 && this.lsb === 0) {
      if (controller === 6) this.semitones = value;
      else this.cents = value;
      const range = this.semitones + this.cents / 100;
      return range > 0 ? Math.min(24, range) : null;
    }
    return null;
  }
}
