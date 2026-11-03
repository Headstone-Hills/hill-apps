// A UTAU voicebank loaded from the user's own folder: oto.ini entries indexed by alias, with samples
// and pitch tracks decoded on first use. Nothing is bundled or copied; the files stay where they are.

import { decodeText, frqPathFor, parseFrq, parseOto, parseWav, type OtoEntry } from './files';
import { repairFileNameInPath } from './names';
import { detectDialect, planUnits, type Dialect } from './planner';
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
  /** Banks that record every sound at several pitches ("ay Bb2", "ay D3"): the variants of each sound, by pitch. */
  private readonly variants = new Map<string, Array<{ hz: number; entry: OtoEntry }>>();
  /** True when the bank was recorded at more than one pitch. */
  multiPitch = false;
  /**
   * Gain that brings a quietly recorded bank up to the level of a typical one (1 for banks that are loud enough). Measured
   * from the vowel samples the planner would use, so every bank sings at a comparable volume.
   */
  levelGain = 1;
  /** How the bank names its sounds (X-SAMPA like Kasane Teto, or ARPAbet like ARPAsing banks). */
  dialect: Dialect = 'xsampa';

  private constructor(readonly name: string) {}

  static async load(files: ReadonlyArray<BankFile>, name = 'voicebank'): Promise<Voicebank> {
    const bank = new Voicebank(name);
    for (const f of files) {
      bank.files.set(key(f.path), f);
      // Samples unzipped with the wrong encoding have mangled names; oto.ini names the repaired ones.
      const fixed = repairFileNameInPath(f.path);
      if (fixed && !bank.files.has(key(fixed))) bank.files.set(key(fixed), f);
    }
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
    bank.collapsePitchVariants();
    bank.dialect = detectDialect((a) => bank.byAlias.has(a));
    await bank.measureLevel();
    return bank;
  }

  /**
   * Multi-pitch banks name each recording with its pitch ("ay Bb2", "b ae D4", "@ @A4"). Fold those into one name per
   * sound, remembering which pitch each recording is at, so a note can use the recording nearest its pitch. A bank only
   * counts as multi-pitch when several such suffixes each cover many entries (so a stray alias like "D3" is not mistaken).
   */
  private collapsePitchVariants(): void {
    const re = /^(.*?)\s*([A-G][#b]?)(\d)$/;
    // A real pitch suffix is also the name of the folder the recording sits in ("Bb2/19.wav" for "b ae Bb2"); sound codes
    // that merely end like a note ("...A4" in a VCCV bank) are not.
    const folder = (e: OtoEntry): string => e.wavPath.split('/').slice(-2, -1)[0] ?? '';
    const isPitch = (e: OtoEntry, m: RegExpExecArray | null): boolean => !!m && folder(e).toLowerCase() === (m[2] + m[3]).toLowerCase();
    const counts = new Map<string, number>();
    for (const e of this.entries) {
      const m = re.exec(e.alias);
      if (isPitch(e, m)) counts.set(m![2] + m![3], (counts.get(m![2] + m![3]) ?? 0) + 1);
    }
    const common = new Set([...counts].filter(([, n]) => n >= 100).map(([k]) => k));
    const covered = this.entries.filter((e) => {
      const m = re.exec(e.alias);
      return isPitch(e, m) && common.has(m![2] + m![3]);
    }).length;
    if (common.size < 2 || covered < this.entries.length * 0.6) return;
    this.multiPitch = true;
    this.byAlias.clear();
    const name = (k: string): number => {
      const semis: Record<string, number> = { C: 0, 'C#': 1, Db: 1, D: 2, 'D#': 3, Eb: 3, E: 4, F: 5, 'F#': 6, Gb: 6, G: 7, 'G#': 8, Ab: 8, A: 9, 'A#': 10, Bb: 10, B: 11 };
      const note = k.slice(0, -1);
      const octave = Number(k.slice(-1));
      return 440 * Math.pow(2, (12 * (octave + 1) + semis[note] - 69) / 12);
    };
    for (const e of this.entries) {
      const m = re.exec(e.alias);
      if (isPitch(e, m) && common.has(m![2] + m![3])) {
        const base = m![1].trim();
        const list = this.variants.get(base) ?? [];
        const hz = name(m![2] + m![3]);
        if (!list.some((v) => Math.abs(v.hz - hz) < 1)) list.push({ hz, entry: e }); // the first definition of each pitch wins
        this.variants.set(base, list);
      } else if (!this.byAlias.has(e.alias)) this.byAlias.set(e.alias, e);
    }
    for (const [base, list] of this.variants) {
      list.sort((a, b) => a.hz - b.hz);
      if (!this.byAlias.has(base)) this.byAlias.set(base, list.reduce((best, v) => (Math.abs(Math.log2(v.hz / 262)) < Math.abs(Math.log2(best.hz / 262)) ? v : best)).entry);
    }
  }

  /** Measure how loud this bank's vowels are and set `levelGain` (see there). Banks whose samples cannot be read keep 1. */
  private async measureLevel(): Promise<void> {
    const TARGET = 0.3; // a typical vowel's steady-part RMS (the English Kasane Teto bank sits at about 0.32)
    const vowels = ['AA1', 'AE1', 'AH1', 'AO1', 'AY1', 'EH1', 'ER1', 'EY1', 'IH1', 'IY1', 'OW1', 'UH1', 'UW1'];
    const levels: number[] = [];
    for (const nucleus of vowels) {
      const u = planUnits((a) => this.byAlias.has(a), { onset: ['B'], nucleus, coda: [] }, { prevVowel: null, releaseAtEnd: false, melisma: false, dialect: this.dialect }).find((x) => x.role === 'vowel');
      const e = u && this.entry(u.alias, 262);
      const src = e && (await this.source(e.wavPath));
      if (!e || !src) continue;
      const r = this.region(e, src);
      const a = r.start + r.fixed;
      const len = r.end - a;
      if (len < 800) continue;
      const from = a + Math.round(len * 0.3);
      const to = a + Math.round(len * 0.8);
      let sum = 0;
      for (let k = from; k < to; k++) sum += src.samples[k] * src.samples[k];
      levels.push(Math.sqrt(sum / (to - from)));
    }
    if (levels.length < 4) return;
    levels.sort((x, y) => x - y);
    const median = levels[levels.length >> 1];
    if (median > 1e-4) this.levelGain = Math.min(6, Math.max(1, TARGET / median));
  }

  has(alias: string): boolean {
    return this.byAlias.has(alias);
  }
  /** The recording for `alias`; in a multi-pitch bank, the one nearest `hz` (middle C if no pitch is given). */
  entry(alias: string, hz?: number): OtoEntry | undefined {
    const list = this.variants.get(alias);
    if (list && hz && hz > 0) {
      return list.reduce((best, v) => (Math.abs(Math.log2(v.hz / hz)) < Math.abs(Math.log2(best.hz / hz)) ? v : best)).entry;
    }
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

  /**
   * Decode every sample ahead of time (for live playing, where a first key press must not wait for a file).
   * Reports progress and yields to the browser between files so the page stays responsive.
   */
  async preload(onProgress?: (done: number, total: number) => void): Promise<void> {
    const paths = [...new Set(this.entries.map((e) => e.wavPath))];
    let done = 0;
    for (const p of paths) {
      await this.source(p);
      done++;
      if (done % 6 === 0) {
        onProgress?.(done, paths.length);
        await new Promise((r) => setTimeout(r, 0));
      }
    }
    onProgress?.(paths.length, paths.length);
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
    const e = this.entry(alias, f0);
    if (!e) return null;
    const src = await this.source(e.wavPath);
    if (!src) return null;
    const r = this.region(e, src);
    const natural = r.end - r.start;
    const length = lengthMs !== undefined ? Math.round((lengthMs * src.sampleRate) / 1000) : natural;
    const samples = renderSegment(src, { start: r.start, end: r.end, fixed: r.fixed, length: Math.max(1, length), f0, gain: this.levelGain });
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
