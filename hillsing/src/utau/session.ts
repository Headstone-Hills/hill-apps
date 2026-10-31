// Shared state for the UTAU feature: which voicebank is loaded. The voicebank panel sets it; the melody
// panel listens so it can offer "UTAU voicebank" as a voice source.

import type { Voicebank } from './voicebank';

export class UtauSession {
  private current: Voicebank | null = null;
  private readonly listeners: Array<(bank: Voicebank | null) => void> = [];

  get bank(): Voicebank | null {
    return this.current;
  }

  set(bank: Voicebank | null): void {
    this.current = bank;
    for (const l of this.listeners) l(bank);
  }

  onChange(listener: (bank: Voicebank | null) => void): void {
    this.listeners.push(listener);
  }
}
