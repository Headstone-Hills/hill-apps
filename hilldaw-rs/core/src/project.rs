//! Versioned project model. A project is a folder: `project.json` + an `audio/` directory of WAV takes.
//! Paths inside are relative so a project can be copied between SD cards. Edits (trim, fades, gain,
//! reverse) are metadata only; recorded audio is never overwritten.

use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 1;
pub const MAX_TRACKS: usize = 8;
/// Highest strip gain: +24 dB. Quiet FM-1 recordings (peaks around -20 dBFS) need far more than the old +12 dB; the master limiter catches the rest.
pub const MAX_STRIP_GAIN: f32 = 16.0;

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
    /// Where the take sits on the timeline and where it starts in its file, so any take in the history can be made the
    /// playing one again. `placed` is false for takes from before this was stored (they cannot be switched to).
    pub start: u64,
    pub offset: u64,
    pub placed: bool,
    /// edits made on the Clip tab, kept with the take so switching takes and back does not lose them
    pub fade_in: u64,
    pub fade_out: u64,
    pub reverse: bool,
    /// the take's full clip list once it has been split or edited with several clips (empty = the single clip the fields above describe)
    pub clips: Vec<ClipModel>,
}

/// The per-track effect chain, in plain numbers. 0 means "off" for the filters and the delay/reverb amounts.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct TrackFxModel {
    /// low-pass cutoff in Hz (0 = off)
    pub lowpass_hz: u32,
    /// high-pass cutoff in Hz (0 = off)
    pub highpass_hz: u32,
    pub delay_ms: u32,
    pub delay_feedback: f32,
    /// 0 = no delay
    pub delay_mix: f32,
    /// amount sent to the shared reverb, 0..1 (0 = none)
    pub reverb_send: f32,
}

impl Default for TrackFxModel {
    fn default() -> Self {
        TrackFxModel { lowpass_hz: 0, highpass_hz: 0, delay_ms: 300, delay_feedback: 0.35, delay_mix: 0.0, reverb_send: 0.0 }
    }
}

impl TrackFxModel {
    pub fn is_off(&self) -> bool {
        self.lowpass_hz == 0 && self.highpass_hz == 0 && self.delay_mix <= 0.0 && self.reverb_send <= 0.0
    }

    /// Pull every value into its range; a damaged file never reaches the filters with nonsense.
    pub fn sanitize(&mut self) {
        let unit = |v: f32, d: f32, max: f32| if v.is_finite() { v.clamp(0.0, max) } else { d };
        self.lowpass_hz = if self.lowpass_hz == 0 { 0 } else { self.lowpass_hz.clamp(100, 20_000) };
        self.highpass_hz = if self.highpass_hz == 0 { 0 } else { self.highpass_hz.clamp(10, 4_000) };
        self.delay_ms = self.delay_ms.clamp(10, 1_000);
        self.delay_feedback = unit(self.delay_feedback, 0.35, 0.9);
        self.delay_mix = unit(self.delay_mix, 0.0, 1.0);
        self.reverb_send = unit(self.reverb_send, 0.0, 1.0);
    }
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
    pub fx: TrackFxModel,
}

impl Default for TrackModel {
    fn default() -> Self {
        TrackModel { name: String::new(), color: 0x4aa3ff, gain: 1.0, pan: 0.0, mute: false, solo: false, clips: vec![], takes: vec![], active_take: None, notes: vec![], fx: TrackFxModel::default() }
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
    /// the drum kit this section plays ("" = the automatic choice for its style)
    pub kit: String,
}

impl Default for DrumSectionModel {
    fn default() -> Self {
        DrumSectionModel { name: String::new(), preset: String::new(), cycles: 1, kit: String::new() }
    }
}

/// What the FM-1 should be set to. Only what the user has chosen is kept (and sent), so connecting never overrides the
/// unit's own patch with invented defaults.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct Fm1Model {
    /// Program Change 0-127 (voice 001-128); only sent when `program_set`
    pub program: u8,
    pub program_set: bool,
    /// FX CC (70-93) -> value in that parameter's own range
    pub fx: std::collections::BTreeMap<u8, u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    #[default]
    Wav,
    Flac,
    Mp3,
}

impl ExportFormat {
    pub fn ext(self) -> &'static str {
        match self {
            ExportFormat::Wav => "wav",
            ExportFormat::Flac => "flac",
            ExportFormat::Mp3 => "mp3",
        }
    }
}

/// How "export" writes its files.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct ExportModel {
    /// WAV, or WAV converted to FLAC / MP3 by the encoders on the device
    pub format: ExportFormat,
    /// 16 or 24 (the WAV that is rendered; MP3 ignores it)
    pub bits: u16,
    /// dither when writing 16-bit (24-bit needs none)
    pub dither: bool,
    /// raise or lower the mix so its peak is -1 dBFS
    pub normalize: bool,
    /// also write one file per track and one for the drums
    pub stems: bool,
    /// include the drum track (off: the mixdown and the stems are made without it)
    pub drums: bool,
}

impl Default for ExportModel {
    fn default() -> Self {
        ExportModel { format: ExportFormat::Wav, bits: 24, dither: true, normalize: false, stems: false, drums: true }
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
    /// the drum strip's pan and solo (`enabled == false` is its mute)
    pub pan: f32,
    pub solo: bool,
    /// where the first section starts on the timeline (frames)
    pub start: u64,
    /// the transport wraps at the end of the drum arrangement, so a groove keeps going instead of stopping after its last bar
    pub loop_arrangement: bool,
    pub fx: TrackFxModel,
}

impl Default for DrumsModel {
    fn default() -> Self {
        DrumsModel { enabled: true, sections: vec![], swing: 0.0, vol: [1.0; 8], mute: [false; 8], gain: 1.0, pan: 0.0, solo: false, start: 0, loop_arrangement: true, fx: TrackFxModel::default() }
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
    /// one bar of click before recording starts
    pub count_in: bool,
    pub loop_region: Option<(u64, u64)>,
    /// record only between these frames (punch in / punch out)
    pub punch: Option<(u64, u64)>,
    /// size of the shared reverb, 0..1
    pub reverb_room: f32,
    pub tracks: Vec<TrackModel>,
    pub drums: DrumsModel,
    pub fm1: Fm1Model,
    pub export: ExportModel,
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
            count_in: true,
            loop_region: None,
            punch: None,
            reverb_room: 0.5,
            tracks: (1..=MAX_TRACKS).map(|i| TrackModel { name: format!("TRACK {i}"), ..Default::default() }).collect(),
            drums: DrumsModel::default(),
            fm1: Fm1Model::default(),
            export: ExportModel::default(),
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
            t.gain = t.gain.clamp(0.0, MAX_STRIP_GAIN);
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
        d.gain = if d.gain.is_finite() { d.gain.clamp(0.0, MAX_STRIP_GAIN) } else { 1.0 };
        if self.export.bits != 16 && self.export.bits != 24 {
            self.export.bits = 24;
        }
        self.fm1.program = self.fm1.program.min(127);
        // keep only known FX controls, each inside its own range
        let map = crate::fm1::fx_map();
        self.fm1.fx.retain(|cc, _| map.by_cc(*cc).is_some());
        for (cc, v) in self.fm1.fx.iter_mut() {
            if let Some(p) = map.by_cc(*cc) {
                *v = p.clamp(*v);
            }
        }
        d.pan = if d.pan.is_finite() { d.pan.clamp(-1.0, 1.0) } else { 0.0 };
        for v in d.vol.iter_mut() {
            *v = if v.is_finite() { v.clamp(0.0, 4.0) } else { 1.0 };
        }
        for t in self.tracks.iter_mut() {
            t.fx.sanitize();
        }
        self.drums.fx.sanitize();
        self.reverb_room = if self.reverb_room.is_finite() { self.reverb_room.clamp(0.0, 1.0) } else { 0.5 };
        if let Some((s, e)) = self.punch {
            if e <= s {
                self.punch = None;
            }
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
        p.tracks[0].takes.push(TakeModel { file: "audio/t1-take1.wav".into(), frames: 88200, latency_comp_frames: 512, ..Default::default() });
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
            DrumSectionModel { name: "Verse".into(), preset: "basic-rock".into(), cycles: 4, ..Default::default() },
            DrumSectionModel { name: "Fill".into(), preset: "funk-1".into(), cycles: 1, ..Default::default() },
        ];
        p.drums.vol[2] = 2.5;
        p.drums.mute[5] = true;
        p.drums.swing = 0.12;
        assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
        let old = Project::from_json(r#"{"version":1,"name":"OLD"}"#).unwrap();
        assert!(old.drums.sections.is_empty() && old.drums.vol == [1.0; 8]);
    }

    #[test]
    fn fm1_settings_roundtrip_old_files_load_and_bad_values_are_fixed() {
        let mut p = Project::default();
        p.fm1.program = 5;
        p.fm1.program_set = true;
        p.fm1.fx.insert(70, 1);
        p.fm1.fx.insert(72, 80);
        assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
        assert!(!Project::from_json(r#"{"version":1}"#).unwrap().fm1.program_set, "old files: nothing is sent to the FM-1");
        let mut bad = Project::default();
        bad.fm1.program = 200;
        bad.fm1.fx.insert(71, 99); // filter type 0-2
        bad.fm1.fx.insert(5, 1); // not an FX CC
        let q = Project::from_json(&bad.to_json()).unwrap();
        assert_eq!((q.fm1.program, q.fm1.fx.get(&71).copied(), q.fm1.fx.contains_key(&5)), (127, Some(2), false));
    }

    #[test]
    fn export_settings_roundtrip_default_to_24_bit_and_bad_bit_depths_are_fixed() {
        let mut p = Project::default();
        assert_eq!((p.export.bits, p.export.dither, p.export.normalize, p.export.stems), (24, true, false, false));
        p.export = ExportModel { format: ExportFormat::Mp3, bits: 16, dither: false, normalize: true, stems: true, drums: false };
        assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
        p.export.bits = 8;
        assert_eq!(Project::from_json(&p.to_json()).unwrap().export.bits, 24);
        assert_eq!(Project::from_json(r#"{"version":1}"#).unwrap().export, ExportModel::default(), "old files");
    }

    #[test]
    fn fx_punch_and_take_placement_roundtrip_old_files_load_and_bad_values_are_fixed() {
        let mut p = Project::default();
        p.tracks[2].fx = TrackFxModel { lowpass_hz: 4000, highpass_hz: 80, delay_ms: 250, delay_feedback: 0.5, delay_mix: 0.3, reverb_send: 0.4 };
        p.drums.fx.reverb_send = 0.2;
        p.punch = Some((88200, 176400));
        p.reverb_room = 0.8;
        p.tracks[0].takes.push(TakeModel { file: "audio/rec-001.wav".into(), frames: 100, latency_comp_frames: 563, start: 10, offset: 573, placed: true, ..Default::default() });
        p.export.format = ExportFormat::Flac;
        assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
        let old = Project::from_json(r#"{"version":1,"tracks":[{"takes":[{"file":"a.wav","frames":5}]}]}"#).unwrap();
        assert!(!old.tracks[0].takes[0].placed && old.tracks[0].fx.is_off() && old.punch.is_none() && old.export.format == ExportFormat::Wav);
        let mut bad = Project::default();
        bad.tracks[0].fx = TrackFxModel { lowpass_hz: 5, highpass_hz: 99_999, delay_ms: 0, delay_feedback: 7.0, delay_mix: 5.0, reverb_send: -3.0 };
        bad.punch = Some((500, 100));
        bad.reverb_room = 9.0;
        let q = Project::from_json(&bad.to_json()).unwrap();
        let f = &q.tracks[0].fx;
        assert_eq!((f.lowpass_hz, f.highpass_hz, f.delay_ms), (100, 4000, 10));
        assert!(f.delay_feedback == 0.9 && f.delay_mix == 1.0 && f.reverb_send == 0.0 && q.punch.is_none() && q.reverb_room == 1.0);
    }

    #[test]
    fn strip_gain_goes_up_to_plus_24_db_and_no_further() {
        let mut p = Project::default();
        p.tracks[0].gain = 15.0;
        p.tracks[1].gain = 99.0;
        p.drums.gain = 99.0;
        let q = Project::from_json(&p.to_json()).unwrap();
        assert_eq!((q.tracks[0].gain, q.tracks[1].gain, q.drums.gain), (15.0, MAX_STRIP_GAIN, MAX_STRIP_GAIN));
        assert!((20.0 * MAX_STRIP_GAIN.log10() - 24.08).abs() < 0.01);
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
