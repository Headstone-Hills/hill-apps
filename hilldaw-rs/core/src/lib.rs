//! HillDAW core: everything that can be tested on a normal computer with no hardware.
//!
//! * [`transport`] sample-position clock: tempo, bars/beats, loop region, snap
//! * [`mixer`]     tracks (gain/pan/mute/solo), sources, master limiter, meters
//! * [`wav`]       WAV decode to float + linear resampling
//! * [`project`]   versioned project model (JSON)
//!
//! One absolute frame counter positions every source, so tracks cannot drift against each other and the
//! output is identical no matter how the host chops it into chunks.

pub mod drift;
pub mod drums;
pub mod edit;
pub mod export;
pub mod fm1;
pub mod effects;
pub mod mixer;
pub mod notes;
pub mod probe;
pub mod project;
pub mod record;
pub mod sampler;
pub mod session;
pub mod settings;
pub mod tempo;
pub mod transport;
pub mod waveform;
pub mod wav;

/// Project sample rate: the FM-1's native capture rate, so no resampling on record.
pub const RATE: u32 = 44100;
