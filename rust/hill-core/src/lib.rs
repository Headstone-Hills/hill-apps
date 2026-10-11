//! Shared engine for the Hill* apps: audio mixer, WAV loading, sequencer
//! clock, software-rendered UI, and the four apps themselves.
//!
//! The crate does no I/O and spawns no threads. A runner (SDL on device,
//! WebAssembly in the browser) feeds it button events, asks it for audio
//! buffers and for a framebuffer, and provides a [`Host`] for samples/state.

pub mod apps;
pub mod fb;
pub mod grid;
pub mod host;
pub mod input;
pub mod instrument;
pub mod kv;
pub mod library;
pub mod mixer;
pub mod subtractive;
pub mod synth;
pub mod theory;
pub mod transport;
pub mod ui;
pub mod wav;

pub use host::{Host, MemHost};
pub use input::Btn;

/// Output sample rate the apps are built for. Runners should open the audio
/// device at this rate and call [`App::set_rate`] if the device disagrees.
pub const SAMPLE_RATE: f32 = 44100.0;

/// The interface every Hill app presents to a runner.
pub trait App: Send {
    fn name(&self) -> &'static str;
    /// A button went down or up.
    fn event(&mut self, b: Btn, down: bool);
    /// Fill `out` (interleaved stereo f32) and advance the musical clock.
    fn audio(&mut self, out: &mut [f32]);
    /// Draw the current screen into `fb` (640x480).
    fn render(&self, fb: &mut fb::Fb);
    /// UI-rate timer (about 60 Hz): hold detection, auto-repeat, toasts.
    fn tick(&mut self, dt_ms: u32);
    fn set_rate(&mut self, rate: f32);
    /// True once the user has asked to quit (START+SELECT held).
    fn wants_quit(&self) -> bool;
    /// Persist state through the host.
    fn save(&mut self);
}
