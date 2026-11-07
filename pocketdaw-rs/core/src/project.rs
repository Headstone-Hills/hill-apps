//! Versioned project model. A project is a folder: `project.json` + an `audio/` directory of WAV takes.
//! Paths inside are relative so a project can be copied between SD cards. Edits (trim, fades, gain,
//! reverse) are metadata only; recorded audio is never overwritten.

use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 1;
pub const MAX_TRACKS: usize = 8;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct ClipModel {
    /// relative to the project folder, e.g. `audio/t1-take3.wav`
    pub file: String,
    /// where the clip starts on the timeline (frames)
    pub start: u64,
    /// trim: first frame used inside the file
    pub offset: u64,
    /// frames used (None = to the end of the file)
    pub len: Option<u64>,
    pub gain: f32,
    pub fade_in: u64,
    pub fade_out: u64,
    pub reverse: bool,
}

impl Default for ClipModel {
    fn default() -> Self {
        ClipModel { file: String::new(), start: 0, offset: 0, len: None, gain: 1.0, fade_in: 0, fade_out: 0, reverse: false }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct TakeModel {
    pub file: String,
    pub frames: u64,
    /// latency compensation that was applied when this take was placed (frames)
    pub latency_comp_frames: i64,
}

/// A recorded or edited note on a track (frames on the project timeline). Played by a sampler track
/// or sent back to the FM-1 as MIDI.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteModel {
    pub start: u64,
    pub len: u64,
    pub note: u8,
    pub vel: u8,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct TrackModel {
    pub name: String,
    pub color: u32,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub clips: Vec<ClipModel>,
    /// every recording is kept; `active_take` says which one the track currently plays
    pub takes: Vec<TakeModel>,
    pub active_take: Option<usize>,
    pub notes: Vec<NoteModel>,
}

impl Default for TrackModel {
    fn default() -> Self {
        TrackModel { name: String::new(), color: 0x4aa3ff, gain: 1.0, pan: 0.0, mute: false, solo: false, clips: vec![], takes: vec![], active_take: None, notes: vec![] }
    }
}

/// One arrangement section: a HillBeat preset repeated `cycles` times. The preset is stored by its **id**
/// (a string), never by index, so adding presets upstream cannot silently change an old project.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct DrumSectionModel {
    pub name: String,
    pub preset: String,
    pub cycles: u32,
}

impl Default for DrumSectionModel {
    fn default() -> Self {
        DrumSectionModel { name: String::new(), preset: String::new(), cycles: 1 }
    }
}

pub const MAX_DRUM_SECTIONS: usize = 64;
pub const MAX_SECTION_CYCLES: u32 = 256;

/// The drum track. Tempo is the project's (constant per project; tempo automation is out of scope).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct DrumsModel {
    pub enabled: bool,
    pub sections: Vec<DrumSectionModel>,
    /// 0..=0.25: fraction of a step by which odd steps are delayed
    pub swing: f32,
    /// per drum voice, 0..=4 (up to 400%, like HillBeat)
    pub vol: [f32; 8],
    pub mute: [bool; 8],
    pub gain: f32,
    /// where the first section starts on the timeline (frames)
    pub start: u64,
}

impl Default for DrumsModel {
    fn default() -> Self {
        DrumsModel { enabled: true, sections: vec![], swing: 0.0, vol: [1.0; 8], mute: [false; 8], gain: 1.0, start: 0 }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub rate: u32,
    pub bpm: f64,
    pub beats_per_bar: u32,
    /// calibrated + manual recording offset, in frames (positive = captured audio arrives late)
    pub latency_offset_frames: i64,
    pub loop_region: Option<(u64, u64)>,
    pub tracks: Vec<TrackModel>,
    pub drums: DrumsModel,
}

impl Default for Project {
    fn default() -> Self {
        Project {
            version: FORMAT_VERSION,
            name: "UNTITLED".into(),
            rate: crate::RATE,
            bpm: 120.0,
            beats_per_bar: 4,
            latency_offset_frames: 0,
            loop_region: None,
            tracks: (1..=MAX_TRACKS).map(|i| TrackModel { name: format!("TRACK {i}"), ..Default::default() }).collect(),
            drums: DrumsModel::default(),
        }
    }
}

/// Only plain relative paths inside the project folder are accepted.
pub fn is_safe_relative(p: &str) -> bool {
    !p.is_empty() && !p.starts_with('/') && !p.contains('\\') && !p.split('/').any(|c| c == ".." || c.is_empty() || c == ".") && !p.contains(':')
}

impl Project {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("project serialises")
    }

    /// Parse, refuse files from the future, and clamp everything into range.
    pub fn from_json(s: &str) -> Result<Project, String> {
        let mut p: Project = serde_json::from_str(s).map_err(|e| format!("bad project file: {e}"))?;
        if p.version > FORMAT_VERSION {
            return Err(format!("project is format v{}, this build reads up to v{}", p.version, FORMAT_VERSION));
        }
        p.sanitize()?;
        Ok(p)
    }

    pub fn sanitize(&mut self) -> Result<(), String> {
        self.bpm = self.bpm.clamp(20.0, 300.0);
        self.beats_per_bar = self.beats_per_bar.clamp(1, 16);
        self.rate = if self.rate == 0 { crate::RATE } else { self.rate };
        self.latency_offset_frames = self.latency_offset_frames.clamp(-(self.rate as i64), self.rate as i64);
        self.tracks.truncate(MAX_TRACKS);
        while self.tracks.len() < MAX_TRACKS {
            self.tracks.push(TrackModel { name: format!("TRACK {}", self.tracks.len() + 1), ..Default::default() });
        }
        for t in self.tracks.iter_mut() {
            t.gain = t.gain.clamp(0.0, 4.0);
            t.pan = t.pan.clamp(-1.0, 1.0);
            if t.active_take.is_some_and(|a| a >= t.takes.len()) {
                t.active_take = None;
            }
            t.notes.retain(|n| n.len > 0 && n.note < 128 && (1..=127).contains(&n.vel));
            for f in t.clips.iter().map(|c| &c.file).chain(t.takes.iter().map(|k| &k.file)) {
                if !is_safe_relative(f) {
                    return Err(format!("unsafe path in project: {f:?}"));
                }
            }
        }
        let d = &mut self.drums;
        d.sections.truncate(MAX_DRUM_SECTIONS);
        for sec in d.sections.iter_mut() {
            sec.cycles = sec.cycles.clamp(1, MAX_SECTION_CYCLES);
        }
        d.swing = if d.swing.is_finite() { d.swing.clamp(0.0, 0.25) } else { 0.0 };
        d.gain = if d.gain.is_finite() { d.gain.clamp(0.0, 4.0) } else { 1.0 };
        for v in d.vol.iter_mut() {
            *v = if v.is_finite() { v.clamp(0.0, 4.0) } else { 1.0 };
        }
        if let Some((s, e)) = self.loop_region {
            if e <= s {
                self.loop_region = None;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrips_and_has_eight_tracks() {
        let p = Project::default();
        assert_eq!(p.tracks.len(), 8);
        assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
    }

    #[test]
    fn full_roundtrip_with_clips_and_takes() {
        let mut p = Project { name: "SONG".into(), bpm: 143.5, loop_region: Some((44100, 441000)), latency_offset_frames: 512, ..Default::default() };
        p.tracks[0].takes.push(TakeModel { file: "audio/t1-take1.wav".into(), frames: 88200, latency_comp_frames: 512 });
        p.tracks[0].active_take = Some(0);
        p.tracks[0].clips.push(ClipModel { file: "audio/t1-take1.wav".into(), start: 22050, offset: 100, len: Some(5000), fade_in: 64, reverse: true, ..Default::default() });
        let q = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(q, p);
    }

    #[test]
    fn missing_fields_take_defaults_so_old_files_still_load() {
        let p = Project::from_json(r#"{"version":1,"name":"OLD","bpm":100}"#).unwrap();
        assert_eq!((p.name.as_str(), p.bpm, p.tracks.len(), p.rate), ("OLD", 100.0, 8, 44100));
    }

    #[test]
    fn future_versions_and_garbage_are_refused() {
        assert!(Project::from_json(r#"{"version":99}"#).unwrap_err().contains("v99"));
        assert!(Project::from_json("{nope").is_err());
    }

    #[test]
    fn sanitize_clamps_and_drops_bad_references() {
        let mut p = Project { bpm: 9999.0, ..Default::default() };
        p.tracks[1].pan = 5.0;
        p.tracks[1].active_take = Some(3);
        p.loop_region = Some((10, 5));
        p.tracks.push(TrackModel::default());
        let q = Project::from_json(&p.to_json()).unwrap();
        assert_eq!((q.bpm, q.tracks[1].pan, q.tracks[1].active_take, q.loop_region, q.tracks.len()), (300.0, 1.0, None, None, 8));
    }

    #[test]
    fn notes_roundtrip_old_files_still_load_and_bad_notes_are_dropped() {
        let mut p = Project::default();
        p.tracks[3].notes = vec![
            NoteModel { start: 0, len: 22050, note: 60, vel: 90 },
            NoteModel { start: 100, len: 0, note: 62, vel: 90 },   // zero length
            NoteModel { start: 100, len: 10, note: 200, vel: 90 }, // not a MIDI note
            NoteModel { start: 100, len: 10, note: 64, vel: 0 },   // velocity 0 is a note-off
        ];
        let q = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(q.tracks[3].notes, vec![NoteModel { start: 0, len: 22050, note: 60, vel: 90 }]);
        let old = Project::from_json(r#"{"version":1,"tracks":[{"name":"A"}]}"#).unwrap();
        assert!(old.tracks[0].notes.is_empty());
    }

    #[test]
    fn drum_sections_roundtrip_by_preset_id_and_old_files_have_none() {
        let mut p = Project::default();
        p.drums.sections = vec![
            DrumSectionModel { name: "Verse".into(), preset: "basic-rock".into(), cycles: 4 },
            DrumSectionModel { name: "Fill".into(), preset: "funk-1".into(), cycles: 1 },
        ];
        p.drums.vol[2] = 2.5;
        p.drums.mute[5] = true;
        p.drums.swing = 0.12;
        assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
        let old = Project::from_json(r#"{"version":1,"name":"OLD"}"#).unwrap();
        assert!(old.drums.sections.is_empty() && old.drums.vol == [1.0; 8]);
    }

    #[test]
    fn drum_settings_are_clamped() {
        let mut p = Project::default();
        p.drums.sections = vec![DrumSectionModel { cycles: 0, ..Default::default() }; 100];
        p.drums.sections[1].cycles = 99999;
        p.drums.swing = 0.9;
        p.drums.vol[0] = 50.0;
        p.drums.vol[1] = -3.0;
        let q = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(q.drums.sections.len(), MAX_DRUM_SECTIONS);
        assert_eq!((q.drums.sections[0].cycles, q.drums.sections[1].cycles), (1, MAX_SECTION_CYCLES));
        assert_eq!((q.drums.swing, q.drums.vol[0], q.drums.vol[1]), (0.25, 4.0, 0.0));
    }

    #[test]
    fn paths_must_stay_inside_the_project() {
        for ok in ["audio/a.wav", "a.wav", "audio/sub/b.wav"] {
            assert!(is_safe_relative(ok), "{ok}");
        }
        for bad in ["/etc/passwd", "../x.wav", "audio/../../x", "a\\b", "", "audio//x", "C:x", "./a"] {
            assert!(!is_safe_relative(bad), "{bad}");
        }
        let mut p = Project::default();
        p.tracks[0].clips.push(ClipModel { file: "../evil.wav".into(), ..Default::default() });
        assert!(Project::from_json(&p.to_json()).is_err());
    }
}
