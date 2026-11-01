// Map any MIDI controller (CC) knob or slider to pitch bend or to modulation, for keyboards whose "pitch" or "mod"
// control sends a controller number instead of the standard pitch-bend message. "Learn" arms a target; the next
// controller that moves becomes its source. Controllers used for something else (the sustain pedal, all-notes-off)
// are ignored while learning.

import type { MidiEvent } from './midi';

export type ControlTarget = 'pitch' | 'mod';

export interface ControlChange {
  target: ControlTarget;
  /** Pitch: -1..1 (a value of 64 is the centre). Modulation: 0..1. */
  value: number;
}

const STORAGE_KEY = 'hillsing.controlMap.v1';
const IGNORED = new Set([64, 120, 121, 123]); // sustain pedal, all sound off, reset controllers, all notes off

export class ControlMap {
  private map = new Map<ControlTarget, number>();
  private armed: ControlTarget | null = null;
  onChange: (() => void) | null = null;

  constructor(private readonly storage: Pick<Storage, 'getItem' | 'setItem'> | null = null) {
    try {
      const raw = storage?.getItem(STORAGE_KEY);
      if (raw) {
        const obj = JSON.parse(raw) as Record<string, number>;
        for (const t of ['pitch', 'mod'] as const) if (Number.isInteger(obj[t]) && obj[t] >= 0 && obj[t] < 128) this.map.set(t, obj[t]);
      }
    } catch {
      /* unreadable saved mapping: start with none */
    }
  }

  get armedTarget(): ControlTarget | null {
    return this.armed;
  }

  controllerFor(target: ControlTarget): number | undefined {
    return this.map.get(target);
  }

  learn(target: ControlTarget | null): void {
    this.armed = target;
    this.onChange?.();
  }

  clear(target: ControlTarget): void {
    this.map.delete(target);
    this.save();
    this.onChange?.();
  }

  /**
   * Feed every MIDI event. Returns the change to apply when the event came from a mapped controller (or was just learned),
   * or null when it is not ours. While a target is armed, the first usable controller becomes its source.
   */
  handle(ev: MidiEvent): ControlChange | null {
    if (ev.type !== 'cc') return null;
    if (this.armed && !IGNORED.has(ev.controller)) {
      for (const [t, c] of this.map) if (c === ev.controller) this.map.delete(t); // one controller, one job
      this.map.set(this.armed, ev.controller);
      this.armed = null;
      this.save();
      this.onChange?.();
    }
    for (const [target, controller] of this.map) {
      if (controller !== ev.controller) continue;
      return target === 'pitch' ? { target, value: Math.max(-1, Math.min(1, (ev.value - 64) / 63)) } : { target, value: ev.value / 127 };
    }
    return null;
  }

  private save(): void {
    try {
      this.storage?.setItem(STORAGE_KEY, JSON.stringify(Object.fromEntries(this.map)));
    } catch {
      /* storage unavailable: the mapping just will not persist */
    }
  }
}

/** One line saying what a MIDI event was, for the on-screen monitor. */
export function describeMidi(ev: MidiEvent): string {
  switch (ev.type) {
    case 'noteOn':
      return `note ${ev.note} on (velocity ${Math.round(ev.velocity * 127)})`;
    case 'noteOff':
      return `note ${ev.note} off`;
    case 'cc':
      return `controller CC ${ev.controller} = ${ev.value}`;
    case 'pitchBend':
      return `pitch bend ${ev.value >= 0 ? '+' : ''}${ev.value.toFixed(2)}`;
    case 'allNotesOff':
      return 'all notes off';
  }
}
