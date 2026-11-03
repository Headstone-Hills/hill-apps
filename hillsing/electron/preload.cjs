// The only bridge from the page to the disk: pick the voicebank folder, list what is in it, read a file from it.
// The main process checks every path against the chosen folder, so the page cannot read anything outside it.

const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('hillLibrary', {
  /** { root, entries, ok, error? } for the saved folder (root null when none has been chosen). */
  get: () => ipcRenderer.invoke('library:get'),
  /** Opens a folder picker; resolves with the same shape for the new folder, or null if cancelled. */
  choose: () => ipcRenderer.invoke('library:choose'),
  /** Relative paths of every file in a voicebank folder from the library. */
  files: (bankPath) => ipcRenderer.invoke('library:files', bankPath),
  /** The bytes of one file inside the library. */
  read: (filePath) => ipcRenderer.invoke('library:read', filePath),
});
