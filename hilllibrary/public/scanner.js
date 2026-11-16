// Camera barcode scanning.
//
// Strategy: use the browser's native BarcodeDetector when it supports EAN-13
// (Chrome/Edge/Android). Otherwise load the vendored ZXing decoder and decode
// canvas frames ourselves (Safari, Firefox, desktop Linux). Either way, only
// checksum-valid ISBN-13 barcodes (978/979 prefix) are reported.
//
// Camera access requires a secure context (HTTPS or localhost) in every browser.

export function isIsbn13(s) {
  if (!/^97[89]\d{10}$/.test(s)) return false;
  let sum = 0;
  for (let i = 0; i < 12; i++) sum += Number(s[i]) * (i % 2 ? 3 : 1);
  return (10 - (sum % 10)) % 10 === Number(s[12]);
}

/** Why the camera can't be used here, or null if it can. */
export function cameraProblem() {
  if (!window.isSecureContext) return 'insecure';
  if (!navigator.mediaDevices?.getUserMedia) return 'unsupported';
  return null;
}

function loadScript(src) {
  return new Promise((resolve, reject) => {
    const s = document.createElement('script');
    s.src = src;
    s.onload = resolve;
    s.onerror = () => reject(new Error('Could not load the barcode decoder'));
    document.head.append(s);
  });
}

async function makeDetector() {
  if ('BarcodeDetector' in window) {
    try {
      const formats = await window.BarcodeDetector.getSupportedFormats();
      if (formats.includes('ean_13')) {
        const d = new window.BarcodeDetector({ formats: ['ean_13'] });
        return { name: 'native', interval: 120, detect: async (video) => (await d.detect(video))[0]?.rawValue ?? null };
      }
    } catch { /* fall through to ZXing */ }
  }
  if (!window.ZXing) await loadScript('/vendor/zxing.min.js');
  const Z = window.ZXing;
  const reader = new Z.MultiFormatReader();
  const hints = new Map();
  hints.set(Z.DecodeHintType.POSSIBLE_FORMATS, [Z.BarcodeFormat.EAN_13]);
  hints.set(Z.DecodeHintType.TRY_HARDER, true);
  reader.setHints(hints);
  const canvas = document.createElement('canvas');
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  return {
    name: 'zxing',
    interval: 220,
    detect: async (video) => {
      const vw = video.videoWidth, vh = video.videoHeight;
      if (!vw || !vh) return null;
      // Decode the centre band (where the guide box is) at up to 1280px wide: faster and less noise.
      const sx = Math.round(vw * 0.04), sw = Math.round(vw * 0.92);
      const sy = Math.round(vh * 0.2), sh = Math.round(vh * 0.6);
      const scale = Math.min(1, 1280 / sw);
      canvas.width = Math.round(sw * scale);
      canvas.height = Math.round(sh * scale);
      ctx.drawImage(video, sx, sy, sw, sh, 0, 0, canvas.width, canvas.height);
      try {
        const bitmap = new Z.BinaryBitmap(new Z.HybridBinarizer(new Z.HTMLCanvasElementLuminanceSource(canvas)));
        return reader.decode(bitmap).getText();
      } catch { return null; } // NotFound / Checksum / Format: just no barcode in this frame
      finally { reader.reset?.(); }
    },
  };
}

/**
 * @param {{video: HTMLVideoElement, onCode: (isbn13:string)=>void, onStatus?: (msg:string)=>void}} o
 */
export function createScanner({ video, onCode, onStatus = () => {} }) {
  let stream = null;
  let detector = null;
  let running = false;
  let paused = false;
  let timer = 0;
  let lastCode = '';
  let lastAt = 0;

  async function tick() {
    if (!running) return;
    if (!paused && video.readyState >= 2) {
      let raw = null;
      try { raw = await detector.detect(video); } catch { /* transient decode error */ }
      if (raw && isIsbn13(raw) && !(raw === lastCode && Date.now() - lastAt < 3000)) {
        lastCode = raw; lastAt = Date.now();
        navigator.vibrate?.(60);
        paused = true;
        onCode(raw);
      }
    }
    if (running) timer = setTimeout(tick, detector.interval);
  }

  return {
    get engine() { return detector?.name ?? null; },
    get active() { return running; },

    async start() {
      if (running) return;
      onStatus('Starting camera…');
      stream = await navigator.mediaDevices.getUserMedia({
        audio: false,
        video: { facingMode: { ideal: 'environment' }, width: { ideal: 1920 }, height: { ideal: 1080 } },
      });
      video.srcObject = stream;
      video.setAttribute('playsinline', '');
      video.muted = true;
      await video.play();
      try { // continuous autofocus where the device supports it
        const track = stream.getVideoTracks()[0];
        if (track.getCapabilities?.().focusMode?.includes('continuous')) await track.applyConstraints({ advanced: [{ focusMode: 'continuous' }] });
      } catch { /* optional */ }
      detector = await makeDetector();
      running = true; paused = false;
      onStatus('Point the camera at the barcode on the back cover.');
      tick();
    },

    stop() {
      running = false;
      clearTimeout(timer);
      stream?.getTracks().forEach((t) => t.stop());
      stream = null;
      video.srcObject = null;
    },

    pause() { paused = true; },
    resume() { lastAt = Date.now(); paused = false; onStatus('Point the camera at the barcode on the back cover.'); },

    /** Torch is only offered where the camera reports it can do it. */
    torchSupported() { return !!stream?.getVideoTracks()[0]?.getCapabilities?.().torch; },
    async setTorch(on) {
      await stream?.getVideoTracks()[0]?.applyConstraints({ advanced: [{ torch: !!on }] });
    },
  };
}
