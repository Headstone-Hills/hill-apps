import { Performance, type VoiceSink } from '../engine/performance';
import { NAV_ACTIONS, NavBindings, type NavAction } from '../midi/bindings';
import { BendRangeTracker, connectMidi, describeRaw, parseMidiMessage, type MidiEvent } from '../midi/midi';
import { ControlMap, describeMidi } from '../midi/controls';
import { SingerEngine } from '../synth/engine';
import type { VoiceParamName } from '../synth/klatt';
import { loadDict, makeLookup, type PhonemeLookup } from '../text/cmudict';
import { buildLyrics } from '../text/lyrics';
import { LyricPointer } from '../text/pointer';
import { getVoice, VOICES } from '../synth/voices';
import { VoiceLab } from './lab';
import { MidiFilePanel } from './midifile';
import { fxModel, getBus } from '../audio/bus';
import { UtauLiveVoice } from '../utau/live';
import { EFFECTS } from '../audio/fxparams';
import { diag, formatTuning, TuningModel, TUNING_PARAMS } from '../utau/tuning';
import { TuningPanel } from './tuning';
import { UtauSession } from '../utau/session';
import { EffectsPanel } from './effects';
import { UtauPanel } from './utau';
import { Spectrogram } from './spectrogram';
import { Scope } from './scope';
import { OnScreenPiano } from './keyboard';
import { ScreenBottomPin } from './pinbar';
import { ALL_SOUNDS_TEXT, SENTENCE_TEXT } from '../text/testtext';

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

  // Live keys go to the sample-based voice when a UTAU voicebank is chosen, else to the formant synth.
  const utauSession = new UtauSession();
  const tuning = new TuningModel(storage);
  let utauLive: UtauLiveVoice | null = null;
  const sink: VoiceSink = {
    noteOn: (plan, f0, vel, legato) => (utauSession.active ? utauLive?.noteOn(plan, f0, vel, legato) : engine?.noteOn(plan, f0, vel, legato)),
    noteOff: () => {
      scope.mark('↑');
      if (utauSession.active) utauLive?.noteOff();
      else engine?.noteOff();
    },
    setF0: (f0, immediate) => (utauSession.active ? utauLive?.setF0(f0, immediate) : engine?.setF0(f0, immediate)),
  };
  const scope = new Scope($<HTMLCanvasElement>('scope-wave'), $<HTMLCanvasElement>('scope-spec'), {
    freeze: $<HTMLButtonElement>('scope-freeze'),
    clipFreeze: $<HTMLInputElement>('scope-clipfreeze'),
    window: $<HTMLSelectElement>('scope-window'),
    readout: $('scope-readout'),
    save: $<HTMLButtonElement>('scope-save'),
    copy: $<HTMLButtonElement>('scope-copy'),
    record: $<HTMLButtonElement>('scope-rec'),
    clear: $<HTMLButtonElement>('scope-clear'),
    toggle: $<HTMLButtonElement>('scope-toggle'),
    badge: $('scope-badge'),
    meta: () => {
      const t = tuning.values;
      const on = EFFECTS.filter((e) => fxModel.state.enabled[e.id]).map((e) => `${e.name} ${e.params.map((q) => `${q.id}=${fxModel.state.values[e.id][q.id]}`).join(' ')}`);
      return [
        `Voice: ${utauSession.active ? `sample-based (${utauSession.bank?.name ?? 'UTAU'})` : getVoice(voiceId).name} · lyrics: ${lyricsBox.value.replace(/\s+/g, ' ').slice(0, 60)}`,
        `Tuning: ${TUNING_PARAMS.map((p) => `${p.id} ${formatTuning(p.id, t[p.id])}`).join(', ')}`,
        `Effects on: ${on.length ? on.join(' | ') : 'none'}`,
        ...(flags.length ? [`Flagged as artifact: ${flags.slice(-12).map((f) => `${f.label} [${f.phones}]`).join(', ')}`] : []),
      ];
    },
  });
  // The scope can be folded away to save room; the choice is remembered.
  const SCOPE_KEY = 'hillsing.scope.collapsed.v1';
  const scopeToggle = $<HTMLButtonElement>('scope-toggle');
  const scopeSection = $('scope');
  const setScopeCollapsed = (collapsed: boolean, remember = true): void => {
    scopeSection.classList.toggle('collapsed', collapsed);
    scopeToggle.setAttribute('aria-expanded', String(!collapsed));
    scopeToggle.textContent = collapsed ? '▸' : '▾';
    if (remember) {
      try {
        storage?.setItem(SCOPE_KEY, collapsed ? '1' : '0');
      } catch {
        /* storage unavailable: fine */
      }
    }
    if (!collapsed) window.requestAnimationFrame(() => window.dispatchEvent(new Event('resize')));
  };
  scopeToggle.addEventListener('click', () => {
    setScopeCollapsed(!scopeSection.classList.contains('collapsed'));
    scopeToggle.blur();
  });
  setScopeCollapsed(storage?.getItem(SCOPE_KEY) === '1', false);

  // The syllable that is sounding (or sounded last), so the Q key can flag it as having an artifact.
  let playing: { index: number; note: number; at: number } | null = null;
  let sounding = false; // a key is down (or the pedal holds the note): the syllable being sung stays highlighted
  let lastNoteOn = 60;
  const performance = new Performance(sink, pointer, {
    onSyllable: (i) => {
      playing = { index: i, note: lastNoteOn, at: window.performance.now() };
      sounding = true;
      updatePointerView();
      scope.mark(pointer.current.syllables[i]?.label ?? '');
    },
    onGate: (open) => {
      sounding = open;
      updatePointerView();
    },
  });
  const bindings = new NavBindings(storage);
  let filePanel: MidiFilePanel | null = null;
  let lab: VoiceLab | null = null;
  const VOICE_KEY = 'hillsing.voice.v1';
  let voiceId = getVoice(storage?.getItem(VOICE_KEY) ?? 'classic').id;

  // ---- Lyrics ----------------------------------------------------------------------
  const lyricsBox = $<HTMLTextAreaElement>('lyrics');
  const sylBox = $('syllables');
  const posLabel = $('position');
  let chips: HTMLElement[] = [];

  lyricsBox.value = storage?.getItem(LYRICS_KEY) ?? DEFAULT_LYRICS;

  // ---- Flags: press Q while a syllable plays to mark it as having a strange sound artifact ---------------------
  interface Flag {
    index: number;
    label: string;
    phones: string;
    note: number;
    text: string;
    at: number;
    voice: string;
  }
  const FLAGS_KEY = 'hillsing.flags.v1';
  let flags: Flag[] = [];
  try {
    const raw = storage?.getItem(FLAGS_KEY);
    if (raw) flags = (JSON.parse(raw) as Flag[]).filter((f) => typeof f.index === 'number' && typeof f.label === 'string');
  } catch {
    /* unreadable saved flags: start empty */
  }
  const flagsCopy = $<HTMLButtonElement>('flags-copy');
  const flagsClear = $<HTMLButtonElement>('flags-clear');
  const noteName = (n: number): string => `${['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'][n % 12]}${Math.floor(n / 12) - 1}`;
  function saveFlags(): void {
    try {
      storage?.setItem(FLAGS_KEY, JSON.stringify(flags.slice(-500)));
    } catch {
      /* storage unavailable: fine */
    }
  }
  function refreshFlagChips(): void {
    chips.forEach((chip, i) => chip.classList.toggle('flagged', flags.some((f) => f.index === i && f.text === lyricsBox.value)));
    flagsCopy.textContent = `⚑ ${flags.length}`;
    flagsClear.disabled = flags.length === 0;
  }
  function flagText(): string {
    const lines = [`Flagged as having a sound artifact in HillSing (${flags.length}):`, ...scope.metaLines().filter((l) => !l.startsWith('Flagged as artifact')), ''];
    flags.forEach((f, i) => {
      lines.push(`${i + 1}. "${f.label}" [${f.phones}] syllable ${f.index + 1}, note ${f.note} (${noteName(f.note)}), voice ${f.voice}, ${new Date(f.at).toLocaleTimeString()}`);
    });
    return lines.join('\n');
  }
  function flagArtifact(): void {
    const p = playing;
    if (!p || (!performance.isGated && window.performance.now() - p.at > 2500)) {
      scope.say('Nothing was playing: press Q while a syllable sounds (or just after it).');
      return;
    }
    const syl = pointer.current.syllables[p.index];
    if (!syl) return;
    const phones = [...syl.onset, syl.nucleus.replace(/\d$/, ''), ...syl.coda].join(' ');
    const voice = utauSession.active ? `sample-based (${utauSession.bank?.name ?? 'UTAU'})` : getVoice(voiceId).name;
    flags.push({ index: p.index, label: syl.label, phones, note: p.note, text: lyricsBox.value, at: Date.now(), voice });
    saveFlags();
    scope.mark(`${syl.label} [${phones}]`, true);
    scope.say(`⚠ Flagged "${syl.label}" [${phones}] at note ${noteName(p.note)}. ${flags.length} flagged so far; ⚑ copies the list.`);
    refreshFlagChips();
  }
  flagsCopy.addEventListener('click', () => {
    flagsCopy.blur();
    const text = flagText();
    const done = (): void => scope.say(`Copied ${flags.length} flagged syllable${flags.length === 1 ? '' : 's'}: paste them into a message.`);
    void navigator.clipboard?.writeText(text).then(done, () => {
      const ta = document.createElement('textarea');
      ta.value = text;
      document.body.append(ta);
      ta.select();
      try {
        document.execCommand('copy');
        done();
      } catch {
        scope.say('Could not copy the list.');
      }
      ta.remove();
    });
  });
  flagsClear.addEventListener('click', () => {
    flagsClear.blur();
    flags = [];
    saveFlags();
    refreshFlagChips();
    scope.say('Flags cleared.');
  });

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
    refreshFlagChips();
    filePanel?.lyricsChanged();
  }

  function updatePointerView(): void {
    // While a key is down the syllable being sung stays highlighted; the highlight moves on when the key comes up.
    // (If you have jumped somewhere else with the arrows or a click, the highlight follows that instead.)
    const next = pointer.index;
    const singing = sounding && playing !== null && (next === playing.index + 1 || next === playing.index);
    const idx = singing && playing ? playing.index : next;
    chips.forEach((chip, i) => {
      chip.classList.toggle('done', i < idx);
      chip.classList.toggle('next', i === idx);
      chip.classList.toggle('playing', singing && i === idx);
    });
    const end = document.getElementById('end-marker');
    if (end) end.style.opacity = pointer.atEnd && !singing ? '1' : '0.25';
    posLabel.textContent = pointer.length
      ? singing
        ? `— singing: ${idx + 1} of ${pointer.length}`
        : pointer.atEnd
          ? `— end of text (${pointer.length} syllables)`
          : performance.hold
            ? `— locked on: ${idx + 1} of ${pointer.length}`
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
    hold: () => setHold(!performance.hold),
  };
  $('nav-reset').addEventListener('click', navActions.reset);
  $('nav-back').addEventListener('click', navActions.back);
  $('nav-forward').addEventListener('click', navActions.forward);
  $('nav-word-back').addEventListener('click', navActions.wordBack);
  $('nav-word-forward').addEventListener('click', navActions.wordForward);
  const holdBtn = $<HTMLButtonElement>('nav-hold');
  const pianoHold = $<HTMLButtonElement>('piano-hold');
  /** Hold on the current word: notes keep singing it and the lyric pointer does not move until hold is turned off. */
  function setHold(on: boolean): void {
    // Turning the lock on while a syllable is sounding locks on that syllable (the pointer has already moved past it).
    if (on && !performance.hold && sounding && playing) pointer.setIndex(playing.index);
    // Letting go of the lock after the locked syllable has been sung: the next note moves on to the one after it.
    if (!on && performance.hold && playing && pointer.index === playing.index) pointer.setIndex(pointer.index + 1);
    performance.hold = on;
    for (const b of [holdBtn, pianoHold]) {
      b.setAttribute('aria-pressed', String(on));
      b.classList.toggle('active', on);
    }
    holdBtn.textContent = on ? '▶ Release hold' : '⏸ Hold word';
    pianoHold.textContent = on ? '▶ Release' : '⏸ Hold';
    updatePointerView();
  }
  for (const b of [holdBtn, pianoHold]) {
    b.addEventListener('click', () => {
      navActions.hold();
      b.blur(); // so computer keys play again
    });
  }
  // Melisma: while Shift (or an on-screen Melisma button) is held, notes glide on the vowel being sung.
  const melismaBtns = [$<HTMLButtonElement>('nav-melisma'), $<HTMLButtonElement>('piano-melisma')];
  const melismaHeld = new Set<HTMLElement>();
  let shiftDown = false;
  function updateMelisma(): void {
    const on = shiftDown || melismaHeld.size > 0;
    performance.melisma = on;
    for (const b of melismaBtns) b.classList.toggle('active', on);
  }
  for (const btn of melismaBtns) {
    btn.addEventListener('pointerdown', (e) => {
      e.preventDefault(); // keep focus where it is so computer keys keep playing
      try {
        btn.setPointerCapture(e.pointerId);
      } catch {
        /* the pointer may already be gone */
      }
      melismaHeld.add(btn);
      updateMelisma();
    });
    const release = (): void => {
      melismaHeld.delete(btn);
      updateMelisma();
    };
    btn.addEventListener('pointerup', release);
    btn.addEventListener('pointercancel', release);
  }
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
    { id: 'conlevel', name: 'consonantLevel', fmt: (v) => `${v.toFixed(2)}×` },
    { id: 'noise', name: 'noiseRolloff', fmt: (v) => `${(v / 1000).toFixed(1)} kHz` },
    { id: 'burst', name: 'burstLevel', fmt: (v) => `${v.toFixed(2)}×` },
    { id: 'fric', name: 'fricLevel', fmt: (v) => `${v.toFixed(2)}×` },
    { id: 'asp', name: 'aspLevel', fmt: (v) => `${v.toFixed(2)}×` },
    { id: 'breath', name: 'breathiness', fmt: (v) => `${Math.round(v * 100)}%` },
    { id: 'bright', name: 'brightness', fmt: (v) => (v > 0 ? `+${v.toFixed(2)}` : v.toFixed(2)) },
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
  const meterBox = meterFill.parentElement as HTMLElement;
  let clipUntil = 0;
  let starting = false;

  const effectsPanel = new EffectsPanel($('fx-grid'), $<HTMLSelectElement>('fx-preset'), $<HTMLButtonElement>('fx-reset'), fxModel);

  async function startAudio(): Promise<void> {
    if (engine || starting) return;
    starting = true;
    startBtn.disabled = true;
    audioStatus.textContent = 'Starting audio…';
    try {
      const bus = await getBus();
      effectsPanel.setBitcrushAvailable(bus.fx.bitcrushAvailable);
      scope.attach(bus);
      engine = await SingerEngine.create(bus);
      utauLive = new UtauLiveVoice(engine.context, engine.audioInput, () => utauSession.bank, () => tuning.values);
      for (const [name, value] of paramValues) engine.setParam(name, value);
      engine.setVoice(voiceId);
      // The engine has one sequence slot: route its position/end events to whoever owns it.
      engine.onSeqPosition = (sec, playing) => {
        if (!lab?.auditioning) filePanel?.handlePosition(sec, playing);
      };
      engine.onSeqEnded = () => {
        if (lab?.auditioning) lab.handleEnded();
        else filePanel?.handleEnded();
      };
      lab?.engineReady(engine);
      spectrogram.attach(engine);
      filePanel?.engineReady(engine);
      audioStatus.textContent = `Audio on (${engine.context.sampleRate} Hz)`;
      startBtn.textContent = 'Audio running';
      window.setTimeout(() => {
        if (engine) latencyLabel.textContent = `Output latency ≈ ${engine.latencyMs().toFixed(0)} ms (browser estimate)`;
      }, 500);
      requestAnimationFrame(function meter() {
        if (engine) meterFill.style.width = `${Math.min(100, engine.peakLevel() * 140)}%`;
        if (bus && bus.overPeak() >= 0.999) clipUntil = window.performance.now() + 900; // would have clipped: show it
        meterBox.classList.toggle('clip', window.performance.now() < clipUntil);
        requestAnimationFrame(meter);
      });
      (window as unknown as { __hillsing?: unknown }).__hillsing = {
        peakLevel: () => engine?.peakLevel() ?? 0,
        // For automated checks: feed a raw MIDI message as if it came from a keyboard, and estimate the output's pitch (Hz).
        midi: (bytes: number[]): void => {
          showMidi(bytes);
          const ev = parseMidiMessage(bytes);
          if (ev) handleEvent(ev);
        },
        pitchHz: (): number => {
          const buf = new Float32Array(bus.waveformTailLength);
          bus.waveformTail(buf);
          const sr = bus.ctx.sampleRate;
          let best = 0;
          let bestLag = 0;
          for (let lag = Math.floor(sr / 900); lag < Math.min(Math.floor(sr / 70), buf.length >> 1); lag++) {
            let c = 0;
            for (let i = 0; i + lag < buf.length; i++) c += buf[i] * buf[i + lag];
            if (c > best) {
              best = c;
              bestLag = lag;
            }
          }
          return bestLag ? sr / bestLag : 0;
        },
        // For automated checks: the effect settings, and the output's average level in a frequency band (dB).
        fx: fxModel,
        bandDb: (lo: number, hi: number): number => {
          if (!engine) return -100;
          const bins = new Uint8Array(engine.spectrumBins);
          engine.spectrum(bins);
          const nyq = engine.context.sampleRate / 2;
          const a = Math.floor((lo / nyq) * bins.length), b = Math.max(a + 1, Math.ceil((hi / nyq) * bins.length));
          let sum = 0;
          for (let i = a; i < b; i++) sum += bins[i];
          return -100 + ((sum / (b - a)) / 255) * 80;
        },
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
    utau: utauSession,
    tuning,
    syllables: () => pointer.current.syllables,
    consonantScale: () => paramValues.get('consonantScale') ?? 0.7,
    ensureEngine: async () => {
      await startAudio();
      return engine;
    },
    beforePlay: () => lab?.cancel(),
    lyricsText: () => lyricsBox.value,
    setLyricsText: (text) => {
      lyricsBox.value = text;
      try {
        storage?.setItem(LYRICS_KEY, text);
      } catch {
        /* storage unavailable: fine */
      }
      rebuildLyrics(true);
    },
    lookup: () => lookup,
  });

  for (const [id, text] of [['text-sounds', ALL_SOUNDS_TEXT], ['text-sentence', SENTENCE_TEXT]] as const) {
    const b = $<HTMLButtonElement>(id);
    b.addEventListener('click', () => {
      lyricsBox.value = text;
      lyricsBox.dispatchEvent(new Event('input', { bubbles: true }));
      navActions.reset();
      b.blur();
    });
  }

  new UtauPanel(utauSession);
  new TuningPanel($('tune-grid'), $<HTMLButtonElement>('tune-reset'), tuning);
  // Listening switches: tick one or more parts of a note to hear only those; "still vowel" holds the vowel steady.
  const solo = { attack: $<HTMLInputElement>('diag-attack'), sustain: $<HTMLInputElement>('diag-sustain'), end: $<HTMLInputElement>('diag-end') };
  const still = $<HTMLInputElement>('diag-still');
  const applyDiag = (): void => {
    const any = solo.attack.checked || solo.sustain.checked || solo.end.checked;
    diag.muteAttack = any && !solo.attack.checked;
    diag.muteSustain = any && !solo.sustain.checked;
    diag.muteEnd = any && !solo.end.checked;
    diag.stillSustain = still.checked;
  };
  for (const c of [solo.attack, solo.sustain, solo.end, still]) {
    c.addEventListener('change', () => {
      applyDiag();
      c.blur();
    });
  }
  const tuneNote = $('tune-note');
  const showTuneNote = (): void => {
    tuneNote.textContent = utauSession.active ? '' : 'These tune the sample-based voice: pick a voicebank in the Voice menu to hear them.';
  };
  utauSession.onChange(showTuneNote);
  showTuneNote();
  tuning.onChange((id) => {
    if (id === null || id === 'volume') utauLive?.refreshVolume();
  });

  const spectrogram = new Spectrogram($<HTMLCanvasElement>('spectrogram'));
  lab = new VoiceLab({
    ensureEngine: async () => {
      await startAudio();
      return engine;
    },
    lookup: () => lookup,
    consonantScale: () => paramValues.get('consonantScale') ?? 0.7,
    centerNote: () => getVoice(voiceId).centerNote,
    restoreFileSequence: () => filePanel?.settingsChanged(),
    settingsText: () =>
      JSON.stringify(
        {
          hillsing: '0.3',
          voice: voiceId,
          source: utauSession.active ? `utau:${utauSession.bank?.name}` : 'formant',
          effects: fxModel.state,
          ...Object.fromEntries([...paramValues].map(([k, v]) => [k, Math.round(v * 1000) / 1000])),
        },
        null,
        2,
      ),
  });

  // ---- Input: MIDI + computer keyboard ---------------------------------------------
  const bendRange = new BendRangeTracker();
  function handleEvent(ev: MidiEvent): void {
    // A knob mapped to pitch or modulation (or being learned for one) is ours; so is a binding for lyric navigation.
    const control = controls.handle(ev);
    if (control) {
      if (control.target === 'pitch') performance.pitchBend(control.value);
      else {
        engine?.setParam('vibratoDepth', control.value);
        utauLive?.setModulation(control.value);
      }
      return;
    }
    if (ev.type === 'cc' && controls.armedTarget) return; // waiting for a knob: do not let the press do anything else
    if (bindings.handle(ev, (a) => navActions[a]())) return;
    switch (ev.type) {
      case 'noteOn':
        if (!engine) {
          audioStatus.textContent = 'Click "Start audio" first (browsers require a click).';
          return;
        }
        lastNoteOn = ev.note;
        if (pointer.atEnd && !performance.hold && !pointer.wrap && pointer.length > 0) {
          // Every key is silent at the end of the text: say why, instead of leaving it looking broken.
          audioStatus.textContent = 'End of the lyrics: press ⏮ Start (Home) to sing from the beginning, or tick Loop.';
          for (const id of ['nav-reset', 'piano-start']) $(id).classList.add('nudge');
          window.setTimeout(() => {
            for (const id of ['nav-reset', 'piano-start']) $(id).classList.remove('nudge');
          }, 1500);
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
      case 'cc': {
        if (ev.controller === 1) {
          // Mod wheel or modulation knob: vibrato, on whichever voice is sounding.
          engine?.setParam('vibratoDepth', ev.value / 127);
          utauLive?.setModulation(ev.value / 127);
        } else if (ev.controller === 64) performance.sustain(ev.value >= 64);
        const range = bendRange.handle(ev.controller, ev.value);
        if (range !== null) performance.bendRange = range;
        break;
      }
    }
  }

  // ---- MIDI panel: a monitor of what the keyboard sends, and knobs mapped to pitch or modulation ----------------
  const BEND_KEY = 'hillsing.bendRange.v1';
  const controls = new ControlMap(storage);
  const midiPanel = $('midi-panel');
  const midiToggle = $<HTMLButtonElement>('midi-toggle');
  const midiLast = $('midi-last');
  const bendInput = $<HTMLInputElement>('bend-range');
  let midiShown = 0;
  let midiText = '';
  /** Show a raw MIDI message from the keyboard, and what the app made of it. */
  function showMidi(data: ArrayLike<number>): void {
    const ev = parseMidiMessage(data);
    midiShown++;
    midiText = `#${midiShown}: ${describeRaw(data)}${ev ? ` → ${describeMidi(ev)}` : ' → not used'}`;
    if (midiPending) return;
    midiPending = true;
    window.requestAnimationFrame(() => {
      midiPending = false;
      midiLast.textContent = midiText;
    });
  }
  let midiPending = false;
  function renderControls(): void {
    for (const t of ['pitch', 'mod'] as const) {
      const c = controls.controllerFor(t);
      $(`ctl-${t}-state`).textContent = controls.armedTarget === t
        ? 'move the knob now…'
        : c !== undefined ? `knob CC ${c}` : t === 'pitch' ? 'pitch wheel (standard pitch bend)' : 'mod wheel (CC 1)';
      const learn = $<HTMLButtonElement>(`ctl-${t}-learn`);
      learn.textContent = controls.armedTarget === t ? 'Cancel' : 'Learn';
      learn.classList.toggle('armed', controls.armedTarget === t);
      $<HTMLButtonElement>(`ctl-${t}-clear`).disabled = c === undefined;
    }
  }
  for (const t of ['pitch', 'mod'] as const) {
    $(`ctl-${t}-learn`).addEventListener('click', () => {
      controls.learn(controls.armedTarget === t ? null : t);
      $(`ctl-${t}-learn`).blur();
    });
    $(`ctl-${t}-clear`).addEventListener('click', () => {
      controls.clear(t);
      $(`ctl-${t}-clear`).blur();
    });
  }
  controls.onChange = renderControls;
  renderControls();
  const savedBend = Number(storage?.getItem(BEND_KEY));
  if (Number.isFinite(savedBend) && savedBend >= 1 && savedBend <= 24) {
    performance.bendRange = savedBend;
    bendInput.value = String(savedBend);
  }
  bendInput.addEventListener('input', () => {
    const v = Number(bendInput.value);
    if (!(v >= 1 && v <= 24)) return;
    performance.bendRange = v;
    try {
      storage?.setItem(BEND_KEY, String(v));
    } catch {
      /* storage unavailable: fine */
    }
  });
  bendInput.addEventListener('keydown', (e) => e.stopPropagation()); // typing a number must not play notes
  midiToggle.addEventListener('click', () => {
    midiPanel.hidden = !midiPanel.hidden;
    midiToggle.setAttribute('aria-expanded', String(!midiPanel.hidden));
    midiToggle.blur();
  });

  const midiStatus = $('midi-status');
  connectMidi(handleEvent, (names) => {
    midiStatus.textContent = names.length ? `MIDI: ${names.join(', ')}` : 'MIDI: no input devices found';
    $('midi-inputs').textContent = names.length ? names.join(', ') : 'none found: is the keyboard connected and switched on?';
  }, showMidi).catch((err: unknown) => {
    midiStatus.textContent = `MIDI unavailable (${err instanceof Error ? err.message : String(err)}); use the computer keyboard`;
  });

  let octave = 0;
  const octaveLabel = $('octave');
  const showOctave = (): void => {
    octaveLabel.textContent = `C${3 + octave}`;
  };
  showOctave();

  // ---- Views: Studio (everything) and Perform (voice, lyrics, syllables and effects on one screen) ----
  type View = 'studio' | 'perform';
  const VIEW_KEY = 'hillsing.view.v1';
  function setView(view: View, remember = true): void {
    document.body.dataset.view = view;
    for (const v of ['studio', 'perform'] as const) {
      const tab = $(`tab-${v}`);
      tab.setAttribute('aria-selected', String(v === view));
      tab.tabIndex = v === view ? 0 : -1;
    }
    if (remember) {
      try {
        storage?.setItem(VIEW_KEY, view);
      } catch {
        /* fine */
      }
      if (location.hash !== `#${view}`) history.replaceState(null, '', `#${view}`);
    }
    // Canvases that were hidden have no size: let them redraw now that they are shown.
    requestAnimationFrame(() => window.dispatchEvent(new Event('resize')));
    updatePointerView();
  }
  $('tab-studio').addEventListener('click', () => setView('studio'));
  $('tab-perform').addEventListener('click', () => setView('perform'));
  for (const id of ['tab-studio', 'tab-perform']) {
    $(id).addEventListener('keydown', (e) => {
      if (e.code === 'ArrowLeft' || e.code === 'ArrowRight') {
        e.preventDefault();
        const next = id === 'tab-studio' ? 'perform' : 'studio';
        setView(next);
        $(`tab-${next}`).focus();
      }
    });
  }
  window.addEventListener('hashchange', () => {
    const h = location.hash.slice(1);
    if (h === 'studio' || h === 'perform') setView(h, false);
  });
  const hashView = location.hash.slice(1);
  let savedView: string | null = null;
  try {
    savedView = storage?.getItem(VIEW_KEY) ?? null;
  } catch {
    /* fine */
  }
  setView(hashView === 'studio' || hashView === 'perform' ? hashView : savedView === 'perform' ? 'perform' : 'studio', false);

  // ---- Voice selection ---------------------------------------------------------------
  const voiceSelect = $<HTMLSelectElement>('voice');
  for (const v of VOICES) {
    const o = document.createElement('option');
    o.value = v.id;
    o.textContent = `${v.name} — ${v.model}`;
    voiceSelect.append(o);
  }
  function applyVoice(id: string): void {
    const v = getVoice(id);
    voiceId = v.id;
    voiceSelect.value = v.id;
    $('voice-desc').textContent = v.description;
    syncMainVoice();
    engine?.setVoice(v.id);
    try {
      storage?.setItem(VOICE_KEY, v.id);
    } catch {
      /* storage unavailable: fine */
    }
    // Put the computer-keyboard octave in this voice's comfortable range.
    octave = Math.max(-2, Math.min(3, Math.floor((v.centerNote - BASE_NOTE) / 12)));
    showOctave();
  }
  // Blur after a choice (so computer keys play again), not on pointerup: that closes the popup
  // the moment it opens.
  voiceSelect.addEventListener('change', () => {
    applyVoice(voiceSelect.value);
    voiceSelect.blur();
  });

  // ---- Voice chooser next to the lyrics: the generated formant voices and the sample-based voicebank ----
  const mainVoice = $<HTMLSelectElement>('voice-main');
  function syncMainVoice(): void {
    mainVoice.value = utauSession.active ? 'utau' : voiceId;
    $('voice-main-desc').textContent = utauSession.active
      ? `Sample-based: live keys and melodies (MIDI file or piano roll) are sung with this voicebank. The Voice lab and its sliders still use the formant voice ${getVoice(voiceId).name}.`
      : getVoice(voiceId).description;
  }
  function fillMainVoice(): void {
    mainVoice.textContent = '';
    const formant = document.createElement('optgroup');
    formant.label = 'Formant synth (generated voices)';
    for (const v of VOICES) {
      const o = document.createElement('option');
      o.value = v.id;
      o.textContent = `${v.name} — ${v.model}`;
      formant.append(o);
    }
    const sample = document.createElement('optgroup');
    sample.label = 'Sample-based (UTAU voicebank)';
    const u = document.createElement('option');
    u.value = 'utau';
    u.textContent = utauSession.bank ? utauSession.bank.name : 'Load a UTAU voicebank…';
    sample.append(u);
    mainVoice.append(formant, sample);
    syncMainVoice();
  }
  // A folder picker is for desktop browsers; phones and tablets load a .zip.
  const canPickFolder = !window.matchMedia('(pointer: coarse)').matches && 'webkitdirectory' in document.createElement('input');
  $<HTMLButtonElement>('voice-load-folder').hidden = !canPickFolder;
  $('voice-load-zip').addEventListener('click', () => $<HTMLInputElement>('voice-zip').click());
  $('voice-load-folder').addEventListener('click', () => $<HTMLInputElement>('voice-folder').click());
  mainVoice.addEventListener('change', () => {
    const choice = mainVoice.value;
    if (choice === 'utau') {
      if (utauSession.bank) utauSession.setUse(true);
      else {
        syncMainVoice(); // keep the current voice until a voicebank is actually loaded
        (canPickFolder ? $<HTMLInputElement>('voice-folder') : $<HTMLInputElement>('voice-zip')).click();
      }
    } else {
      utauSession.setUse(false);
      applyVoice(choice);
    }
    mainVoice.blur();
  });
  utauSession.onChange(() => {
    // Switching between the voicebank and the formant voices: let go of anything still sounding.
    performance.allNotesOff();
    utauLive?.stop();
    engine?.stop();
    fillMainVoice();
  });
  applyVoice(voiceId);
  fillMainVoice();

  // ---- On-screen piano: for phones and tablets, or anywhere with no keyboard; shown by default on touch screens ----
  const PIANO_KEY = 'hillsing.piano.v1';
  const pianoBar = $('piano');
  const pianoToggle = $<HTMLButtonElement>('piano-toggle');
  const piano = new OnScreenPiano(
    $('piano-keys'),
    {
      noteOn: (note) => {
        // iPhones only let audio resume from inside a touch: do it here, in the gesture, not after any waiting.
        if (engine && engine.context.state !== 'running') void engine.context.resume();
        if (!engine) {
          void startAudio(); // this touch is the user gesture; the next one will sound
          audioStatus.textContent = 'Audio starting: tap a key again.';
          return;
        }
        handleEvent({ type: 'noteOn', note, velocity: 0.8 });
      },
      noteOff: (note) => handleEvent({ type: 'noteOff', note }),
    },
    60,
    (label) => {
      $('piano-range').textContent = label;
    },
  );
  $('piano-down').addEventListener('click', () => piano.shiftOctave(-1));
  $('piano-up').addEventListener('click', () => piano.shiftOctave(1));
  $('piano-start').addEventListener('click', () => navActions.reset());
  $('piano-back').addEventListener('click', () => navActions.back());
  $('piano-fwd').addEventListener('click', () => navActions.forward());
  const setPiano = (on: boolean, remember = true): void => {
    pianoBar.hidden = !on;
    document.body.classList.toggle('piano-on', on);
    pianoToggle.setAttribute('aria-pressed', String(on));
    if (!on) piano.allOff();
    if (remember) {
      try {
        storage?.setItem(PIANO_KEY, on ? '1' : '0');
      } catch {
        /* storage unavailable: fine */
      }
    }
  };
  pianoToggle.addEventListener('click', () => {
    setPiano(pianoBar.hidden === true);
    pianoToggle.blur();
  });
  // Inside a frame that is as tall as the whole page (a viewer may embed it that way) "fixed to the bottom" would pin the
  // piano below everything, so there it follows the bottom of what is on screen instead. A frame that scrolls on its own
  // keeps ordinary fixed positioning.
  const framed = (() => {
    try {
      return window.self !== window.top;
    } catch {
      return true;
    }
  })();
  if (framed) {
    const pin = new ScreenBottomPin(pianoBar);
    const check = (): void => {
      const grownToContent = window.innerHeight >= document.documentElement.scrollHeight - 200 && !pianoBar.hidden; // the frame is about as tall as its content
      pin.setActive(grownToContent && document.documentElement.scrollHeight > 1.5 * pianoBar.offsetHeight);
    };
    window.setInterval(check, 800);
    window.addEventListener('resize', check);
    check();
  }
  // Keep the page's bottom clear of the piano, whatever size it ends up.
  new ResizeObserver(() => document.body.style.setProperty('--piano-h', `${pianoBar.offsetHeight + 8}px`)).observe(pianoBar);
  const savedPiano = storage?.getItem(PIANO_KEY);
  setPiano(savedPiano === null || savedPiano === undefined ? window.matchMedia('(pointer: coarse)').matches || window.innerWidth <= 760 : savedPiano === '1', false);

  const pressed = new Map<string, number>(); // key code -> note (so octave changes don't strand notes)

  // A dropdown keeps keyboard focus after you pick from it, and then swallows the arrow keys: let go of it.
  document.addEventListener('change', (e) => {
    if (e.target instanceof HTMLSelectElement) e.target.blur();
  });

  const isTyping = (t: EventTarget | null): boolean =>
    t instanceof HTMLTextAreaElement || t instanceof HTMLSelectElement ||
    (t instanceof HTMLInputElement && t.type !== 'checkbox');

  // Caps Lock locks on the current syllable (the same as the Hold button) until it is switched off.
  let capsLock = false;
  const syncCaps = (e: KeyboardEvent): void => {
    const on = typeof e.getModifierState === 'function' && e.getModifierState('CapsLock');
    if (on === capsLock) return;
    capsLock = on;
    setHold(on);
  };

  window.addEventListener('keydown', (e) => {
    syncCaps(e);
    shiftDown = e.shiftKey;
    updateMelisma();
    const arrow = e.code === 'ArrowLeft' || e.code === 'ArrowRight';
    if (isTyping(e.target) || e.ctrlKey || e.metaKey || (e.altKey && !arrow)) return;
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
    } else if (e.code === 'KeyQ') {
      if (!e.repeat) flagArtifact();
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
    } else if (e.code === 'Tab') {
      // Tab steps back one syllable (Shift+Tab too). Typing in a text box is unaffected: that returns above.
      navActions.back();
      e.preventDefault();
    } else if (e.code === 'ArrowLeft') {
      // Arrow keys step by syllable, Shift or not (Shift is for melisma); Alt/Option+arrow jumps a whole word.
      (e.altKey ? navActions.wordBack : navActions.back)();
      e.preventDefault();
    } else if (e.code === 'ArrowRight') {
      (e.altKey ? navActions.wordForward : navActions.forward)();
      e.preventDefault();
    } else if (e.code === 'Backquote') {
      navActions.hold();
      e.preventDefault();
    } else if (e.code === 'Home') {
      navActions.reset();
      e.preventDefault();
    }
  });
  window.addEventListener('keyup', (e) => {
    syncCaps(e);
    shiftDown = e.shiftKey;
    updateMelisma();
    const note = pressed.get(e.code);
    if (note === undefined) return;
    pressed.delete(e.code);
    handleEvent({ type: 'noteOff', note });
  });
  window.addEventListener('blur', () => {
    shiftDown = false;
    melismaHeld.clear();
    updateMelisma();
    pressed.clear();
    performance.allNotesOff();
  });
}
