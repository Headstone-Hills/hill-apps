// Shared state for the UTAU feature: which voicebank is loaded and whether melodies are sung with it. The
// voicebank panel loads one; the voice chooser selects it; the melody panel listens so it renders with it.

import type { Voicebank } from './voicebank';

export class UtauSession {
  private current: Voicebank | null = null;
  private use = false;
  private readonly listeners: Array<() => void> = [];

  get bank(): Voicebank | null {
    return this.current;
  }

  /** True when melodies should be sung by the voicebank: one is loaded and chosen as the voice. */
  get active(): boolean {
    return this.use && this.current !== null;
  }

  /** Load (or clear) the voicebank. Loading one selects it as the voice. */
  set(bank: Voicebank | null): void {
    this.current = bank;
    this.use = bank !== null;
    this.emit();
  }

  /** Choose the voicebank (true) or the formant voices (false) as the voice for melodies. */
  setUse(use: boolean): void {
    this.use = use && this.current !== null;
    this.emit();
  }

  onChange(listener: () => void): void {
    this.listeners.push(listener);
  }

  private emit(): void {
    for (const l of this.listeners) l();
  }
}
