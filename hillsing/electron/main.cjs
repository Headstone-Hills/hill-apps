// Electron wrapper for HillSing (Apple Silicon Mac app). The renderer is the same web build
// (dist/); this file only opens a window, serves dist/ over a privileged app:// scheme, and allows
// Web MIDI. Electron rather than Tauri so Chromium's Web MIDI and AudioWorklet behave exactly as in
// Chrome.
//
// Why a custom scheme instead of file://: AudioWorklet modules, fetch() of the dictionary and
// module workers need a secure origin with normal fetch semantics, which file:// does not give.

const { app, BrowserWindow, Menu, protocol, net, session, shell } = require('electron');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const DIST = path.join(__dirname, '..', 'dist');
const SCHEME = 'app';

protocol.registerSchemesAsPrivileged([
  { scheme: SCHEME, privileges: { standard: true, secure: true, supportFetchAPI: true, stream: true } },
]);

/** Serve files from dist/ and refuse anything that would escape it. */
function registerProtocol() {
  protocol.handle(SCHEME, (request) => {
    const url = new URL(request.url);
    const rel = decodeURIComponent(url.pathname === '/' ? '/index.html' : url.pathname);
    const file = path.normalize(path.join(DIST, rel));
    if (file !== DIST && !file.startsWith(DIST + path.sep)) {
      return new Response('Forbidden', { status: 403 });
    }
    return net.fetch(pathToFileURL(file).toString()).then((res) => {
      // Never cache the app's own files: a cached index.html would keep showing an old build after an update.
      const headers = new Headers(res.headers);
      headers.set('Cache-Control', 'no-store');
      return new Response(res.body, { status: res.status, statusText: res.statusText, headers });
    });
  });
}

/**
 * Only the permissions the app needs: Web MIDI, and "media" for audio only. The media permission is not used to record
 * anything: Chromium hides the names of audio devices (such as BlackHole) from the output picker unless it is granted.
 */
function allowMidi() {
  const midi = new Set(['midi', 'midiSysex']);
  session.defaultSession.setPermissionRequestHandler((_wc, permission, callback, details) => {
    if (permission === 'media') return callback((details.mediaTypes || []).every((t) => t === 'audio'));
    callback(midi.has(permission));
  });
  session.defaultSession.setPermissionCheckHandler((_wc, permission, _origin, details) => {
    if (permission === 'media') return details && details.mediaType !== 'video';
    return midi.has(permission);
  });
}

function createWindow() {
  const win = new BrowserWindow({
    width: 1180,
    height: 900,
    minWidth: 720,
    minHeight: 600,
    title: 'HillSing',
    backgroundColor: '#14171c',
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      // The page starts audio from its own "Start audio" button; this just avoids a second gate.
      autoplayPolicy: 'no-user-gesture-required',
    },
  });
  // Links out of the app (e.g. documentation) open in the default browser, never in the app window.
  win.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:/.test(url)) shell.openExternal(url);
    return { action: 'deny' };
  });
  win.loadURL(`${SCHEME}://hillsing/index.html`);
  return win;
}

function buildMenu() {
  const template = [
    { role: 'appMenu' },
    { role: 'editMenu' },
    { label: 'View', submenu: [{ role: 'reload' }, { role: 'toggleDevTools' }, { type: 'separator' }, { role: 'togglefullscreen' }] },
    { role: 'windowMenu' },
  ];
  Menu.setApplicationMenu(Menu.buildFromTemplate(template));
}

app.whenReady().then(async () => {
  registerProtocol();
  allowMidi();
  buildMenu();
  // Start from an empty HTTP cache so nothing from an older build can be shown (settings in localStorage are untouched).
  await session.defaultSession.clearCache().catch(() => {});
  createWindow();
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit();
});
