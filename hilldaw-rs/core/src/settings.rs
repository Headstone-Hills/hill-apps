//! Settings that belong to the device, not to a song: kept in their own file so opening a different project never changes them.

use serde::{Deserialize, Serialize};

/// Key under which the host stores these (the project has its own).
pub const SETTINGS_KEY: &str = "hilldaw-settings";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct Settings {
    /// The USB audio input to record from as `vid:pid` in hex; `None` = automatic (the FM-1 if it is attached, else the first input).
    pub input_device: Option<String>,
    /// The USB audio device playback comes out of as `vid:pid`; `None` = the built-in output.
    pub output_device: Option<String>,
}

impl Settings {
    /// Unreadable or missing settings give the defaults: a damaged settings file must never stop the app.
    pub fn from_json(s: &str) -> Settings {
        let mut v: Settings = serde_json::from_str(s).unwrap_or_default();
        // keep only something that looks like an id, so a damaged value cannot select a device that does not exist
        if v.input_device.as_deref().is_some_and(|d| !is_device_id(d)) {
            v.input_device = None;
        }
        if v.output_device.as_deref().is_some_and(|d| !is_device_id(d)) {
            v.output_device = None;
        }
        v
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("settings serialise")
    }
}

/// `vid:pid`, four hex digits each.
pub fn is_device_id(s: &str) -> bool {
    let mut parts = s.split(':');
    let ok = |p: Option<&str>| p.is_some_and(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_hexdigit()));
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip_and_damaged_ones_fall_back_to_automatic() {
        let s = Settings { input_device: Some("1234:00ab".into()), output_device: Some("0b0e:00ff".into()) };
        assert_eq!(Settings::from_json(&s.to_json()), s);
        assert_eq!(Settings::from_json("{nope"), Settings::default());
        assert_eq!(Settings::from_json("{}"), Settings::default(), "an old file with nothing in it");
        assert_eq!(Settings::from_json(r#"{"input_device":"../../etc"}"#).input_device, None);
        assert_eq!(Settings::from_json(r#"{"input_device":"12:34"}"#).input_device, None);
        assert_eq!(Settings::from_json(r#"{"output_device":"../x"}"#).output_device, None);
    }

    #[test]
    fn ids_are_vid_colon_pid() {
        assert!(is_device_id("4c4a:c755") && is_device_id("DEAD:beef"));
        assert!(!is_device_id("4c4a") && !is_device_id("4c4a:c755:1") && !is_device_id("zzzz:0000") && !is_device_id(""));
    }
}
