//! Browser build. The wasm module exposes a small C ABI that JavaScript
//! drives from an AudioWorklet (or, as a fallback, the main thread):
//!
//! * `hill_reset`, `hill_add_sample`, `hill_set_state` - give the host its files/state
//! * `hill_init(app, rate)` - create an app
//! * `hill_event`, `hill_tick`, `hill_audio`, `hill_render` - run it
//! * `hill_state_*` - read back saved state so JS can persist it

use hill_core::fb::{Fb, H, W};
use hill_core::{apps, App, Btn, Host, MemHost};
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

struct WebHost(Arc<Mutex<MemHost>>);

impl Host for WebHost {
    fn list_samples(&self) -> Vec<String> {
        self.0.lock().map(|h| h.list_samples()).unwrap_or_default()
    }
    fn read_sample(&self, name: &str) -> Option<Vec<u8>> {
        self.0.lock().ok()?.read_sample(name)
    }
    fn load_state(&self, app: &str) -> Option<String> {
        self.0.lock().ok()?.load_state(app)
    }
    fn save_state(&mut self, app: &str, data: &str) {
        if let Ok(mut h) = self.0.lock() {
            h.save_state(app, data);
        }
    }
}

struct Web {
    host: Arc<Mutex<MemHost>>,
    app: Option<Box<dyn App>>,
    app_name: &'static str,
    audio: Vec<f32>,
    fb: Fb,
    rgba: Vec<u8>,
    state_buf: Vec<u8>,
}

thread_local! {
    static WEB: RefCell<Web> = RefCell::new(Web {
        host: Arc::new(Mutex::new(MemHost::default())),
        app: None,
        app_name: "",
        audio: Vec::new(),
        fb: Fb::new(),
        rgba: vec![0; (W * H * 4) as usize],
        state_buf: Vec::new(),
    });
}

fn with<R>(f: impl FnOnce(&mut Web) -> R) -> R {
    WEB.with(|w| f(&mut w.borrow_mut()))
}

fn btn_from(i: u32) -> Option<Btn> {
    Btn::ALL.get(i as usize).copied()
}

/// Order JavaScript must use for button ids.
#[no_mangle]
pub extern "C" fn hill_button_count() -> u32 {
    Btn::ALL.len() as u32
}

#[no_mangle]
pub extern "C" fn hill_alloc(n: usize) -> *mut u8 {
    let mut v: Vec<u8> = Vec::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `p`/`n` must come from a matching `hill_alloc(n)`.
#[no_mangle]
pub unsafe extern "C" fn hill_free(p: *mut u8, n: usize) {
    drop(Vec::from_raw_parts(p, 0, n.max(1)));
}

unsafe fn bytes<'a>(p: *const u8, n: usize) -> &'a [u8] {
    std::slice::from_raw_parts(p, n)
}

/// Forget the current app, all samples and all saved state.
#[no_mangle]
pub extern "C" fn hill_reset() {
    with(|w| {
        w.app = None;
        w.host = Arc::new(Mutex::new(MemHost::default()));
    });
}

/// # Safety
/// Pointers must be valid for their lengths (they come from `hill_alloc`).
#[no_mangle]
pub unsafe extern "C" fn hill_add_sample(name: *const u8, name_len: usize, data: *const u8, data_len: usize) {
    let name = String::from_utf8_lossy(bytes(name, name_len)).to_string();
    let data = bytes(data, data_len).to_vec();
    with(|w| {
        if let Ok(mut h) = w.host.lock() {
            h.samples.insert(name, data);
        }
    });
}

/// # Safety
/// Pointer must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn hill_set_state(app: u32, p: *const u8, len: usize) {
    let Some(name) = apps::NAMES.get(app as usize) else { return };
    let text = String::from_utf8_lossy(bytes(p, len)).to_string();
    with(|w| {
        if let Ok(mut h) = w.host.lock() {
            h.state.insert(name.to_string(), text);
            h.state_dirty = false;
        }
    });
}

/// Create app number `app` (index into `apps::NAMES`). Returns 1 on success.
#[no_mangle]
pub extern "C" fn hill_init(app: u32, rate: f32) -> u32 {
    with(|w| {
        let Some(name) = apps::NAMES.get(app as usize) else { return 0 };
        match apps::create(name, Box::new(WebHost(w.host.clone()))) {
            Some(mut a) => {
                a.set_rate(rate);
                w.app = Some(a);
                w.app_name = name;
                1
            }
            None => 0,
        }
    })
}

#[no_mangle]
pub extern "C" fn hill_event(btn: u32, down: u32) {
    if let Some(b) = btn_from(btn) {
        with(|w| {
            if let Some(a) = w.app.as_mut() {
                a.event(b, down != 0);
            }
        });
    }
}

#[no_mangle]
pub extern "C" fn hill_tick(dt_ms: u32) {
    with(|w| {
        if let Some(a) = w.app.as_mut() {
            a.tick(dt_ms);
        }
    });
}

/// Render `frames` stereo frames; returns a pointer to interleaved f32.
#[no_mangle]
pub extern "C" fn hill_audio(frames: u32) -> *const f32 {
    with(|w| {
        let n = frames as usize * 2;
        if w.audio.len() < n {
            w.audio.resize(n, 0.0);
        }
        let Web { app, audio, .. } = w;
        match app.as_mut() {
            Some(a) => a.audio(&mut audio[..n]),
            None => audio[..n].iter_mut().for_each(|x| *x = 0.0),
        }
        w.audio.as_ptr()
    })
}

/// Draw the screen; returns a pointer to 640x480 RGBA bytes.
#[no_mangle]
pub extern "C" fn hill_render() -> *const u8 {
    with(|w| {
        let Web { app, fb, rgba, .. } = w;
        if let Some(a) = app.as_ref() {
            a.render(fb);
        }
        fb.to_rgba(rgba);
        w.rgba.as_ptr()
    })
}

#[no_mangle]
pub extern "C" fn hill_wants_quit() -> u32 {
    with(|w| w.app.as_ref().map_or(0, |a| a.wants_quit() as u32))
}

/// Ask the app to persist its state through the host.
#[no_mangle]
pub extern "C" fn hill_save() {
    with(|w| {
        if let Some(a) = w.app.as_mut() {
            a.save();
        }
    });
}

#[no_mangle]
pub extern "C" fn hill_state_dirty() -> u32 {
    with(|w| w.host.lock().map_or(0, |h| h.state_dirty as u32))
}

/// Copy the current app's saved state into an internal buffer, clear the
/// dirty flag and return its length. Read it with `hill_state_ptr`.
#[no_mangle]
pub extern "C" fn hill_state_take() -> usize {
    with(|w| {
        let text = w
            .host
            .lock()
            .ok()
            .map(|mut h| {
                h.state_dirty = false;
                h.state.get(w.app_name).cloned().unwrap_or_default()
            })
            .unwrap_or_default();
        w.state_buf = text.into_bytes();
        w.state_buf.len()
    })
}

#[no_mangle]
pub extern "C" fn hill_state_ptr() -> *const u8 {
    with(|w| w.state_buf.as_ptr())
}

#[no_mangle]
pub extern "C" fn hill_sample_count() -> u32 {
    with(|w| w.host.lock().map_or(0, |h| h.samples.len() as u32))
}
