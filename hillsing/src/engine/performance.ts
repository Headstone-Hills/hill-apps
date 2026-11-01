import type { LyricPointer } from '../text/pointer';
import type { SyllablePlan } from '../synth/types';

/** What the performance layer needs from a voice. Implemented by SingerEngine. */
export interface VoiceSink {
  noteOn(plan: SyllablePlan, f0: number, velocity: number, legato: boolean): void;
  noteOff(): void;
  setF0(f0: number): void;
}

export interface PerformanceEvents {
  /** Called when a syllable starts sounding (for UI highlighting). */
  onSyllable?: (index: number) => void;
}

const A4 = 440;
const noteToHz = (note: number, bendSemitones: number): number =>
  A4 * Math.pow(2, (note - 69 + bendSemitones) / 12);

/**
 * Monophonic, last-note-priority performance with legato.
 *
 * - A note pressed while the voice is gated (another key held, or sustain pedal down) is
 *   *legato*: it consumes the next syllable but glides into it without a new onset.
 * - Each fresh note consumes one syllable from the pointer; at end of text, fresh notes
 *   are silent (until the pointer is moved, or wraps if enabled). With `hold` on, notes
 *   re-sing the current syllable instead and the pointer stays where it is. With `melisma` on, a note pressed
 *   while a word is sounding just glides to the new pitch on its vowel.
 * - Releasing the top key while others are held glides back to the previous held pitch
 *   without consuming a syllable.
 */
export class Performance {
  private held: number[] = [];
  private gateOpen = false;
  private pedal = false;
  private lastNote = 60;
  private bend = 0; // -1..1
  bendRange = 2; // semitones
  /** While true, fresh notes sing the current syllable again and the lyric pointer does not move. */
  hold = false;
  /**
   * While true, a note pressed while a word is sounding glides to the new pitch on the vowel already being sung
   * (melisma): no new syllable, no new onset. With nothing sounding it plays normally.
   */
  melisma = false;
  /** The syllable most recently sung, so melisma can sing its vowel again after the key came up. */
  private lastPlan: { nucleus: string; holdFirst?: boolean } | null = null;

  constructor(
    private sink: VoiceSink,
    private readonly pointer: LyricPointer,
    private readonly events: PerformanceEvents = {},
  ) {}

  setSink(sink: VoiceSink): void {
    this.sink = sink;
  }

  get isGated(): boolean {
    return this.gateOpen;
  }

  noteOn(note: number, velocity01: number): void {
    this.held = this.held.filter((n) => n !== note);
    this.held.push(note);
    this.lastNote = note;
    const f0 = this.frequency(note);
    const legato = this.gateOpen;
    if (legato && this.melisma) {
      this.sink.setF0(f0);
      return;
    }
    if (this.melisma && this.lastPlan) {
      // Nothing is sounding any more (the first key came up): sing the same vowel again, with no consonant, and stay on this syllable.
      this.sink.noteOn({ onset: [], nucleus: this.lastPlan.nucleus, coda: [], holdFirst: this.lastPlan.holdFirst }, f0, velocity01, false);
      this.gateOpen = true;
      return;
    }
    const index = this.pointer.index;
    const syl = this.hold ? this.pointer.peek() : this.pointer.advance();
    if (!syl) {
      // End of text: no new syllable. If already sounding, just follow the pitch.
      if (legato) this.sink.setF0(f0);
      return;
    }
    this.lastPlan = { nucleus: syl.nucleus, holdFirst: syl.holdFirst };
    this.sink.noteOn(
      { onset: syl.onset, nucleus: syl.nucleus, coda: syl.coda, holdFirst: syl.holdFirst },
      f0,
      velocity01,
      legato,
    );
    this.gateOpen = true;
    this.events.onSyllable?.(index);
  }

  noteOff(note: number): void {
    const wasTop = this.held[this.held.length - 1] === note;
    this.held = this.held.filter((n) => n !== note);
    if (!this.gateOpen) return;
    if (this.held.length > 0) {
      if (wasTop) {
        this.lastNote = this.held[this.held.length - 1];
        this.sink.setF0(this.frequency(this.lastNote));
      }
      return;
    }
    if (!this.pedal) this.closeGate();
  }

  sustain(down: boolean): void {
    this.pedal = down;
    if (!down && this.held.length === 0 && this.gateOpen) this.closeGate();
  }

  /** Pitch bend as a normalized value in -1..1. */
  pitchBend(value: number): void {
    this.bend = Math.max(-1, Math.min(1, value));
    if (this.gateOpen) this.sink.setF0(this.frequency(this.lastNote));
  }

  allNotesOff(): void {
    this.held = [];
    this.pedal = false;
    if (this.gateOpen) this.closeGate();
  }

  private closeGate(): void {
    this.gateOpen = false;
    this.sink.noteOff();
  }

  private frequency(note: number): number {
    return noteToHz(note, this.bend * this.bendRange);
  }
}
