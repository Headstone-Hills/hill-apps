// UTAU voicebank panel (stage 1): pick a voicebank folder, see what is in it, and audition any alias at a
// chosen pitch and length. The files are read in place by the browser; nothing is uploaded or copied.

import { getBus } from '../audio/bus';
import type { UtauSession } from '../utau/session';
import { Voicebank, type BankFile } from '../utau/voicebank';
import { openZip } from '../utau/zip';

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
const noteName = (n: number): string => `${NOTE_NAMES[((n % 12) + 12) % 12]}${Math.floor(n / 12) - 1}`;
const hzOf = (note: number): number => 440 * Math.pow(2, (note - 69) / 12);

export class UtauPanel {
  private bank: Voicebank | null = null;
  private source: AudioBufferSourceNode | null = null;

  private readonly el = {
    folder: this.q<HTMLInputElement>('utau-folder'),
    zip: this.q<HTMLInputElement>('utau-zip'),
    name: this.q('utau-name'),
    status: this.q('utau-status'),
    alias: this.q<HTMLInputElement>('utau-alias'),
    list: this.q<HTMLDataListElement>('utau-alias-list'),
    pitch: this.q<HTMLInputElement>('utau-pitch'),
    pitchName: this.q('utau-pitch-name'),
    length: this.q<HTMLInputElement>('utau-len'),
    play: this.q<HTMLButtonElement>('utau-play'),
    // The same pickers next to the voice menu, so a voicebank can be loaded from the Perform tab (and from a phone).
    voiceZip: document.getElementById('voice-zip') as HTMLInputElement | null,
    voiceFolder: document.getElementById('voice-folder') as HTMLInputElement | null,
    voiceStatus: document.getElementById('voice-load-status'),
  };

  constructor(private readonly session: UtauSession) {
    const { el } = this;
    el.folder.addEventListener('change', () => void this.loadFolder(el.folder));
    el.zip.addEventListener('change', () => void this.loadZip(el.zip));
    el.voiceFolder?.addEventListener('change', () => void this.loadFolder(el.voiceFolder!));
    el.voiceZip?.addEventListener('change', () => void this.loadZip(el.voiceZip!));
    el.pitch.addEventListener('input', () => this.showPitch());
    el.play.addEventListener('click', () => void this.play());
    el.alias.addEventListener('keydown', (e) => {
      e.stopPropagation(); // typing an alias must not play the page's computer-keyboard notes
      if (e.code === 'Enter') void this.play();
    });
    for (const i of [el.pitch, el.length]) i.addEventListener('keydown', (e) => e.stopPropagation());
    this.showPitch();
  }

  private q<T extends HTMLElement = HTMLElement>(id: string): T {
    const e = document.getElementById(id);
    if (!e) throw new Error(`Missing element #${id}`);
    return e as T;
  }

  private say(text: string, error = false): void {
    this.el.status.textContent = text;
    this.el.status.classList.toggle('error', error);
    if (this.el.voiceStatus) {
      this.el.voiceStatus.textContent = text;
      this.el.voiceStatus.classList.toggle('error', error);
    }
  }

  private showPitch(): void {
    const n = Number(this.el.pitch.value);
    this.el.pitchName.textContent = Number.isFinite(n) ? `${noteName(n)} · ${hzOf(n).toFixed(1)} Hz` : '';
  }

  private async loadFolder(input: HTMLInputElement): Promise<void> {
    const list = input.files;
    if (!list || list.length === 0) return;
    const files: BankFile[] = [...list].map((f) => ({
      path: f.webkitRelativePath || f.name,
      read: async () => new Uint8Array(await f.arrayBuffer()),
    }));
    await this.loadFiles(async () => files, (files[0].path.split('/')[0] || 'voicebank').normalize('NFC'));
  }

  private async loadZip(input: HTMLInputElement): Promise<void> {
    const f = input.files?.[0];
    if (!f) return;
    await this.loadFiles(() => openZip(f), f.name.replace(/\.zip$/i, '').normalize('NFC'));
    input.value = ''; // so picking the same file again still loads it
  }

  private async loadFiles(open: () => Promise<BankFile[]>, name: string): Promise<void> {
    this.say('Reading voicebank…');
    this.el.play.disabled = true;
    try {
      this.bank = await Voicebank.load(await open(), name);
    } catch (err) {
      this.bank = null;
      this.session.set(null);
      this.el.name.textContent = '';
      this.say(err instanceof Error ? err.message : String(err), true);
      return;
    }
    const b = this.bank;
    this.session.set(b);
    this.el.name.textContent = `— ${b.name}`;
    this.el.list.textContent = '';
    for (const a of b.aliases()) {
      const o = document.createElement('option');
      o.value = a;
      this.el.list.append(o);
    }
    if (!this.el.alias.value || !b.has(this.el.alias.value)) this.el.alias.value = b.aliases().find((a) => a.length <= 3) ?? b.aliases()[0];
    this.el.play.disabled = false;
    const summary = `${b.entries.length} entries, ${b.aliases().length} aliases from ${b.otoFiles.length} oto.ini.`;
    // Phones and tablets: decode samples when first used instead of all at once (the whole bank is ~190 MB decoded).
    if (window.matchMedia?.('(pointer: coarse)').matches) {
      this.say(`${summary} Samples are loaded as they are used on this device. Pick an alias and a pitch, then play.`);
      return;
    }
    this.say(`${summary} Preparing samples for live playing…`);
    // Decode every sample now so the first key press does not wait for a file.
    void b
      .preload((done, total) => {
        if (this.bank === b) this.say(`${summary} Preparing samples for live playing… ${done}/${total}`);
      })
      .then(() => {
        if (this.bank === b) this.say(`${summary} Ready: play live keys, run a melody, or audition an alias below.`);
      });
  }

  private async play(): Promise<void> {
    const bank = this.bank;
    if (!bank) return;
    const alias = this.el.alias.value.trim();
    if (!bank.has(alias)) {
      this.say(`"${alias}" is not an alias in this voicebank.`, true);
      return;
    }
    const lengthMs = Number(this.el.length.value);
    const out = await bank.renderAlias(alias, hzOf(Number(this.el.pitch.value) || 60), lengthMs > 0 ? lengthMs : undefined);
    if (!out) {
      this.say(`The sample for "${alias}" could not be read.`, true);
      return;
    }
    this.say(`${alias}: ${(out.samples.length / out.sampleRate).toFixed(2)} s at ${noteName(Number(this.el.pitch.value))}.`);
    const bus = await getBus();
    if (bus.ctx.state !== 'running') await bus.ctx.resume();
    this.source?.stop();
    const buf = bus.ctx.createBuffer(1, out.samples.length, out.sampleRate);
    buf.getChannelData(0).set(out.samples);
    const src = bus.ctx.createBufferSource();
    const gain = bus.ctx.createGain();
    gain.gain.value = 0.5;
    src.buffer = buf;
    src.connect(gain).connect(bus.input);
    src.start();
    this.source = src;
  }
}
