// An on-screen piano you can play with a finger or a mouse: every touch plays its own key (so chords of fingers and a
// thumb on the Melisma button work), and sliding a finger across keys plays each one in turn, legato.

export interface PianoKey {
  note: number;
  black: boolean;
  /** Left edge and width as fractions of the keyboard's width. */
  left: number;
  width: number;
}

const WHITE_STEPS = [0, 2, 4, 5, 7, 9, 11]; // semitones above C for C D E F G A B
const BLACK_AFTER = new Set([0, 1, 3, 4, 5]); // a black key follows C, D, F, G and A

/** Layout of `whiteCount` white keys starting on the C at `startNote` (which must be a C), black keys included. */
export function pianoLayout(whiteCount: number, startNote: number): PianoKey[] {
  const w = 1 / whiteCount;
  const keys: PianoKey[] = [];
  for (let i = 0; i < whiteCount; i++) {
    const degree = i % 7;
    const note = startNote + 12 * Math.floor(i / 7) + WHITE_STEPS[degree];
    keys.push({ note, black: false, left: i * w, width: w });
  }
  for (let i = 0; i < whiteCount - 1; i++) {
    if (!BLACK_AFTER.has(i % 7)) continue;
    const note = startNote + 12 * Math.floor(i / 7) + WHITE_STEPS[i % 7] + 1;
    const bw = w * 0.62;
    keys.push({ note, black: true, left: (i + 1) * w - bw / 2, width: bw });
  }
  return keys;
}

const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
export const noteLabel = (note: number): string => `${NAMES[note % 12]}${Math.floor(note / 12) - 1}`;

export interface PianoHooks {
  noteOn(note: number): void;
  noteOff(note: number): void;
}

export class OnScreenPiano {
  private startNote: number;
  private whiteCount = 8;
  private readonly down = new Map<number, number>(); // pointer id -> note it is playing
  private readonly keyEls = new Map<number, HTMLElement>();

  constructor(
    private readonly keysEl: HTMLElement,
    private readonly hooks: PianoHooks,
    startNote = 60,
    private readonly onRange: (label: string) => void = () => {},
  ) {
    this.startNote = startNote;
    keysEl.addEventListener('pointerdown', (e) => this.press(e));
    keysEl.addEventListener('pointermove', (e) => this.move(e));
    for (const t of ['pointerup', 'pointercancel', 'lostpointercapture'] as const) keysEl.addEventListener(t, (e) => this.release(e));
    keysEl.addEventListener('contextmenu', (e) => e.preventDefault());
    new ResizeObserver(() => this.fit()).observe(keysEl);
    window.addEventListener('blur', () => this.allOff());
    document.addEventListener('visibilitychange', () => {
      if (document.hidden) this.allOff();
    });
    this.fit();
  }

  /** Shift the keyboard by whole octaves (lowest start is C1, highest keeps the top key at or below G9). */
  shiftOctave(delta: number): void {
    const next = this.startNote + 12 * delta;
    if (next < 24 || next + 12 * Math.floor((this.whiteCount - 1) / 7) > 108) return;
    this.allOff();
    this.startNote = next;
    this.render();
  }

  get start(): number {
    return this.startNote;
  }

  private fit(): void {
    const width = this.keysEl.clientWidth;
    if (width <= 0) return;
    // About 42 px per white key, from one octave plus the next C up to two octaves plus a C.
    const count = Math.max(8, Math.min(15, Math.floor(width / 42) + 1));
    if (count !== this.whiteCount || this.keyEls.size === 0) {
      this.whiteCount = count;
      this.allOff();
      this.render();
    }
  }

  private render(): void {
    this.keysEl.textContent = '';
    this.keyEls.clear();
    const keys = pianoLayout(this.whiteCount, this.startNote);
    for (const k of keys) {
      const el = document.createElement('div');
      el.className = k.black ? 'pk black' : 'pk white';
      el.dataset.note = String(k.note);
      el.style.left = `${k.left * 100}%`;
      el.style.width = `${k.width * 100}%`;
      el.setAttribute('role', 'button');
      el.setAttribute('aria-label', noteLabel(k.note));
      if (!k.black && k.note % 12 === 0) {
        const c = document.createElement('span');
        c.className = 'pk-label';
        c.textContent = noteLabel(k.note);
        el.append(c);
      }
      this.keysEl.append(el);
      this.keyEls.set(k.note, el);
    }
    const last = keys.filter((k) => !k.black).slice(-1)[0];
    this.onRange(`${noteLabel(this.startNote)}–${noteLabel(last.note)}`);
  }

  private noteAt(x: number, y: number): number | null {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>('.pk');
    return el && this.keysEl.contains(el) ? Number(el.dataset.note) : null;
  }

  private mark(note: number, on: boolean): void {
    this.keyEls.get(note)?.classList.toggle('down', on);
  }

  private press(e: PointerEvent): void {
    const note = this.noteAt(e.clientX, e.clientY);
    if (note === null) return;
    e.preventDefault();
    try {
      this.keysEl.setPointerCapture(e.pointerId);
    } catch {
      /* the pointer may already be gone */
    }
    this.down.set(e.pointerId, note);
    this.mark(note, true);
    this.hooks.noteOn(note);
  }

  private move(e: PointerEvent): void {
    const from = this.down.get(e.pointerId);
    if (from === undefined) return;
    const to = this.noteAt(e.clientX, e.clientY);
    if (to === null || to === from) return;
    // Start the new note before ending the old one, so the voice glides (legato) instead of restarting.
    this.down.set(e.pointerId, to);
    this.mark(to, true);
    this.hooks.noteOn(to);
    this.mark(from, false);
    this.hooks.noteOff(from);
  }

  private release(e: PointerEvent): void {
    const note = this.down.get(e.pointerId);
    if (note === undefined) return;
    this.down.delete(e.pointerId);
    this.mark(note, false);
    this.hooks.noteOff(note);
  }

  /** Let go of everything (the page lost focus, the layout changed). */
  allOff(): void {
    for (const note of this.down.values()) {
      this.mark(note, false);
      this.hooks.noteOff(note);
    }
    this.down.clear();
  }
}
