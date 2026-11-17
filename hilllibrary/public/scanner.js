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

const PHOTO_ANGLES = [0, 90, 270, -6, 6, -12, 12];

/** Draw `bitmap` scaled and rotated by `angleDeg` onto the canvas, resizing it to fit (white fill, no cropping). */
function drawRotated(canvas, ctx, bitmap, scale, angleDeg) {
  const w = bitmap.width * scale, h = bitmap.height * scale, r = (angleDeg * Math.PI) / 180;
  canvas.width = Math.max(1, Math.round(Math.abs(w * Math.cos(r)) + Math.abs(h * Math.sin(r)))); // resizing resets the transform
  canvas.height = Math.max(1, Math.round(Math.abs(w * Math.sin(r)) + Math.abs(h * Math.cos(r))));
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.translate(canvas.width / 2, canvas.height / 2);
  ctx.rotate(r);
  ctx.drawImage(bitmap, -w / 2, -h / 2, w, h);
  ctx.setTransform(1, 0, 0, 1, 0, 0);
}

async function makeDetector() {
  if ('BarcodeDetector' in window) {
    try {
      const formats = await window.BarcodeDetector.getSupportedFormats();
      if (formats.includes('ean_13')) {
        const d = new window.BarcodeDetector({ formats: ['ean_13'] });
        return {
          name: 'native',
          interval: 120,
          detect: async (video) => (await d.detect(video))[0]?.rawValue ?? null,
          // A still photo may contain several codes; take the first that is a real ISBN.
          detectImage: async (bitmap) => (await d.detect(bitmap)).map((r) => r.rawValue).find(isIsbn13) ?? null,
        };
      }
    } catch { /* fall through to ZXing */ }
  }
  if (!window.ZXing) await loadScript('/vendor/zxing.min.js');
  const Z = window.ZXing;
  // Use the EAN-13 reader directly. (MultiFormatReader.decode() throws away our hints and runs every 2D reader
  // too; decodeWithState() keeps them but logs a console warning for every frame that has no barcode in it.)
  const reader = new Z.EAN13Reader();
  const hints = new Map();
  hints.set(Z.DecodeHintType.TRY_HARDER, true); // scan more rows, more thoroughly
  const canvas = document.createElement('canvas');
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  const decodeCanvas = () => {
    try {
      const bitmap = new Z.BinaryBitmap(new Z.HybridBinarizer(new Z.HTMLCanvasElementLuminanceSource(canvas)));
      return reader.decode(bitmap, hints).getText();
    } catch { return null; } // NotFound / Checksum / Format: just no barcode in this image
    finally { reader.reset?.(); }
  };
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
      return decodeCanvas();
    },
    // A photo is much bigger than a video frame, and the barcode can be anywhere in it, at any angle. Try the whole
    // image at a few sizes (downscaling often helps with blur and sensor noise), upright first, then sideways and
    // slightly tilted (1D readers only scan along straight lines, so a tilted code needs a straightened copy).
    detectImage: async (bitmap) => {
      const attempts = [
        ...PHOTO_ANGLES.map((a) => [1800, a]),
        ...PHOTO_ANGLES.map((a) => [1100, a]),
        [2600, 0],
      ];
      for (const [maxSide, angle] of attempts) {
        drawRotated(canvas, ctx, bitmap, Math.min(1, maxSide / Math.max(bitmap.width, bitmap.height)), angle);
        const text = decodeCanvas();
        if (text && isIsbn13(text)) return text;
        await new Promise((r) => setTimeout(r)); // let the page breathe between attempts
      }
      return null;
    },
  };
}

let detectorPromise = null;
const getDetector = () => (detectorPromise ??= makeDetector().catch((e) => { detectorPromise = null; throw e; }));

/**
 * Read an ISBN-13 barcode from a single photo (e.g. one taken with the phone's own camera app).
 * Needs no live camera stream, no getUserMedia, and works on plain HTTP too.
 * Resolves to the ISBN-13 string, or null if no valid ISBN barcode was found.
 */
export async function decodeIsbnFromImage(file) {
  const bitmap = await createImageBitmap(file);
  try {
    return await (await getDetector()).detectImage(bitmap);
  } finally {
    bitmap.close?.();
  }
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
      detector = await getDetector();
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
