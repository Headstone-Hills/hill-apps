import { buildSequence, type Sequence } from '../engine/sequence';
import { parseMidiFile, singableTracks, type MidiFile, type MidiTrack } from '../midi/smf';
import type { SingerEngine } from '../synth/engine';
import type { LyricSyllable } from '../text/lyrics';
import { drawPreview, timeAtClick } from './preview';

export interface PanelContext {
  syllables(): LyricSyllable[];
  consonantScale(): number;
  /** Start audio if needed (must be reachable from a user gesture). */
  ensureEngine(): Promise<SingerEngine | null>;
}

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
const noteName = (n: number): string => `${NOTE_NAMES[n % 12]}${Math.floor(n / 12) - 1}`;
/** m:ss.s (tenths matter when lining a melody up against lyrics). */
const clock = (s: number): string => {
  const t = Math.max(0, s);
  const m = Math.floor(t / 60);
  return `${m}:${(t - m * 60).toFixed(1).padStart(4, '0')}`;
};

/** MIDI file panel: load a file, choose the melody track, preview, play through the synth. */
export class MidiFilePanel {
  private file: MidiFile | null = null;
  private track: MidiTrack | null = null;
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
  };

  constructor(private readonly ctx: PanelContext) {
    const { el } = this;
    el.file.addEventListener('change', () => void this.loadFile());
    el.track.addEventListener('change', () => {
      this.track = this.file?.tracks[Number(el.track.value)] ?? null;
      this.rebuild();
    });
    for (const c of [el.beat, el.repeat]) c.addEventListener('change', () => this.rebuild());
    el.loop.addEventListener('change', () => this.engine?.seqLoop(el.loop.checked));
    el.tempo.addEventListener('input', () => {
      el.tempoOut.textContent = `${el.tempo.value}%`;
      this.rebuild();
    });
    el.tempo.addEventListener('pointerup', () => el.tempo.blur());
    el.play.addEventListener('click', () => void this.togglePlay());
    el.stop.addEventListener('click', () => this.stop());
    el.canvas.addEventListener('click', (e) => {
      if (!this.seq) return;
      const t = timeAtClick(el.canvas, e.clientX, this.previewLength());
      this.scorePos = t;
      this.engine?.seqSeek(t + this.seq.leadInSec);
      this.refresh();
    });
    window.addEventListener('resize', () => this.refresh());
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
    engine.onSeqPosition = (sec, playing) => {
      this.playing = playing;
      this.scorePos = sec - (this.seq?.leadInSec ?? 0);
      this.refresh();
    };
    engine.onSeqEnded = () => {
      this.playing = false;
      this.refresh();
    };
    this.rebuild();
  }
  lyricsChanged(): void {
    this.rebuild();
  }
  settingsChanged(): void {
    this.rebuild();
  }

  async togglePlay(): Promise<void> {
    if (!this.file || !this.seq) return;
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
    this.engine?.seqStop();
    this.playing = false;
    this.scorePos = 0;
    this.refresh();
  }

  // ---- loading and mapping -----------------------------------------------------------
  private async loadFile(): Promise<void> {
    const f = this.el.file.files?.[0];
    if (!f) return;
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
    this.el.name.textContent = `— ${f.name} (${this.file.initialBpm.toFixed(0)} BPM${this.file.tempoEvents > 1 ? ', tempo changes' : ''})`;
    const candidates = singableTracks(this.file);
    if (candidates.length === 0) {
      this.track = null;
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
    this.rebuild();
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
    const rate = Number(this.el.tempo.value) / 100;
    const keepScore = Math.max(0, this.scorePos) * (this.lastRate / rate); // same musical spot at the new tempo
    this.lastRate = rate;
    if (!this.track) {
      this.seq = null;
      this.refresh();
      return;
    }
    const syllables = this.ctx.syllables();
    this.seq = buildSequence(this.track.notes, syllables, {
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
    const labels = this.seq ? this.seq.notes.map((n) => this.ctx.syllables()[n.syllable]?.label ?? '') : [];
    drawPreview(el.canvas, this.seq ? { notes: this.seq.notes, labels, lengthSec: len } : null, this.scorePos);
  }
}
