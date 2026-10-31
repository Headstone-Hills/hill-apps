// A UTAU voicebank loaded from the user's own folder: oto.ini entries indexed by alias, with samples
// and pitch tracks decoded on first use. Nothing is bundled or copied; the files stay where they are.

import { decodeText, frqPathFor, parseFrq, parseOto, parseWav, type OtoEntry } from './files';
import { renderSegment, type PsolaSource } from './psola';

export interface BankFile {
  /** Path relative to the folder the user picked, '/' separated. */
  path: string;
  read(): Promise<Uint8Array>;
}

export interface RenderedSample {
  samples: Float32Array;
  sampleRate: number;
}

/** Match paths across macOS (decomposed Unicode names) and oto.ini (usually composed) and ignore case. */
const key = (p: string): string => p.normalize('NFC').toLowerCase().replace(/\\/g, '/');

export class Voicebank {
  readonly entries: OtoEntry[] = [];
  private readonly byAlias = new Map<string, OtoEntry>();
  private readonly files = new Map<string, BankFile>();
  private readonly cache = new Map<string, Promise<PsolaSource | null>>();
  /** oto.ini files that were found, with how many entries each gave. */
  readonly otoFiles: Array<{ path: string; entries: number }> = [];

  private constructor(readonly name: string) {}

  static async load(files: ReadonlyArray<BankFile>, name = 'voicebank'): Promise<Voicebank> {
    const bank = new Voicebank(name);
    for (const f of files) bank.files.set(key(f.path), f);
    for (const f of files) {
      if (!/(^|\/)oto\.ini$/i.test(f.path)) continue;
      const dir = f.path.slice(0, f.path.length - 'oto.ini'.length);
      const entries = parseOto(decodeText(await f.read()), dir);
      bank.otoFiles.push({ path: f.path, entries: entries.length });
      for (const e of entries) {
        bank.entries.push(e);
        if (!bank.byAlias.has(e.alias)) bank.byAlias.set(e.alias, e); // the first definition wins, as in UTAU
      }
    }
    if (bank.entries.length === 0) throw new Error('No oto.ini found in that folder.');
    return bank;
  }

  has(alias: string): boolean {
    return this.byAlias.has(alias);
  }
  entry(alias: string): OtoEntry | undefined {
    return this.byAlias.get(alias);
  }
  aliases(): string[] {
    return [...this.byAlias.keys()];
  }

  /** Decode a sample and its pitch track (cached). Null when the WAV is missing or unreadable. */
  source(wavPath: string): Promise<PsolaSource | null> {
    const k = key(wavPath);
    let p = this.cache.get(k);
    if (!p) {
      p = this.decode(wavPath);
      this.cache.set(k, p);
    }
    return p;
  }

  private async decode(wavPath: string): Promise<PsolaSource | null> {
    const f = this.files.get(key(wavPath));
    if (!f) return null;
    try {
      const wav = parseWav(await f.read());
      const frqFile = this.files.get(key(frqPathFor(wavPath)));
      let frq = null;
      if (frqFile) {
        try {
          frq = parseFrq(await frqFile.read());
        } catch {
          frq = null;
        }
      }
      return { samples: wav.samples, sampleRate: wav.sampleRate, frq };
    } catch {
      return null;
    }
  }

  /** The part of the file an oto entry uses, in samples: [start, end) and the length of its fixed (consonant) part. */
  region(e: OtoEntry, src: PsolaSource): { start: number; end: number; fixed: number } {
    const ms = src.sampleRate / 1000;
    const total = src.samples.length;
    const start = Math.max(0, Math.min(total, Math.round(e.offset * ms)));
    // A negative cutoff is the length of the usable area after the offset; a positive one is trimmed from the end.
    const end = e.cutoff < 0 ? start + Math.round(-e.cutoff * ms) : total - Math.round(e.cutoff * ms);
    const clamped = Math.max(start + 1, Math.min(total, end));
    return { start, end: clamped, fixed: Math.max(0, Math.min(clamped - start - 1, Math.round(e.consonant * ms))) };
  }

  /**
   * Render one alias at a pitch, for auditioning. `lengthMs` stretches the part after the consonant
   * (the vowel); without it the sample plays at its natural length.
   */
  async renderAlias(alias: string, f0: number, lengthMs?: number): Promise<RenderedSample | null> {
    const e = this.byAlias.get(alias);
    if (!e) return null;
    const src = await this.source(e.wavPath);
    if (!src) return null;
    const r = this.region(e, src);
    const natural = r.end - r.start;
    const length = lengthMs !== undefined ? Math.round((lengthMs * src.sampleRate) / 1000) : natural;
    const samples = renderSegment(src, { start: r.start, end: r.end, fixed: r.fixed, length: Math.max(1, length), f0 });
    // A short fade at both ends so a cut-off sample does not click.
    const fade = Math.min(Math.round(src.sampleRate * 0.004), samples.length >> 1);
    for (let i = 0; i < fade; i++) {
      const w = i / fade;
      samples[i] *= w;
      samples[samples.length - 1 - i] *= w;
    }
    return { samples, sampleRate: src.sampleRate };
  }
}
