//! HillBeat presets -> HillDAW [`DrumTrack`].
//!
//! The preset data, the row-to-sound planning (which sample plays which drum, flams, accents, tom pitch) and
//! the kit matching all come from the owner's `hill_core::presets`; nothing is re-implemented or invented here.
//! This crate only loads the planned sounds from a [`Host`] and packs them into `hilldaw_core::drums` types.

pub mod host;

pub use hill_core::{Host, MemHost};

use hill_core::presets::{self, Kit, Preset, Sound};
use hill_core::{library, synth, wav};
use hilldaw_core::drums::{DrumPattern, DrumSection, DrumTrack, DrumVoice, VOICES};
use hilldaw_core::project::DrumsModel;
use std::collections::HashMap;
use std::sync::Arc;

/// A preset laid out for the DAW, plus the kit it ended up using (`None` = built-in sounds).
#[derive(Clone)]
pub struct LoadedPattern {
    pub pattern: Arc<DrumPattern>,
    pub kit: Option<String>,
}

/// Loads and caches patterns for one sample library.
pub struct DrumLibrary {
    host: Box<dyn Host>,
    kits: Vec<Kit>,
    sounds: HashMap<String, Option<DrumVoice>>,
    patterns: HashMap<String, LoadedPattern>,
}

pub fn find_preset(id: &str) -> Option<&'static Preset> {
    presets::builtin().patterns.iter().find(|p| p.id == id)
}

impl DrumLibrary {
    /// Finds the drum kits in the host's sample folders the same way HillBeat does.
    pub fn new(host: Box<dyn Host>) -> Self {
        let paths = host.list_samples();
        let kits = library::kit_entries(&paths)
            .iter()
            .filter_map(|e| match &e.kind {
                library::Kind::Kit(files) => {
                    let dir = e.key.strip_prefix("kit:").unwrap_or(&e.key);
                    Some(Kit::new(dir.rsplit('/').next().unwrap_or(dir), files))
                }
                _ => None,
            })
            .collect();
        DrumLibrary { host, kits, sounds: HashMap::new(), patterns: HashMap::new() }
    }

    /// The sample/state host this library reads from (the app saves its project through it).
    pub fn host_mut(&mut self) -> &mut dyn Host {
        self.host.as_mut()
    }

    pub fn kit_names(&self) -> Vec<&str> {
        self.kits.iter().map(|k| k.name.as_str()).collect()
    }

    fn load_sound(&mut self, s: &Sound) -> Option<DrumVoice> {
        let key = match s {
            Sound::Builtin(n) => format!("builtin:{n}"),
            Sound::File(p) => p.clone(),
        };
        if let Some(v) = self.sounds.get(&key) {
            return v.clone();
        }
        let sample = match s {
            Sound::Builtin(n) => synth::builtin_by_name(n).and_then(|i| i.first().cloned()),
            Sound::File(p) => self.host.read_sample(p).as_deref().and_then(wav::parse),
        };
        let voice = sample.map(|s| {
            let mut data = Vec::with_capacity(s.frames() * 2);
            for i in 0..s.frames() {
                let (l, r) = s.frame(i);
                data.push(l);
                data.push(r);
            }
            DrumVoice { data: Arc::new(data), rate: s.rate }
        });
        self.sounds.insert(key, voice.clone());
        voice
    }

    /// Lay out preset `id` on the DAW's 8 drum voices.
    pub fn pattern(&mut self, id: &str) -> Result<LoadedPattern, String> {
        if let Some(p) = self.patterns.get(id) {
            return Ok(p.clone());
        }
        let preset = find_preset(id).ok_or_else(|| format!("unknown drum preset {id:?}"))?;
        let plan = preset.plan(&self.kits, VOICES);
        let mut pat = DrumPattern::silent(preset.steps, preset.div);
        for (vi, v) in plan.voices.iter().enumerate().take(VOICES) {
            // a sound that fails to load leaves its voice silent rather than failing the whole song
            pat.voices[vi] = self.load_sound(&v.sound);
            pat.cells[vi] = plan.cells[vi].clone();
            pat.pitch[vi] = plan.pitch[vi].clone();
        }
        let loaded = LoadedPattern { pattern: Arc::new(pat), kit: plan.kit };
        self.patterns.insert(id.to_string(), loaded.clone());
        Ok(loaded)
    }

    /// Build the drum track for a project: its sections, swing, per-voice volume and mute.
    pub fn build_track(&mut self, m: &DrumsModel, bpm: f64) -> Result<DrumTrack, String> {
        let mut t = DrumTrack::new(bpm);
        t.swing = m.swing;
        t.vol = m.vol;
        t.mute = m.mute;
        t.gain = m.gain;
        t.start = m.start;
        for s in &m.sections {
            let p = self.pattern(&s.preset)?;
            t.sections.push(DrumSection { name: s.name.clone(), pattern: p.pattern, cycles: s.cycles });
        }
        Ok(t)
    }
}

/// All presets as `(id, name, style)`, in the owner's order.
pub fn all_presets() -> Vec<(&'static str, &'static str, &'static str)> {
    presets::builtin().patterns.iter().map(|p| (p.id.as_str(), p.name.as_str(), p.style.as_str())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hill_core::MemHost;
    use hilldaw_core::mixer::Source;
    use hilldaw_core::project::DrumSectionModel;

    const R: u32 = 44100;

    fn empty() -> DrumLibrary {
        DrumLibrary::new(Box::new(MemHost::default()))
    }

    /// A constant-level mono WAV: each kit file gets its own level so a test can tell which one played.
    fn const_wav(level: f32, frames: usize) -> Vec<u8> {
        wav::encode16(&vec![level; frames * 2], 2, R)
    }

    fn kit_host() -> MemHost {
        let mut h = MemHost::default();
        for (name, level) in [("Test Kit/kick.wav", 0.50), ("Test Kit/snare.wav", 0.25), ("Test Kit/hat closed.wav", 0.125), ("Test Kit/hat open.wav", 0.0625)] {
            h.samples.insert(name.into(), const_wav(level, 300));
        }
        h
    }

    /// First preset whose rows include a kick, a snare and a closed hat on sixteenths.
    fn rock_like() -> &'static Preset {
        presets::builtin()
            .patterns
            .iter()
            .find(|p| p.div == 4 && p.steps == 16 && ["BD", "SD", "CH"].iter().all(|r| p.tracks.iter().any(|(k, _)| k == r)))
            .expect("a kick/snare/hat preset exists")
    }

    fn audible_frames(t: &mut DrumTrack, frames: usize) -> usize {
        let mut o = vec![0.0; frames * 2];
        t.render_add(0, &mut o, R);
        o.chunks_exact(2).filter(|f| f[0] != 0.0 || f[1] != 0.0).count()
    }

    #[test]
    fn every_shipped_preset_loads_with_builtin_sounds_and_is_audible() {
        let mut lib = empty();
        let all = presets::builtin();
        assert!(all.patterns.len() > 200, "the owner's preset set is present ({})", all.patterns.len());
        for p in &all.patterns {
            let lp = lib.pattern(&p.id).unwrap_or_else(|e| panic!("{}: {e}", p.id));
            let pat = &lp.pattern;
            assert_eq!((pat.steps, pat.div), (p.steps, p.div), "{} length/feel", p.id);
            assert_eq!((pat.cells.len(), pat.voices.len()), (VOICES, VOICES), "{}", p.id);
            assert!(lp.kit.is_none(), "{}: no kits in an empty library", p.id);
            let hits: usize = pat.cells.iter().flatten().filter(|c| **c & 0x7f != 0).count();
            assert!(hits > 0, "{} has no hits", p.id);
            for vi in 0..VOICES {
                let used = pat.cells[vi].iter().any(|c| *c != 0);
                assert!(!used || pat.voices[vi].is_some(), "{} voice {vi} has hits but no sound", p.id);
            }
        }
    }

    #[test]
    fn a_kit_in_the_library_supplies_the_sounds() {
        let mut lib = DrumLibrary::new(Box::new(kit_host()));
        assert_eq!(lib.kit_names(), vec!["Test Kit"]);
        let p = rock_like();
        let lp = lib.pattern(&p.id).unwrap();
        assert_eq!(lp.kit.as_deref(), Some("Test Kit"));
        let level_of = |vi: usize| lp.pattern.voices[vi].as_ref().map(|v| v.data[0]);
        let plan = p.plan(&lib.kits, VOICES);
        let find = |row: &'static str| plan.voices.iter().position(|v| v.rows.contains(&row)).unwrap();
        // 16-bit quantisation: compare with a tolerance
        assert!((level_of(find("BD")).unwrap() - 0.50).abs() < 1e-3, "kick voice plays the kit's kick");
        assert!((level_of(find("SD")).unwrap() - 0.25).abs() < 1e-3, "snare voice plays the kit's snare");
        assert!((level_of(find("CH")).unwrap() - 0.125).abs() < 1e-3, "hat voice plays the closed hat");
    }

    #[test]
    fn unknown_preset_is_an_error_not_a_silent_song() {
        assert!(empty().pattern("no-such-preset").err().expect("must fail").contains("no-such-preset"));
    }

    #[test]
    fn project_model_becomes_a_track_with_sections_and_voice_settings() {
        let id = &rock_like().id;
        let mut m = DrumsModel {
            sections: vec![
                DrumSectionModel { name: "Verse".into(), preset: id.clone(), cycles: 2 },
                DrumSectionModel { name: "Again".into(), preset: id.clone(), cycles: 1 },
            ],
            ..Default::default()
        };
        m.swing = 0.1;
        m.vol[3] = 2.0;
        m.mute[4] = true;
        let mut t = empty().build_track(&m, 100.0).unwrap();
        assert_eq!((t.sections.len(), t.swing, t.vol[3], t.mute[4]), (2, 0.1, 2.0, true));
        // 3 cycles of 16 sixteenths at 100 bpm
        assert_eq!(t.total_frames(R), (3.0 * 16.0 * 44100.0 * 60.0 / 100.0 / 4.0_f64).round() as u64);
        assert!(audible_frames(&mut t, 200_000) > 1000);
        m.sections[1].preset = "nope".into();
        assert!(empty().build_track(&m, 100.0).is_err());
    }

    #[test]
    fn muting_every_voice_makes_silence() {
        let m = DrumsModel {
            sections: vec![DrumSectionModel { name: "A".into(), preset: rock_like().id.clone(), cycles: 1 }],
            mute: [true; 8],
            ..Default::default()
        };
        assert_eq!(audible_frames(&mut empty().build_track(&m, 120.0).unwrap(), 100_000), 0);
    }

    #[test]
    fn dirhost_lists_wavs_and_refuses_paths_outside_the_folder() {
        let dir = std::env::temp_dir().join(format!("pdrums-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Kit")).unwrap();
        std::fs::write(dir.join("Kit/kick.WAV"), const_wav(0.5, 10)).unwrap();
        std::fs::write(dir.join("Kit/notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("secret.txt"), b"x").unwrap();
        let h = host::DirHost::new(dir.join("Kit"));
        assert_eq!(h.list_samples(), vec!["kick.WAV".to_string()]);
        assert!(h.read_sample("kick.WAV").is_some());
        assert!(h.read_sample("../secret.txt").is_none());
        assert!(h.read_sample("/etc/hosts").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
