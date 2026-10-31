import { Performance, type VoiceSink } from '../engine/performance';
import { NAV_ACTIONS, NavBindings, type NavAction } from '../midi/bindings';
import { connectMidi, type MidiEvent } from '../midi/midi';
import { SingerEngine } from '../synth/engine';
import type { VoiceParamName } from '../synth/klatt';
import { loadDict, makeLookup, type PhonemeLookup } from '../text/cmudict';
import { buildLyrics } from '../text/lyrics';
import { LyricPointer } from '../text/pointer';
import { MidiFilePanel } from './midifile';

const DEFAULT_LYRICS = 'Twinkle twinkle little star, how I wonder what you are';
const LYRICS_KEY = 'hillsing.lyrics.v1';

// Computer-keyboard piano: semitone offsets from the base note (white keys on the home row).
const KEY_OFFSETS: Record<string, number> = {
  KeyA: 0, KeyW: 1, KeyS: 2, KeyE: 3, KeyD: 4, KeyF: 5, KeyT: 6, KeyG: 7, KeyY: 8,
  KeyH: 9, KeyU: 10, KeyJ: 11, KeyK: 12, KeyO: 13, KeyL: 14, KeyP: 15, Semicolon: 16,
};
const BASE_NOTE = 48; // C3 at octave 0 (about 130 Hz: comfortable for the voice)

function safeStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function startApp(): void {
  const $ = <T extends HTMLElement>(id: string): T => {
    const el = document.getElementById(id);
    if (!el) throw new Error(`Missing element #${id}`);
    return el as T;
  };
  const storage = safeStorage();

  const pointer = new LyricPointer();
  let engine: SingerEngine | null = null;
  let lookup: PhonemeLookup = () => undefined; // until the dictionary loads: guessed pronunciations

  const sink: VoiceSink = {
    noteOn: (plan, f0, vel, legato) => engine?.noteOn(plan, f0, vel, legato),
    noteOff: () => engine?.noteOff(),
    setF0: (f0) => engine?.setF0(f0),
  };
  const performance = new Performance(sink, pointer);
  const bindings = new NavBindings(storage);
  let filePanel: MidiFilePanel | null = null;

  // ---- Lyrics ----------------------------------------------------------------------
  const lyricsBox = $<HTMLTextAreaElement>('lyrics');
  const sylBox = $('syllables');
  const posLabel = $('position');
  let chips: HTMLElement[] = [];

  lyricsBox.value = storage?.getItem(LYRICS_KEY) ?? DEFAULT_LYRICS;

  function rebuildLyrics(keepPosition: boolean): void {
    const lyrics = buildLyrics(lyricsBox.value, lookup);
    pointer.setLyrics(lyrics, keepPosition);
    sylBox.textContent = '';
    chips = [];
    let n = 0;
    for (const word of lyrics.words) {
      const w = document.createElement('span');
      w.className = 'word' + (word.inDict ? '' : ' guessed');
      if (!word.inDict) w.title = `"${word.text}" is not in the dictionary: pronunciation guessed`;
      for (let i = 0; i < word.count; i++) {
        const syl = lyrics.syllables[word.start + i];
        const chip = document.createElement('span');
        chip.className = 'syl';
        const idx = n++;
        chip.dataset.index = String(idx);
        chip.append(document.createTextNode(syl.label));
        const ph = document.createElement('span');
        ph.className = 'ph';
        ph.textContent = [...syl.onset, syl.nucleus.replace(/\d$/, ''), ...syl.coda].join(' ');
        chip.append(ph);
        chip.addEventListener('click', () => pointer.setIndex(idx));
        chips.push(chip);
        w.append(chip);
      }
      sylBox.append(w);
    }
    const end = document.createElement('span');
    end.className = 'end-marker';
    end.id = 'end-marker';
    end.textContent = '⏹';
    end.title = 'End of text';
    sylBox.append(end);
    updatePointerView();
    filePanel?.lyricsChanged();
  }

  function updatePointerView(): void {
    const idx = pointer.index;
    chips.forEach((chip, i) => {
      chip.classList.toggle('done', i < idx);
      chip.classList.toggle('next', i === idx);
    });
    const end = document.getElementById('end-marker');
    if (end) end.style.opacity = pointer.atEnd ? '1' : '0.25';
    posLabel.textContent = pointer.length
      ? pointer.atEnd
        ? `— end of text (${pointer.length} syllables)`
        : `— next: ${idx + 1} of ${pointer.length}`
      : '— type some lyrics';
    chips[idx]?.scrollIntoView({ block: 'nearest' });
  }
  pointer.onChange = updatePointerView;

  let debounce = 0;
  lyricsBox.addEventListener('input', () => {
    window.clearTimeout(debounce);
    debounce = window.setTimeout(() => {
      try {
        storage?.setItem(LYRICS_KEY, lyricsBox.value);
      } catch {
        /* storage unavailable: fine */
      }
      rebuildLyrics(true);
    }, 150);
  });

  rebuildLyrics(false);
  loadDict(`${import.meta.env.BASE_URL}cmudict.txt`)
    .then((dict) => {
      lookup = makeLookup(dict);
      $('dict-status').textContent = `Dictionary loaded (${dict.size.toLocaleString()} words). Words not found are guessed (dashed underline).`;
      rebuildLyrics(true);
    })
    .catch((err: unknown) => {
      $('dict-status').textContent = `Dictionary failed to load (${String(err)}); all words are being guessed.`;
    });

  // ---- Navigation ------------------------------------------------------------------
  const navActions: Record<NavAction, () => void> = {
    reset: () => pointer.reset(),
    back: () => pointer.back(),
    forward: () => pointer.forward(),
    wordBack: () => pointer.prevWord(),
    wordForward: () => pointer.nextWord(),
  };
  $('nav-reset').addEventListener('click', navActions.reset);
  $('nav-back').addEventListener('click', navActions.back);
  $('nav-forward').addEventListener('click', navActions.forward);
  $('nav-word-back').addEventListener('click', navActions.wordBack);
  $('nav-word-forward').addEventListener('click', navActions.wordForward);
  const wrap = $<HTMLInputElement>('wrap');
  wrap.addEventListener('change', () => (pointer.wrap = wrap.checked));

  // MIDI navigation bindings table.
  const bindTable = $<HTMLTableElement>('bindings');
  function renderBindings(): void {
    bindTable.textContent = '';
    for (const { action, label } of NAV_ACTIONS) {
      const tr = bindTable.insertRow();
      tr.insertCell().textContent = label;
      const b = bindings.bindingFor(action);
      const state = tr.insertCell();
      state.className = 'hint';
      state.textContent = bindings.armedAction === action
        ? 'press a key/pad/CC…'
        : b ? `${b.kind === 'note' ? 'note' : 'CC'} ${b.number}` : 'unbound';
      const actions = tr.insertCell();
      const learn = document.createElement('button');
      learn.type = 'button';
      learn.textContent = bindings.armedAction === action ? 'Cancel' : 'Learn';
      learn.classList.toggle('armed', bindings.armedAction === action);
      learn.addEventListener('click', () => bindings.learn(bindings.armedAction === action ? null : action));
      actions.append(learn);
      if (b) {
        const clear = document.createElement('button');
        clear.type = 'button';
        clear.textContent = 'Clear';
        clear.addEventListener('click', () => {
          bindings.clear(action);
          renderBindings();
        });
        actions.append(' ', clear);
      }
    }
  }
  bindings.onLearn = renderBindings;
  renderBindings();

  // ---- Voice controls --------------------------------------------------------------
  const params: Array<{ id: string; name: VoiceParamName; fmt: (v: number) => string }> = [
    { id: 'formant', name: 'formantShift', fmt: (v) => `${v.toFixed(2)}×` },
    { id: 'consonant', name: 'consonantScale', fmt: (v) => `${v.toFixed(2)}×` },
    { id: 'glide', name: 'portamento', fmt: (v) => `${Math.round(v)} ms` },
    { id: 'volume', name: 'volume', fmt: (v) => `${Math.round(v * 100)}%` },
  ];
  const paramValues = new Map<VoiceParamName, number>();
  for (const p of params) {
    const input = $<HTMLInputElement>(p.id);
    const out = $(`out-${p.id}`);
    const apply = (): void => {
      const v = Number(input.value);
      paramValues.set(p.name, v);
      out.textContent = p.fmt(v);
      engine?.setParam(p.name, v);
      if (p.name === 'consonantScale') filePanel?.settingsChanged(); // changes consonant pre-roll
    };
    input.addEventListener('input', apply);
    input.addEventListener('pointerup', () => input.blur()); // so computer keys play again
    apply();
  }

  // ---- Audio start -----------------------------------------------------------------
  const startBtn = $<HTMLButtonElement>('start');
  const audioStatus = $('audio-status');
  const latencyLabel = $('latency');
  const meterFill = $('meter-fill');
  let starting = false;

  async function startAudio(): Promise<void> {
    if (engine || starting) return;
    starting = true;
    startBtn.disabled = true;
    audioStatus.textContent = 'Starting audio…';
    try {
      engine = await SingerEngine.create();
      for (const [name, value] of paramValues) engine.setParam(name, value);
      filePanel?.engineReady(engine);
      audioStatus.textContent = `Audio on (${engine.context.sampleRate} Hz)`;
      startBtn.textContent = 'Audio running';
      window.setTimeout(() => {
        if (engine) latencyLabel.textContent = `Output latency ≈ ${engine.latencyMs().toFixed(0)} ms (browser estimate)`;
      }, 500);
      requestAnimationFrame(function meter() {
        if (engine) meterFill.style.width = `${Math.min(100, engine.peakLevel() * 140)}%`;
        requestAnimationFrame(meter);
      });
      (window as unknown as { __hillsing?: unknown }).__hillsing = {
        peakLevel: () => engine?.peakLevel() ?? 0,
      };
    } catch (err) {
      audioStatus.textContent = `Audio failed: ${String(err)}`;
      startBtn.disabled = false;
    } finally {
      starting = false;
    }
  }
  startBtn.addEventListener('click', () => void startAudio());

  filePanel = new MidiFilePanel({
    syllables: () => pointer.current.syllables,
    consonantScale: () => paramValues.get('consonantScale') ?? 0.7,
    ensureEngine: async () => {
      await startAudio();
      return engine;
    },
  });

  // ---- Input: MIDI + computer keyboard ---------------------------------------------
  function handleEvent(ev: MidiEvent): void {
    if (bindings.handle(ev, (a) => navActions[a]())) return;
    switch (ev.type) {
      case 'noteOn':
        if (!engine) {
          audioStatus.textContent = 'Click "Start audio" first (browsers require a click).';
          return;
        }
        performance.noteOn(ev.note, ev.velocity);
        break;
      case 'noteOff':
        performance.noteOff(ev.note);
        break;
      case 'pitchBend':
        performance.pitchBend(ev.value);
        break;
      case 'allNotesOff':
        performance.allNotesOff();
        break;
      case 'cc':
        if (ev.controller === 1) engine?.setParam('vibratoDepth', ev.value / 127);
        else if (ev.controller === 64) performance.sustain(ev.value >= 64);
        break;
    }
  }

  const midiStatus = $('midi-status');
  connectMidi(handleEvent, (names) => {
    midiStatus.textContent = names.length ? `MIDI: ${names.join(', ')}` : 'MIDI: no input devices found';
  }).catch((err: unknown) => {
    midiStatus.textContent = `MIDI unavailable (${err instanceof Error ? err.message : String(err)}); use the computer keyboard`;
  });

  let octave = 0;
  const octaveLabel = $('octave');
  const showOctave = (): void => {
    octaveLabel.textContent = `C${3 + octave}`;
  };
  showOctave();
  const pressed = new Map<string, number>(); // key code -> note (so octave changes don't strand notes)

  const isTyping = (t: EventTarget | null): boolean =>
    t instanceof HTMLTextAreaElement || t instanceof HTMLSelectElement ||
    (t instanceof HTMLInputElement && t.type !== 'checkbox');

  window.addEventListener('keydown', (e) => {
    if (isTyping(e.target) || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.code in KEY_OFFSETS) {
      if (e.repeat || pressed.has(e.code)) return;
      if (!engine) {
        void startAudio(); // this key press is the user gesture; the next one will sound
        return;
      }
      const note = BASE_NOTE + 12 * octave + KEY_OFFSETS[e.code];
      pressed.set(e.code, note);
      handleEvent({ type: 'noteOn', note, velocity: 0.8 });
      e.preventDefault();
    } else if (e.code === 'KeyZ') {
      octave = Math.max(-2, octave - 1);
      showOctave();
    } else if (e.code === 'KeyX') {
      octave = Math.min(3, octave + 1);
      showOctave();
    } else if (e.code === 'Space' && !(e.target instanceof HTMLButtonElement)) {
      void filePanel?.togglePlay();
      e.preventDefault();
    } else if (e.code === 'ArrowLeft') {
      (e.shiftKey ? navActions.wordBack : navActions.back)();
      e.preventDefault();
    } else if (e.code === 'ArrowRight') {
      (e.shiftKey ? navActions.wordForward : navActions.forward)();
      e.preventDefault();
    } else if (e.code === 'Home') {
      navActions.reset();
      e.preventDefault();
    }
  });
  window.addEventListener('keyup', (e) => {
    const note = pressed.get(e.code);
    if (note === undefined) return;
    pressed.delete(e.code);
    handleEvent({ type: 'noteOff', note });
  });
  window.addEventListener('blur', () => {
    pressed.clear();
    performance.allNotesOff();
  });
}
