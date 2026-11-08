//! Settings that belong to the device, not to a song: kept in their own file so opening a different project never changes them.

use serde::{Deserialize, Serialize};

/// Key under which the host stores these (the project has its own).
pub const SETTINGS_KEY: &str = "hilldaw-settings";

/// The song that exists before anything is made: its project is stored under the key the first versions used, so an old install keeps it.
pub const FIRST_SONG: &str = "song";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// The USB audio input to record from as `vid:pid` in hex; `None` = automatic (the FM-1 if it is attached, else the first input).
    pub input_device: Option<String>,
    /// The USB audio device playback comes out of as `vid:pid`; `None` = the built-in output.
    pub output_device: Option<String>,
    /// Every song on the card (folder names under the projects folder) and the one that is open.
    pub projects: Vec<String>,
    pub current_project: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { input_device: None, output_device: None, projects: vec![FIRST_SONG.into()], current_project: FIRST_SONG.into() }
    }
}

/// A song's folder name: lower-case letters, digits and `-`, so it cannot name anything outside the projects folder.
pub fn is_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// Where a song's project file is kept (the host's state store): the first song under the original key.
pub fn state_key(slug: &str) -> String {
    if slug == FIRST_SONG { "hilldaw".to_string() } else { format!("hilldaw-p-{slug}") }
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
        // the song list: only valid, distinct folder names, always containing the open song, never empty
        let mut seen = Vec::new();
        for s in v.projects.drain(..) {
            if is_slug(&s) && !seen.contains(&s) {
                seen.push(s);
            }
        }
        v.projects = seen;
        if !is_slug(&v.current_project) {
            v.current_project = v.projects.first().cloned().unwrap_or_else(|| FIRST_SONG.into());
        }
        if !v.projects.contains(&v.current_project) {
            v.projects.push(v.current_project.clone());
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
        let s = Settings { input_device: Some("1234:00ab".into()), output_device: Some("0b0e:00ff".into()), projects: vec!["song".into(), "song-2".into()], current_project: "song-2".into() };
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

    #[test]
    fn the_song_list_is_cleaned_and_always_has_the_open_song() {
        let d = Settings::from_json("");
        assert_eq!((d.projects.clone(), d.current_project.as_str()), (vec!["song".to_string()], "song"));
        let s = Settings::from_json(r#"{"projects":["a","../b","a","UP","c d","x1"],"current_project":"zz"}"#);
        assert_eq!(s.projects, vec!["a", "x1", "zz"], "invalid and repeated names dropped, the open song added");
        let s = Settings::from_json(r#"{"projects":["a"],"current_project":"../../etc"}"#);
        assert_eq!(s.current_project, "a");
        assert_eq!(state_key("song"), "hilldaw");
        assert_eq!(state_key("song-2"), "hilldaw-p-song-2");
        // settings from before songs existed still load
        let old = Settings::from_json(r#"{"input_device":"1234:00ab"}"#);
        assert_eq!((old.input_device.as_deref(), old.current_project.as_str()), (Some("1234:00ab"), "song"));
    }
}

