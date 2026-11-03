// The voicebank library in the Mac app: a folder chosen once (an external drive is fine) where each voicebank is a
// sub-folder or a .zip. The page reaches the disk only through the small bridge in electron/preload.cjs.

export interface LibraryEntry {
  kind: 'folder' | 'zip';
  name: string;
  path: string;
  size?: number;
}

export interface LibraryState {
  /** The saved folder, or null if none has been chosen yet. */
  root: string | null;
  ok: boolean;
  error?: 'not-found' | 'not-a-folder' | 'unreadable';
  entries: LibraryEntry[];
}

export interface HillLibrary {
  get(): Promise<LibraryState>;
  choose(): Promise<LibraryState | null>;
  files(bankPath: string): Promise<string[]>;
  read(filePath: string): Promise<Uint8Array>;
}

declare global {
  interface Window {
    /** Present only in the Mac app. */
    hillLibrary?: HillLibrary;
  }
}

/** The last part of a folder path, for showing. */
export const baseName = (p: string): string => p.replace(/\/+$/, '').split('/').pop() || p;

export function describeLibrary(state: LibraryState): string {
  if (!state.root) return 'No voicebank folder chosen yet.';
  if (!state.ok) {
    return state.error === 'not-found'
      ? `Voicebank folder not found: ${state.root}. Is the drive connected?`
      : `Could not read the voicebank folder ${state.root}.`;
  }
  const n = state.entries.length;
  return n === 0
    ? `No voicebanks found in ${state.root}: put each one in its own folder (with its oto.ini) or as a .zip.`
    : `${n} voicebank${n === 1 ? '' : 's'} in ${state.root}.`;
}
