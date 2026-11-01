import type { MidiEvent } from './midi';

export type NavAction = 'reset' | 'back' | 'forward' | 'wordBack' | 'wordForward' | 'hold';

export const NAV_ACTIONS: ReadonlyArray<{ action: NavAction; label: string }> = [
  { action: 'reset', label: 'Reset to start' },
  { action: 'back', label: 'Step back (syllable)' },
  { action: 'forward', label: 'Skip forward (syllable)' },
  { action: 'wordBack', label: 'Back (word)' },
  { action: 'wordForward', label: 'Forward (word)' },
  { action: 'hold', label: 'Hold on the current word (toggle)' },
];

export interface Binding {
  kind: 'note' | 'cc';
  number: number;
}

const STORAGE_KEY = 'hillsing.navBindings.v1';

/**
 * Maps MIDI notes / CCs to lyric navigation. A bound note is swallowed (never sung), so a
 * spare key or pad on the controller can step through the lyrics. "Learn" arms an action;
 * the next note-on or CC press on the controller becomes its binding.
 */
export class NavBindings {
  private map = new Map<NavAction, Binding>();
  private armed: NavAction | null = null;
  /** Notes whose note-on we swallowed, so the matching note-off is swallowed too. */
  private swallowed = new Set<number>();
  /** CC "button" latch so a held/repeating CC only fires once per press. */
  private ccDown = new Set<number>();
  onLearn: ((action: NavAction | null) => void) | null = null;

  constructor(private readonly storage: Pick<Storage, 'getItem' | 'setItem'> | null = null) {
    this.load();
  }

  get armedAction(): NavAction | null {
    return this.armed;
  }

  bindingFor(action: NavAction): Binding | undefined {
    return this.map.get(action);
  }

  learn(action: NavAction | null): void {
    this.armed = action;
    this.onLearn?.(action);
  }

  clear(action: NavAction): void {
    this.map.delete(action);
    this.save();
  }

  /**
   * Returns true if the event was consumed (learn capture or a navigation binding), in
   * which case it must not be played. `fire` is called for a triggered navigation action.
   */
  handle(ev: MidiEvent, fire: (action: NavAction) => void): boolean {
    if (ev.type === 'noteOff') {
      return this.swallowed.delete(ev.note);
    }
    let binding: Binding | null = null;
    let pressed = false;
    if (ev.type === 'noteOn') {
      binding = { kind: 'note', number: ev.note };
      pressed = true;
    } else if (ev.type === 'cc') {
      binding = { kind: 'cc', number: ev.controller };
      pressed = ev.value > 63;
      if (!pressed) this.ccDown.delete(ev.controller);
    }
    if (!binding) return false;

    if (this.armed) {
      // Only learn on a press, and ignore the continuous controllers we use for playing.
      // Never swallow releases (e.g. a sustain-pedal up would otherwise stick).
      if (!pressed) return false;
      if (binding.kind === 'cc' && (binding.number === 1 || binding.number === 64)) return false;
      if (binding.kind === 'cc' && this.ccDown.has(binding.number)) return true;
      const action = this.armed;
      for (const [a, b] of this.map) {
        if (b.kind === binding.kind && b.number === binding.number) this.map.delete(a);
      }
      this.map.set(action, binding);
      if (binding.kind === 'note') this.swallowed.add(binding.number);
      else this.ccDown.add(binding.number);
      this.armed = null;
      this.save();
      this.onLearn?.(null);
      return true;
    }

    for (const [action, b] of this.map) {
      if (b.kind !== binding.kind || b.number !== binding.number) continue;
      if (binding.kind === 'note') {
        this.swallowed.add(binding.number);
        fire(action);
      } else if (pressed && !this.ccDown.has(binding.number)) {
        this.ccDown.add(binding.number);
        fire(action);
      }
      return true;
    }
    return false;
  }

  private load(): void {
    try {
      const raw = this.storage?.getItem(STORAGE_KEY);
      if (!raw) return;
      const obj = JSON.parse(raw) as Record<string, Binding>;
      for (const { action } of NAV_ACTIONS) {
        const b = obj[action];
        if (b && (b.kind === 'note' || b.kind === 'cc') && Number.isInteger(b.number)) {
          this.map.set(action, b);
        }
      }
    } catch {
      /* corrupt or unavailable storage: start with no bindings */
    }
  }

  private save(): void {
    try {
      const obj: Record<string, Binding> = {};
      for (const [a, b] of this.map) obj[a] = b;
      this.storage?.setItem(STORAGE_KEY, JSON.stringify(obj));
    } catch {
      /* storage may be unavailable (private mode); bindings just won't persist */
    }
  }
}
