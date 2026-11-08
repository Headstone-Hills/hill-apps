//! Several songs, and recordings that never made it into one.
//!
//! **Songs.** Each song is a folder under the projects folder (`<root>/<slug>/audio/...`) and a project file in the host's state store
//! (the first song under the original key, the others under `hilldaw-p-<slug>`). The list of songs and the open one are device settings
//! (`Settings.projects`, `Settings.current_project`). The browser (FN+R1) lists them, opens one (saving the open one first), makes a new
//! one, or takes one off the list (its files stay on the card: nothing here deletes audio).
//!
//! **Recovery.** A recording that was running when the power failed has its WAV on the card but no take in any project, because the take is
//! only filed when the recording stops. [`HillDaw::refresh_orphans`] finds such files (in the open song's `audio/` folder, not named by any
//! take) and the Takes tab offers them back one at a time (Y+X), oldest first, as a new take on the picked track.

use super::*;
pub use hilldaw_core::settings::FIRST_SONG;
use hilldaw_core::settings::{is_slug, state_key};

/// One line of the song browser.
#[derive(Clone, Debug, PartialEq)]
pub struct SongRow {
    pub slug: String,
    pub name: String,
    pub bpm: f64,
    /// recordings the song has (takes on all tracks)
    pub takes: usize,
    pub open: bool,
}

impl HillDaw {
    // ---- recovery ----------------------------------------------------------------------------------------------------

    /// Find recordings on the card that no take names, in the open song.
    pub(crate) fn refresh_orphans(&mut self) {
        let Some(r) = self.recorder.as_mut() else {
            self.orphans.clear();
            return;
        };
        let used: std::collections::HashSet<&str> = self.project.tracks.iter().flat_map(|t| t.takes.iter().map(|k| k.file.as_str()).chain(t.clips.iter().map(|c| c.file.as_str()))).collect();
        self.orphans = r.list_recordings().into_iter().filter(|(f, _)| !used.contains(f.as_str())).collect();
    }

    pub fn orphan_count(&self) -> usize {
        self.orphans.len()
    }

    /// Put the oldest unplaced recording on `strip` as its playing take (at the start of the song, with the latency offset applied; move it
    /// with the Clip tab if the take began elsewhere: where it was is not recorded in the file).
    pub(crate) fn recover_orphan(&mut self, strip: usize) {
        if strip >= MAX_TRACKS || self.rec.is_some() {
            return;
        }
        if self.orphans.is_empty() {
            self.toast.say("No unused recordings");
            return;
        }
        let (file, len) = self.orphans.remove(0);
        let offset = self.project.latency_offset_frames.max(0) as u64;
        let frames = len.saturating_sub(offset);
        if frames == 0 {
            self.toast.say("That recording is too short");
            return;
        }
        self.checkpoint("recover", NO_COALESCE);
        let tm = &mut self.project.tracks[strip];
        tm.takes.push(TakeModel { file: file.clone(), frames, latency_comp_frames: offset as i64, start: 0, offset, placed: true, ..Default::default() });
        let idx = tm.takes.len() - 1;
        self.audio_cache.remove(&file);
        self.peak_cache.remove(&file);
        if self.set_active_take(strip, idx).is_ok() {
            self.take_sel = idx;
            let left = self.orphans.len();
            let name = file.rsplit('/').next().unwrap_or(&file).to_string();
            self.toast.say(&hill_core::fb::clip(&format!("Recovered {name}{}", if left > 0 { format!(" ({left} more)") } else { String::new() }), 30));
            self.save();
        }
    }

    // ---- songs -------------------------------------------------------------------------------------------------------

    pub fn song_slug(&self) -> &str {
        &self.settings.current_project
    }

    pub fn song_rows(&self) -> &[SongRow] {
        &self.songs
    }

    pub fn song_sel(&self) -> usize {
        self.song_sel
    }

    /// Open the song browser (not while recording or exporting).
    pub(crate) fn open_songs(&mut self) {
        if self.rec.is_some() || self.export_job.is_some() {
            self.toast.say("Stop recording or exporting first");
            return;
        }
        self.build_song_rows();
        self.song_sel = self.songs.iter().position(|s| s.open).unwrap_or(0);
        self.song_confirm = None;
        self.view = View::Songs;
    }

    fn build_song_rows(&mut self) {
        let cur = self.settings.current_project.clone();
        let list = self.settings.projects.clone();
        let mut rows = Vec::new();
        for slug in list {
            let open = slug == cur;
            let info = if open {
                Some(self.project.clone())
            } else {
                self.lib.host_mut().load_state(&state_key(&slug)).and_then(|t| Project::from_json(&t).ok())
            };
            let (name, bpm, takes) = match &info {
                Some(p) => (p.name.clone(), p.bpm, p.tracks.iter().map(|t| t.takes.len()).sum()),
                None => (slug.to_uppercase(), 0.0, 0), // listed but never saved
            };
            rows.push(SongRow { slug, name, bpm, takes, open });
        }
        self.songs = rows;
    }

    fn save_settings(&mut self) {
        let json = self.settings.to_json();
        self.lib.host_mut().save_state(SETTINGS_KEY, &json);
    }

    pub(crate) fn songs_act(&mut self, b: Btn) {
        let n = self.songs.len() + 1; // the last row makes a new song
        self.song_sel = self.song_sel.min(n - 1);
        match b {
            Btn::Up => self.song_sel = self.song_sel.saturating_sub(1),
            Btn::Down => self.song_sel = (self.song_sel + 1).min(n - 1),
            Btn::B => self.view = View::Mixer,
            Btn::A if self.song_sel == n - 1 => self.new_song(),
            Btn::A => {
                let slug = self.songs[self.song_sel].slug.clone();
                if slug == self.settings.current_project {
                    self.view = View::Mixer;
                } else {
                    self.open_song(&slug);
                }
            }
            Btn::X if self.song_sel < n - 1 => {
                let s = self.songs[self.song_sel].clone();
                if s.open {
                    self.toast.say("The open song cannot be removed");
                } else if self.song_confirm == Some(self.song_sel) {
                    self.song_confirm = None;
                    self.settings.projects.retain(|p| *p != s.slug);
                    self.save_settings();
                    self.build_song_rows();
                    self.song_sel = self.song_sel.min(self.songs.len());
                    self.toast.say("Removed from the list (files stay on the card)");
                } else {
                    self.song_confirm = Some(self.song_sel);
                    self.toast.say("X again: take it off the list (files stay)");
                }
            }
            _ => {}
        }
        if b != Btn::X {
            self.song_confirm = None;
        }
    }

    /// A new, empty song named SONG n, opened at once.
    fn new_song(&mut self) {
        let slug = (2u32..)
            .map(|n| format!("song-{n}"))
            .find(|s| !self.settings.projects.contains(s) && self.lib.host_mut().load_state(&state_key(s)).is_none())
            .unwrap_or_else(|| "song-x".into());
        if self.settings.projects.len() >= 40 {
            self.toast.say("Too many songs: remove one first");
            return;
        }
        self.settings.projects.push(slug.clone());
        self.open_song(&slug);
    }

    /// Save the open song, then switch to `slug` (its project file, its folder of recordings). Everything built from a song is rebuilt and
    /// undo history starts over: it belonged to the other song.
    pub(crate) fn open_song(&mut self, slug: &str) {
        if !is_slug(slug) || self.rec.is_some() || self.export_job.is_some() {
            return;
        }
        self.settle();
        self.save();
        if self.monitoring {
            if let Some(r) = self.recorder.as_mut() {
                let _ = r.monitor(false);
            }
            self.monitoring = false;
        }
        self.settings.current_project = slug.to_string();
        if !self.settings.projects.iter().any(|p| p == slug) {
            self.settings.projects.push(slug.to_string());
        }
        self.save_settings();
        if let Some(r) = self.recorder.as_mut() {
            r.set_project(slug);
        }
        let project = match self.lib.host_mut().load_state(&state_key(slug)) {
            Some(text) => match Project::from_json(&text) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("hilldaw: {slug}: {e}");
                    self.toast.say("That song is unreadable: new one");
                    Self::fresh_named(slug)
                }
            },
            None => Self::fresh_named(slug),
        };
        self.install_project(project);
        self.view = View::Mixer;
        self.toast.say(&hill_core::fb::clip(&format!("Opened {}", self.project.name), 30));
    }

    fn fresh_named(slug: &str) -> Project {
        let mut p = Self::fresh_project();
        if slug != FIRST_SONG {
            p.name = slug.strip_prefix("song-").map_or_else(|| slug.to_uppercase(), |n| format!("SONG {n}"));
        }
        p
    }

    /// Make `p` the open project and rebuild everything that comes from a project (the same work as starting the app with it).
    fn install_project(&mut self, mut p: Project) {
        let before = p.drums.sections.len();
        p.drums.sections.retain(|s| find_preset(&s.preset).is_some());
        if p.drums.sections.len() != before {
            self.toast.say("Some drum sections used unknown presets: removed");
        }
        self.transport.playing = false;
        self.transport.pos = 0;
        self.project = p;
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
        self.last_commit = None;
        self.coalesce_touch = None;
        self.audio_cache.clear();
        self.peak_cache.clear();
        self.armed = [false; MAX_TRACKS];
        (self.sel, self.take_sel, self.clip_sel, self.track_row, self.waves_start) = (0, 0, 0, 0, 0);
        self.confirm_delete = None;
        self.hold = [0.0; STRIPS];
        self.hold_master = 0.0;
        self.transport.bpm = self.project.bpm;
        self.transport.beats_per_bar = self.project.beats_per_bar;
        self.apply_loop();
        self.rebuild_click();
        self.rebuild_drums();
        self.apply_mixer();
        self.apply_all_fx();
        for i in 0..MAX_TRACKS {
            self.rebuild_track(i);
        }
        // the FM-1 keeps whatever it was last told: send this song's voice and effect values
        let fm1 = self.project.fm1.clone();
        if fm1.program_set {
            self.fm1_send(Fm1Msg::Program(fm1.program));
        }
        for (cc, value) in fm1.fx.iter() {
            self.fm1_send(Fm1Msg::FxCc { cc: *cc, value: *value });
        }
        self.refresh_orphans();
        if !self.orphans.is_empty() {
            self.toast.say(&format!("{} unused recording(s): Takes, Y+X", self.orphans.len()));
        }
    }
}
