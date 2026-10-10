pub mod band;
pub mod beat;
pub mod chord;
pub mod sequencer;

use crate::{App, Host};

/// Names accepted by [`create`].
pub const NAMES: [&str; 4] = ["hillbeat", "hillsequencer", "hillband", "hillchord"];

pub fn create(name: &str, host: Box<dyn Host>) -> Option<Box<dyn App>> {
    Some(match name {
        "hillbeat" => Box::new(beat::new(host)),
        "hillsequencer" => Box::new(sequencer::new(host)),
        "hillband" => Box::new(band::new(host)),
        "hillchord" => Box::new(chord::new(host)),
        _ => return None,
    })
}
