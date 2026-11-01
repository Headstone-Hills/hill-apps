import { buildSequence, reduceToMonophonic, type Sequence } from '../engine/sequence';
import { renameSyllable, toggleMelisma } from '../editor/lyricEdit';
import { History, sameNotes, withIds, type EditNote } from '../editor/notes';
import { parseMidiFile, singableTracks, type MidiFile, type MidiNote, type MidiTrack } from '../midi/smf';
import type { SingerEngine } from '../synth/engine';
import type { PhonemeLookup } from '../text/cmudict';
import type { LyricSyllable } from '../text/lyrics';
import { renderUtau, type UtauRender } from '../utau/render';
import type { UtauSession } from '../utau/session';
import { PianoRoll, type LaneCell } from './pianoroll';
import { UtauPlayer } from './utauplayer';
import type { TuningModel } from '../utau/tuning';

export interface PanelContext {
  syllables(): LyricSyllable[];
  consonantScale(): number;
  /** Start audio if needed (must be reachable from a user gesture). */
  ensureEngine(): Promise<SingerEngine | null>;
  /** Called before playback starts (e.g. to stop a voice-lab phrase that owns the engine). */
  beforePlay?(): void;
  /** The lyric text (the editor changes words and melismas by rewriting it). */
  lyricsText(): string;
  setLyricsText(text: string): void;
  lookup(): PhonemeLookup;
  /** The loaded UTAU voicebank, if any. */
  utau: UtauSession;
  tuning: TuningModel;
}

interface Snapshot {
  notes: EditNote[];
  lyrics: string;
}

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
const noteName = (n: number): string => `${NOTE_NAMES[n % 12]}${Math.floor(n / 12) - 1}`;
/** m:ss.s (tenths matter when lining a melody up against lyrics). */
const clock = (s: number): string => {
  const t = Math.max(0, s);
  const m = Math.floor(t / 60);
  return `${m}:${(t - m * 60).toFixed(1).padStart(4, '0')}`;
};

/**
 * Melody panel: load a MIDI file (or start an empty melody), edit it in the piano roll, and play it
 * through the synth. Edits are made on a copy of the notes (with undo); the file itself is untouched.
 */
export class MidiFilePanel {
  private file: MidiFile | null = null;
  private track: MidiTrack | null = null;
  private notes: EditNote[] = [];
  /** Undo snapshots: the notes and the lyric text they were edited with. */
  private readonly history = new History<Snapshot>({ notes: [], lyrics: '' }, (a, b) => a.lyrics === b.lyrics && sameNotes(a.notes, b.notes));
  private roll!: PianoRoll;
  private utauPlayer!: UtauPlayer;
  private utauRender: UtauRender | null = null;
  private utauDirty = true;
  private rendering = false;
  private bpm = 120;
  private seq: Sequence | null = null;
  private engine: SingerEngine | null = null;
  private playing = false;
  private scorePos = 0; // seconds of score time (can be < 0 during the consonant lead-in)
  private lastRate = 1;

  private readonly el = {
    file: this.q<HTMLInputElement>('file'),
    name: this.q('file-name'),
    track: this.q<HTMLSelectElement>('track'),
    play: this.q<HTMLButtonElement>('play'),
    stop: this.q<HTMLButtonElement>('stop'),
    time: this.q('time'),
    loop: this.q<HTMLInputElement>('loop-file'),
    beat: this.q<HTMLInputElement>('beat'),
    repeat: this.q<HTMLInputElement>('repeat'),
    tempo: this.q<HTMLInputElement>('tempo'),
    tempoOut: this.q('out-tempo'),
    canvas: this.q<HTMLCanvasElement>('preview'),
    summary: this.q('file-summary'),
    utauLog: this.q('utau-log'),
    rollWrap: this.q('roll-wrap'),
    rollScroll: this.q<HTMLInputElement>('roll-scroll'),
    undo: this.q<HTMLButtonElement>('roll-undo'),
    redo: this.q<HTMLButtonElement>('roll-redo'),
    del: this.q<HTMLButtonElement>('roll-delete'),
    mel: this.q<HTMLButtonElement>('roll-melisma'),
    quant: this.q<HTMLButtonElement>('roll-quantize'),
    grid: this.q<HTMLSelectElement>('roll-grid'),
    zoomIn: this.q<HTMLButtonElement>('roll-zoom-in'),
    zoomOut: this.q<HTMLButtonElement>('roll-zoom-out'),
    fit: this.q<HTMLButtonElement>('roll-fit'),
    fresh: this.q<HTMLButtonElement>('roll-new'),
  };

  constructor(private readonly ctx: PanelContext) {
    const { el } = this;
    el.file.addEventListener('change', () => void this.loadFile());
    el.track.addEventListener('change', () => {
      this.track = this.file?.tracks[Number(el.track.value)] ?? null;
      this.setNotes(this.track?.notes ?? []);
      this.rebuild();
    });
    for (const c of [el.beat, el.repeat]) c.addEventListener('change', () => this.rebuild());
    el.loop.addEventListener('change', () => {
      this.engine?.seqLoop(el.loop.checked);
      this.utauPlayer.loop = el.loop.checked;
    });
    this.utauPlayer = new UtauPlayer(
      (sec) => {
        this.playing = true;
        this.scorePos = sec - (this.utauRender?.leadSec ?? 0);
        this.refresh();
      },
      () => {
        this.playing = false;
        this.scorePos = 0;
        this.refresh();
      },
    );
    this.ctx.tuning.onChange(() => {
      this.utauDirty = true; // the next Play renders again with the new tuning
    });
    let wasActive = this.ctx.utau.active;
    this.ctx.utau.onChange(() => {
      this.utauDirty = true; // a different voicebank, or a switch between the voicebank and the formant voices
      const now = this.ctx.utau.active;
      if (now === wasActive) return;
      wasActive = now;
      this.stopAll();
      this.say(now ? 'Singing melodies with the UTAU voicebank: the next Play renders them with it.' : 'Singing melodies with the formant synth.', false);
    });
    el.tempo.addEventListener('input', () => {
      el.tempoOut.textContent = `${el.tempo.value}%`;
      this.rebuild();
    });
    el.tempo.addEventListener('pointerup', () => el.tempo.blur());
    el.play.addEventListener('click', () => void this.togglePlay());
    el.stop.addEventListener('click', () => this.stop());
    this.roll = new PianoRoll(el.canvas, el.rollScroll, el.rollWrap, {
      commit: (notes) => this.applyEdit(notes),
      seek: (t) => this.seekFile(t),
      toggleMelisma: (id) => this.toggleMelismaFor(id),
      renameSyllable: (id, text) => this.renameFor(id, text),
      undo: () => this.undo(),
      redo: () => this.redo(),
      selectionChanged: () => this.updateTools(),
    });
    el.undo.addEventListener('click', () => this.undo());
    el.redo.addEventListener('click', () => this.redo());
    el.del.addEventListener('click', () => this.roll.deleteSelected());
    el.mel.addEventListener('click', () => this.roll.toggleMelismaOfSelection());
    el.quant.addEventListener('click', () => this.roll.quantize());
    el.grid.addEventListener('change', () => this.applyGrid());
    el.zoomIn.addEventListener('click', () => this.roll.zoomBy(1.25));
    el.zoomOut.addEventListener('click', () => this.roll.zoomBy(0.8));
    el.fit.addEventListener('click', () => this.roll.fit());
    el.fresh.addEventListener('click', () => this.newMelody());
    for (const b of [el.undo, el.redo, el.del, el.mel, el.quant, el.zoomIn, el.zoomOut, el.fit, el.fresh]) {
      b.addEventListener('pointerup', () => b.blur()); // so Space / computer keys keep working
    }
    el.grid.addEventListener('pointerup', () => el.grid.blur());
    this.applyGrid();
    this.updateTools();
    el.tempoOut.textContent = `${el.tempo.value}%`;
    this.setEnabled(false);
    this.refresh();
  }

  private q<T extends HTMLElement = HTMLElement>(id: string): T {
    const e = document.getElementById(id);
    if (!e) throw new Error(`Missing element #${id}`);
    return e as T;
  }

  // ---- hooks called by the app ---------------------------------------------------
  engineReady(engine: SingerEngine): void {
    this.engine = engine;
    this.rebuild();
  }
  /** Sequence position from the engine (the app routes it here when the file owns the engine). */
  handlePosition(sec: number, playing: boolean): void {
    this.playing = playing;
    this.scorePos = sec - (this.seq?.leadInSec ?? 0);
    this.refresh();
  }
  handleEnded(): void {
    this.playing = false;
    this.refresh();
  }
  lyricsChanged(): void {
    this.rebuild();
  }
  settingsChanged(): void {
    this.rebuild();
  }

  /** True when the melody should be sung by the voicebank and one is loaded. */
  private usingUtau(): boolean {
    return this.ctx.utau.active;
  }

  private stopAll(): void {
    this.engine?.seqStop();
    this.utauPlayer.stop();
    this.playing = false;
    this.scorePos = 0;
    this.refresh();
  }

  /** Render the melody with the voicebank if the score or settings changed since the last render. */
  private async ensureUtauRender(): Promise<boolean> {
    const bank = this.ctx.utau.bank;
    if (!bank || !this.seq) return false;
    if (!this.utauDirty && this.utauRender) return true;
    if (this.rendering) return false;
    this.rendering = true;
    this.say('Rendering with the voicebank…', false);
    try {
      const r = await renderUtau(bank, this.seq.notes, this.ctx.syllables(), this.ctx.tuning.values);
      this.utauRender = r;
      this.utauPlayer.setAudio(r.samples, r.sampleRate);
      this.utauPlayer.loop = this.el.loop.checked;
      this.utauDirty = false;
      this.el.utauLog.textContent = r.log.join('\n') + (r.missing.length ? `\n\nNo sample found for: ${r.missing.join(', ')}` : '');
      this.say(`Rendered ${(r.samples.length / r.sampleRate).toFixed(1)} s with ${bank.name}.` + (r.missing.length ? ` ${r.missing.length} sounds had no sample (see the alias list).` : ''), r.missing.length > 0);
      return true;
    } catch (err) {
      this.say(`Rendering failed: ${err instanceof Error ? err.message : String(err)}`, true);
      return false;
    } finally {
      this.rendering = false;
    }
  }

  private async toggleUtau(): Promise<void> {
    this.ctx.beforePlay?.();
    this.engine?.seqStop();
    if (this.utauPlayer.isPlaying) {
      this.utauPlayer.pause();
      this.playing = false;
      this.refresh();
      return;
    }
    if (!(await this.ensureUtauRender())) return;
    this.utauPlayer.level = this.ctx.tuning.values.volume;
    await this.utauPlayer.play(Math.max(0, this.scorePos) + (this.utauRender?.leadSec ?? 0));
    this.playing = true;
    this.refresh();
  }

  async togglePlay(): Promise<void> {
    if (!this.seq) return;
    if (this.usingUtau()) {
      await this.toggleUtau();
      return;
    }
    this.utauPlayer.stop();
    this.ctx.beforePlay?.();
    const engine = this.engine ?? (await this.ctx.ensureEngine());
    if (!engine) return;
    if (this.playing) {
      engine.seqPause();
      this.playing = false;
    } else {
      engine.seqPlay();
      this.playing = true;
    }
    this.refresh();
  }

  private stop(): void {
    this.stopAll();
  }

  // ---- loading and mapping -----------------------------------------------------------
  private async loadFile(): Promise<void> {
    const f = this.el.file.files?.[0];
    if (!f) return;
    this.utauPlayer.stop();
    this.engine?.seqStop();
    this.playing = false;
    this.scorePos = 0;
    try {
      this.file = parseMidiFile(new Uint8Array(await f.arrayBuffer()));
    } catch (err) {
      this.file = null;
      this.track = null;
      this.seq = null;
      this.el.name.textContent = '';
      this.setEnabled(false);
      this.fillTracks();
      this.say(`Could not read "${f.name}": ${err instanceof Error ? err.message : String(err)}`, true);
      this.refresh();
      return;
    }
    this.bpm = this.file.initialBpm || 120;
    this.applyGrid();
    this.el.name.textContent = `— ${f.name} (${this.file.initialBpm.toFixed(0)} BPM${this.file.tempoEvents > 1 ? ', tempo changes' : ''})`;
    const candidates = singableTracks(this.file);
    if (candidates.length === 0) {
      this.track = null;
      this.setNotes([]);
      this.fillTracks();
      this.say('This file has no pitched notes to sing (only percussion or empty tracks).', true);
      this.setEnabled(false);
      this.seq = null;
      this.refresh();
      return;
    }
    // Default to the busiest pitched track: usually the melody.
    this.track = candidates.reduce((a, b) => (b.notes.length > a.notes.length ? b : a));
    this.fillTracks();
    this.el.track.value = String(this.track.index);
    this.lastRate = 1;
    this.setNotes(this.track.notes);
    this.rebuild();
    this.roll.fit();
  }

  // ---- editing -----------------------------------------------------------------------------------
  /** Replace the score with a fresh copy of these notes (loading a file / choosing a track): clears undo. */
  private setNotes(notes: ReadonlyArray<MidiNote>): void {
    this.notes = withIds(notes);
    this.history.reset({ notes: this.notes, lyrics: this.ctx.lyricsText() });
    this.roll.select([]);
    this.updateTools();
  }

  private newMelody(): void {
    this.utauPlayer.stop();
    this.engine?.seqStop();
    this.playing = false;
    this.scorePos = 0;
    this.file = null;
    this.track = null;
    this.bpm = 120;
    this.el.name.textContent = '— new melody';
    this.fillTracks();
    this.applyGrid();
    this.setNotes([]);
    this.rebuild();
    this.say('Double-click in the piano roll to add notes. Each note sings the next syllable of the lyrics.', false);
  }

  private applyEdit(notes: EditNote[]): void {
    this.history.current.lyrics = this.ctx.lyricsText(); // typed lyric changes since the last edit are not part of this one
    if (this.history.push({ notes, lyrics: this.ctx.lyricsText() })) this.notes = notes;
    this.rebuild();
    this.updateTools();
  }

  /** A lyric edit made from the roll (word change, melisma): recorded in the same undo history. */
  private applyLyricEdit(next: string): void {
    this.history.current.lyrics = this.ctx.lyricsText();
    this.history.push({ notes: this.notes, lyrics: next });
    this.ctx.setLyricsText(next); // rebuilds the lyrics, which rebuilds the score
    this.updateTools();
  }

  private restore(s: Snapshot | null): void {
    if (!s) return;
    this.notes = s.notes;
    if (s.lyrics !== this.ctx.lyricsText()) this.ctx.setLyricsText(s.lyrics);
    else this.rebuild();
    this.updateTools();
  }

  private undo(): void {
    this.restore(this.history.undo());
  }

  private redo(): void {
    this.restore(this.history.redo());
  }

  /** Snap step in seconds: the chosen fraction of a beat at the file's first tempo. */
  private applyGrid(): void {
    const frac = Number(this.el.grid.value);
    this.roll.setGrid(frac > 0 ? (frac * 60) / this.bpm : 0);
  }

  private updateTools(): void {
    const sel = this.roll?.selectedIds.size ?? 0;
    this.el.undo.disabled = !this.history.canUndo;
    this.el.redo.disabled = !this.history.canRedo;
    this.el.del.disabled = sel === 0;
    this.el.mel.disabled = sel !== 1;
    this.el.quant.disabled = Number(this.el.grid.value) <= 0 || this.notes.length === 0;
  }

  /** Index into the sung (monophonic) melody for a note, or -1 when the note is not sung (chord, hidden). */
  private monoIndex(id: number): number {
    return reduceToMonophonic(this.notes).findIndex((n) => (n as EditNote).id === id);
  }

  /** Which lyric syllable a melody index sings (-1 for none), following the "Repeat lyrics" rule. */
  private syllableFor(monoIdx: number): number {
    const len = this.ctx.syllables().length;
    if (monoIdx < 0 || len === 0) return -1;
    if (monoIdx < len) return monoIdx;
    return this.el.repeat.checked ? monoIdx % len : -1;
  }

  private toggleMelismaFor(id: number): void {
    const si = this.syllableFor(this.monoIndex(id));
    const next = si < 0 ? null : toggleMelisma(this.ctx.lyricsText(), this.ctx.lookup(), si);
    if (next === null) {
      this.say('That note cannot hold the previous vowel: the first note has nothing to continue, and a note in the middle of a word cannot be split.', true);
      return;
    }
    this.applyLyricEdit(next);
  }

  private renameFor(id: number, word: string): void {
    const mi = this.monoIndex(id);
    const len = this.ctx.syllables().length;
    const text = this.ctx.lyricsText();
    if (mi < 0) return;
    if (mi >= len) {
      if (mi > len) {
        this.say('Give the earlier notes a word first: words are assigned to notes in order.', true);
        return;
      }
      if (word.trim()) this.applyLyricEdit(`${text.trimEnd()} ${word.trim()}`);
      return;
    }
    const next = renameSyllable(text, this.ctx.lookup(), this.syllableFor(mi), word);
    if (next !== null) this.applyLyricEdit(next);
  }

  /** Click on the ruler / empty roll: move the playhead (file time -> score time). */
  private seekFile(fileSec: number): void {
    if (!this.seq) return;
    const mono = reduceToMonophonic(this.notes);
    const t0 = mono.length ? mono[0].start : 0;
    const t = Math.max(0, (fileSec - t0) / (Number(this.el.tempo.value) / 100));
    this.scorePos = t;
    if (this.usingUtau()) {
      if (this.utauPlayer.isPlaying) void this.utauPlayer.play(t + (this.utauRender?.leadSec ?? 0));
    } else {
      this.engine?.seqSeek(t + this.seq.leadInSec);
    }
    this.refresh();
  }

  private fillTracks(): void {
    const sel = this.el.track;
    sel.textContent = '';
    for (const t of this.file ? singableTracks(this.file) : []) {
      const lo = Math.min(...t.notes.map((n) => n.note));
      const hi = Math.max(...t.notes.map((n) => n.note));
      const opt = document.createElement('option');
      opt.value = String(t.index);
      opt.textContent = `${t.index + 1}. ${t.name || 'Track'} — ${t.notes.length} notes, ${noteName(lo)}–${noteName(hi)}, ch ${t.channels.map((c) => c + 1).join('/')}`;
      sel.append(opt);
    }
    sel.disabled = sel.options.length === 0;
  }

  private rebuild(): void {
    this.utauDirty = true; // the score, lyrics or tempo changed: the voicebank render is out of date
    if (this.utauPlayer.isPlaying) {
      this.utauPlayer.pause();
      this.playing = false;
    }
    const rate = Number(this.el.tempo.value) / 100;
    const keepScore = Math.max(0, this.scorePos) * (this.lastRate / rate); // same musical spot at the new tempo
    this.lastRate = rate;
    if (this.notes.length === 0) {
      this.seq = null;
      this.setEnabled(false);
      this.refresh();
      return;
    }
    const syllables = this.ctx.syllables();
    this.seq = buildSequence(this.notes, syllables, {
      rate,
      vowelOnBeat: this.el.beat.checked,
      repeatLyrics: this.el.repeat.checked,
      consonantScale: this.ctx.consonantScale(),
    });
    this.setEnabled(this.seq.commands.length > 0);
    this.scorePos = this.playing || keepScore > 0 ? keepScore : 0;

    const st = this.seq.stats;
    const parts = [`${st.sungNotes} of ${st.melodyNotes} notes sung (one syllable per note)`];
    if (st.syllables === 0) parts.push('type some lyrics to sing');
    else if (st.unsungNotes > 0) parts.push(`${st.unsungNotes} notes have no syllable: add lyrics or turn on "Repeat lyrics"`);
    if (st.unusedSyllables > 0) parts.push(`${st.unusedSyllables} syllables unused (melody is shorter than the lyrics)`);
    this.say(parts.join('. ') + '.', false);

    if (this.engine) {
      const start = this.scorePos > 0 ? this.scorePos + this.seq.leadInSec : 0;
      this.engine.seqLoad(this.seq.commands, this.seq.lengthSec, start);
      this.engine.seqLoop(this.el.loop.checked);
    }
    this.refresh();
  }

  // ---- view ---------------------------------------------------------------------------
  private previewLength(): number {
    const n = this.seq?.notes;
    return n && n.length ? n[n.length - 1].end + 0.25 : 1;
  }

  private setEnabled(on: boolean): void {
    this.el.play.disabled = !on;
    this.el.stop.disabled = !on;
  }

  private say(text: string, error: boolean): void {
    this.el.summary.textContent = text;
    this.el.summary.classList.toggle('error', error);
  }

  private refresh(): void {
    const { el } = this;
    el.play.textContent = this.playing ? '⏸ Pause' : '▶ Play';
    const len = this.previewLength();
    el.time.textContent = this.seq ? `${clock(Math.min(len, this.scorePos))} / ${clock(len)}` : '0:00.0 / 0:00.0';
    const syl = this.ctx.syllables();
    const mono = reduceToMonophonic(this.notes);
    const monoIdx = new Map(mono.map((n, i) => [(n as EditNote).id, i]));
    const lane = new Map<number, LaneCell>();
    for (const n of this.notes) {
      const mi = monoIdx.get(n.id);
      const si = mi === undefined ? -1 : this.syllableFor(mi);
      const s = si >= 0 ? syl[si] : undefined;
      lane.set(n.id, s ? { label: s.label, sung: true, melisma: !!s.melisma } : { label: '', sung: false, melisma: false });
    }
    const t0 = mono.length ? mono[0].start : 0;
    const rate = Number(el.tempo.value) / 100;
    const last = this.notes.reduce((m, n) => Math.max(m, n.end), 0);
    this.roll.setView({
      notes: this.notes,
      lane,
      playheadSec: this.seq ? t0 + this.scorePos * rate : null,
      playing: this.playing,
      lengthSec: Math.max(4, last + 1),
    });
  }
}
