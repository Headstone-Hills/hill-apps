import { AUDITIONS, buildAudition, type AuditionId } from '../engine/audition';
import type { SingerEngine } from '../synth/engine';
import type { PhonemeLookup } from '../text/cmudict';

export interface LabContext {
  ensureEngine(): Promise<SingerEngine | null>;
  lookup(): PhonemeLookup;
  consonantScale(): number;
  centerNote(): number;
  /** Re-send the MIDI file's sequence to the engine after an audition replaced it. */
  restoreFileSequence(): void;
  /** Current voice + control values as text, for pasting into a message. */
  settingsText(): string;
}

/**
 * Voice lab: audition phrases that each isolate one thing to listen for, plus a copyable
 * settings dump. Phrases play through the same sequencer as MIDI files.
 */
export class VoiceLab {
  private engine: SingerEngine | null = null;
  private current: AuditionId | null = null;
  private readonly buttons = new Map<AuditionId, HTMLButtonElement>();
  private readonly hint: HTMLElement;

  constructor(private readonly ctx: LabContext) {
    const host = document.getElementById('audition-buttons');
    const hint = document.getElementById('audition-hint');
    if (!host || !hint) throw new Error('Missing audition elements');
    this.hint = hint;
    for (const a of AUDITIONS) {
      const b = document.createElement('button');
      b.type = 'button';
      b.textContent = `▶ ${a.label}`;
      b.title = a.hint;
      b.dataset.audition = a.id;
      b.addEventListener('click', () => void this.toggle(a.id));
      host.append(b);
      this.buttons.set(a.id, b);
    }
    const stop = document.createElement('button');
    stop.type = 'button';
    stop.textContent = '⏹ Stop';
    stop.addEventListener('click', () => this.cancel());
    host.append(stop);

    const copy = document.getElementById('copy-settings');
    const out = document.getElementById('settings-out') as HTMLTextAreaElement | null;
    copy?.addEventListener('click', () => {
      const text = ctx.settingsText();
      if (out) {
        out.hidden = false;
        out.value = text;
        out.select();
      }
      void navigator.clipboard?.writeText(text).catch(() => {
        /* clipboard may be blocked; the text box above can be copied by hand */
      });
    });
  }

  get auditioning(): boolean {
    return this.current !== null;
  }

  engineReady(engine: SingerEngine): void {
    this.engine = engine;
  }

  private async toggle(id: AuditionId): Promise<void> {
    if (this.current === id) {
      this.cancel();
      return;
    }
    const engine = this.engine ?? (await this.ctx.ensureEngine());
    if (!engine) return;
    this.engine = engine;
    const seq = buildAudition(id, {
      centerNote: this.ctx.centerNote(),
      consonantScale: this.ctx.consonantScale(),
      lookup: this.ctx.lookup(),
    });
    engine.seqStop();
    engine.seqLoop(false);
    engine.seqLoad(seq.commands, seq.lengthSec, 0);
    engine.seqPlay();
    this.current = id;
    this.hint.textContent = AUDITIONS.find((a) => a.id === id)?.hint ?? '';
    this.mark();
  }

  /** Stop the phrase and give the engine back to the MIDI file panel. */
  cancel(): void {
    // Only touch the engine when a phrase really owns it: this is also called before every MIDI
    // file Play/Pause, and must not stop file playback.
    if (!this.current) return;
    this.current = null;
    this.engine?.seqStop();
    this.mark();
    this.ctx.restoreFileSequence();
  }

  /** The engine finished the phrase on its own. */
  handleEnded(): void {
    if (!this.current) return;
    this.current = null;
    this.mark();
    this.ctx.restoreFileSequence();
  }

  private mark(): void {
    for (const [id, b] of this.buttons) b.classList.toggle('armed', id === this.current);
  }
}
