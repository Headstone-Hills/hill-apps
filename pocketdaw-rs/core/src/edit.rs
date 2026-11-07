//! Edits and undo/redo. Every edit is applied to the [`Project`] and returns its own inverse, so a
//! history is just two stacks of edits. Nothing here touches audio files: trim, fades, gain, reverse,
//! split and move are all metadata (the recorded WAV is never rewritten).

use crate::project::{ClipModel, Project, TakeModel, TrackModel};
use crate::transport::Transport;

#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Replace a clip (trim, fades, gain, reverse and move are all expressed this way).
    SetClip { track: usize, clip: usize, new: ClipModel },
    InsertClip { track: usize, index: usize, clip: ClipModel },
    DeleteClip { track: usize, index: usize },
    /// Mixer settings and name of a track (clips and takes are untouched).
    SetTrack { track: usize, name: String, gain: f32, pan: f32, mute: bool, solo: bool },
    AddTake { track: usize, take: TakeModel },
    PopTake { track: usize },
    SetActiveTake { track: usize, take: Option<usize> },
    SetTempo { bpm: f64 },
    /// Several edits as one undo step. Applied atomically: if one fails the earlier ones are rolled back.
    Batch(Vec<Edit>),
}

pub fn track_params(t: &TrackModel, track: usize) -> Edit {
    Edit::SetTrack { track, name: t.name.clone(), gain: t.gain, pan: t.pan, mute: t.mute, solo: t.solo }
}

fn track_mut(p: &mut Project, i: usize) -> Result<&mut TrackModel, String> {
    p.tracks.get_mut(i).ok_or_else(|| format!("no track {i}"))
}

impl Edit {
    /// Apply to `p`; on success returns the edit that undoes it.
    pub fn apply(self, p: &mut Project) -> Result<Edit, String> {
        match self {
            Edit::SetClip { track, clip, new } => {
                let c = track_mut(p, track)?.clips.get_mut(clip).ok_or_else(|| format!("no clip {clip} on track {track}"))?;
                let old = std::mem::replace(c, new);
                Ok(Edit::SetClip { track, clip, new: old })
            }
            Edit::InsertClip { track, index, clip } => {
                let t = track_mut(p, track)?;
                if index > t.clips.len() {
                    return Err(format!("insert index {index} out of range"));
                }
                t.clips.insert(index, clip);
                Ok(Edit::DeleteClip { track, index })
            }
            Edit::DeleteClip { track, index } => {
                let t = track_mut(p, track)?;
                if index >= t.clips.len() {
                    return Err(format!("no clip {index} on track {track}"));
                }
                let old = t.clips.remove(index);
                Ok(Edit::InsertClip { track, index, clip: old })
            }
            Edit::SetTrack { track, name, gain, pan, mute, solo } => {
                let t = track_mut(p, track)?;
                let inv = track_params(t, track);
                t.name = name;
                t.gain = gain.clamp(0.0, 4.0);
                t.pan = pan.clamp(-1.0, 1.0);
                t.mute = mute;
                t.solo = solo;
                Ok(inv)
            }
            Edit::AddTake { track, take } => {
                track_mut(p, track)?.takes.push(take);
                Ok(Edit::PopTake { track })
            }
            Edit::PopTake { track } => {
                let t = track_mut(p, track)?;
                let prev_active = t.active_take;
                let removed = t.takes.pop().ok_or("no take to remove")?;
                if t.active_take.is_some_and(|a| a >= t.takes.len()) {
                    t.active_take = None;
                }
                // inverse: put the take back, then restore which take was active
                Ok(Edit::Batch(vec![Edit::AddTake { track, take: removed }, Edit::SetActiveTake { track, take: prev_active }]))
            }
            Edit::SetActiveTake { track, take } => {
                let t = track_mut(p, track)?;
                if take.is_some_and(|k| k >= t.takes.len()) {
                    return Err("no such take".into());
                }
                let old = std::mem::replace(&mut t.active_take, take);
                Ok(Edit::SetActiveTake { track, take: old })
            }
            Edit::SetTempo { bpm } => {
                let old = p.bpm;
                p.bpm = bpm.clamp(20.0, 300.0);
                Ok(Edit::SetTempo { bpm: old })
            }
            Edit::Batch(edits) => {
                let mut inverses: Vec<Edit> = Vec::with_capacity(edits.len());
                for e in edits {
                    match e.apply(p) {
                        Ok(inv) => inverses.push(inv),
                        Err(msg) => {
                            for inv in inverses.into_iter().rev() {
                                let _ = inv.apply(p); // roll back
                            }
                            return Err(msg);
                        }
                    }
                }
                inverses.reverse();
                Ok(Edit::Batch(inverses))
            }
        }
    }
}

/// How many frames a clip plays: its explicit `len`, or the rest of the file after `offset`.
pub fn effective_len(c: &ClipModel, file_frames: u64) -> u64 {
    c.len.unwrap_or_else(|| file_frames.saturating_sub(c.offset))
}

/// Split a clip at timeline frame `at` into two clips that together play exactly what the original did.
/// Fade-in stays with the first part, fade-out with the second. Reversed clips split correctly
/// (the earlier-in-time half is the *later* part of the file region).
pub fn split(p: &Project, track: usize, clip: usize, at: u64, file_frames: u64) -> Result<Edit, String> {
    let c = p.tracks.get(track).and_then(|t| t.clips.get(clip)).ok_or("no such clip")?;
    let len = effective_len(c, file_frames);
    if at <= c.start || at >= c.start + len {
        return Err("split point is not inside the clip".into());
    }
    let k = at - c.start; // frames in the first (earlier) half
    let (mut a, mut b) = (c.clone(), c.clone());
    a.len = Some(k);
    a.fade_out = 0;
    b.start = at;
    b.len = Some(len - k);
    b.fade_in = 0;
    if c.reverse {
        a.offset = c.offset + (len - k);
        b.offset = c.offset;
    } else {
        b.offset = c.offset + k;
    }
    Ok(Edit::Batch(vec![Edit::SetClip { track, clip, new: a }, Edit::InsertClip { track, index: clip + 1, clip: b }]))
}

/// Chop a clip at every 1/`div`-beat grid line that falls strictly inside it.
pub fn slice_on_grid(p: &Project, tr: &Transport, track: usize, clip: usize, div: u32, file_frames: u64) -> Result<Edit, String> {
    let c = p.tracks.get(track).and_then(|t| t.clips.get(clip)).ok_or("no such clip")?;
    let len = effective_len(c, file_frames);
    let (s, e) = (c.start, c.start + len);
    let step = tr.frames_per_beat() / div.max(1) as f64;
    let mut cuts = Vec::new();
    let mut k = (s as f64 / step).floor() as i64;
    loop {
        k += 1;
        let at = (k as f64 * step).round() as u64;
        if at >= e {
            break;
        }
        if at > s && cuts.last() != Some(&at) {
            cuts.push(at);
        }
    }
    // cut from the last position backwards so earlier clip indices stay valid
    let mut scratch = p.clone();
    let mut edits = Vec::new();
    for at in cuts.iter().rev() {
        let sp = split(&scratch, track, clip, *at, file_frames);
        // after each split the left part keeps index `clip`; the next (earlier) cut must split the left part
        match sp {
            Ok(ed) => {
                ed.clone().apply(&mut scratch)?;
                edits.push(ed);
            }
            Err(_) => continue,
        }
    }
    if edits.is_empty() {
        return Err("no grid line falls inside the clip".into());
    }
    Ok(Edit::Batch(edits))
}

/// Gain that brings `peak` (linear, 0..1) up to `target_db` dBFS. Returns 1.0 for silence.
pub fn normalize_gain(peak: f32, target_db: f32) -> f32 {
    if peak <= 1e-9 { 1.0 } else { 10f32.powf(target_db / 20.0) / peak }
}

/// Undo/redo. `done` holds the inverses of applied edits; `undone` holds the inverses of undone ones.
pub struct History {
    done: Vec<Edit>,
    undone: Vec<Edit>,
    pub max_depth: usize,
}

impl Default for History {
    fn default() -> Self {
        History { done: Vec::new(), undone: Vec::new(), max_depth: 100 }
    }
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Apply a new edit. Clears the redo stack.
    pub fn apply(&mut self, p: &mut Project, e: Edit) -> Result<(), String> {
        let inv = e.apply(p)?;
        self.done.push(inv);
        if self.done.len() > self.max_depth {
            self.done.remove(0);
        }
        self.undone.clear();
        Ok(())
    }

    pub fn undo(&mut self, p: &mut Project) -> Result<bool, String> {
        let Some(inv) = self.done.pop() else { return Ok(false) };
        let redo = inv.apply(p)?;
        self.undone.push(redo);
        Ok(true)
    }

    pub fn redo(&mut self, p: &mut Project) -> Result<bool, String> {
        let Some(e) = self.undone.pop() else { return Ok(false) };
        let inv = e.apply(p)?;
        self.done.push(inv);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixer::{Mixer, Source, Track as MixTrack, TrimmedClip};
    use std::sync::Arc;

    fn clip(file: &str, start: u64) -> ClipModel {
        ClipModel { file: file.into(), start, ..Default::default() }
    }

    fn project() -> Project {
        let mut p = Project::default();
        p.tracks[0].clips.push(ClipModel { start: 1000, len: Some(500), ..clip("audio/a.wav", 1000) });
        p
    }

    #[test]
    fn every_edit_undoes_and_redoes_exactly() {
        let mut p = project();
        let orig = p.clone();
        let mut h = History::default();
        let edits = vec![
            Edit::SetClip { track: 0, clip: 0, new: ClipModel { gain: 0.5, fade_in: 10, reverse: true, ..p.tracks[0].clips[0].clone() } },
            Edit::InsertClip { track: 1, index: 0, clip: clip("audio/b.wav", 5) },
            Edit::SetTrack { track: 2, name: "BASS".into(), gain: 0.8, pan: -0.5, mute: true, solo: false },
            Edit::AddTake { track: 0, take: TakeModel { file: "audio/t.wav".into(), frames: 10, latency_comp_frames: 3 } },
            Edit::SetTempo { bpm: 90.0 },
            Edit::DeleteClip { track: 0, index: 0 },
        ];
        let mut states = vec![p.clone()];
        for e in edits {
            h.apply(&mut p, e).unwrap();
            states.push(p.clone());
        }
        for i in (0..states.len() - 1).rev() {
            assert!(h.undo(&mut p).unwrap());
            assert_eq!(p, states[i], "undo to state {i}");
        }
        assert_eq!(p, orig);
        assert!(!h.undo(&mut p).unwrap());
        for s in states.iter().skip(1) {
            assert!(h.redo(&mut p).unwrap());
            assert_eq!(&p, s);
        }
        assert!(!h.redo(&mut p).unwrap());
    }

    #[test]
    fn removing_a_take_is_undoable_and_restores_the_active_take() {
        let mut p = project();
        let mut h = History::default();
        for f in ["audio/1.wav", "audio/2.wav"] {
            h.apply(&mut p, Edit::AddTake { track: 0, take: TakeModel { file: f.into(), frames: 5, latency_comp_frames: 0 } }).unwrap();
        }
        h.apply(&mut p, Edit::SetActiveTake { track: 0, take: Some(1) }).unwrap();
        let with_two = p.clone();
        h.apply(&mut p, Edit::PopTake { track: 0 }).unwrap();
        assert_eq!((p.tracks[0].takes.len(), p.tracks[0].active_take), (1, None));
        h.undo(&mut p).unwrap();
        assert_eq!(p, with_two, "undoing the removal brings the take and the active selection back");
        h.redo(&mut p).unwrap();
        assert_eq!(p.tracks[0].takes.len(), 1);
    }

    #[test]
    fn new_edit_clears_redo_and_depth_is_capped() {
        let mut p = project();
        let mut h = History { max_depth: 3, ..Default::default() };
        for b in [100.0, 110.0, 120.0, 130.0, 140.0] {
            h.apply(&mut p, Edit::SetTempo { bpm: b }).unwrap();
        }
        let mut n = 0;
        while h.undo(&mut p).unwrap() {
            n += 1;
        }
        assert_eq!(n, 3);
        assert_eq!(p.bpm, 110.0, "only the last 3 edits are undoable: 130, 120 and 110 are restored in turn");
        h.redo(&mut p).unwrap();
        h.apply(&mut p, Edit::SetTempo { bpm: 77.0 }).unwrap();
        assert!(!h.can_redo());
    }

    #[test]
    fn failed_batch_rolls_back_everything() {
        let mut p = project();
        let orig = p.clone();
        let e = Edit::Batch(vec![
            Edit::SetTempo { bpm: 99.0 },
            Edit::InsertClip { track: 0, index: 1, clip: clip("audio/x.wav", 0) },
            Edit::DeleteClip { track: 0, index: 42 }, // fails
        ]);
        assert!(e.apply(&mut p).is_err());
        assert_eq!(p, orig);
        assert!(Edit::SetClip { track: 9, clip: 0, new: ClipModel::default() }.apply(&mut p).is_err());
    }

    fn render(p: &Project, data: &Arc<Vec<f32>>, frames: usize) -> Vec<f32> {
        let mut m = Mixer::new(44100, frames);
        m.limiter.ceiling = 100.0;
        let mut t = MixTrack::new("t");
        t.pan = 0.0;
        for c in &p.tracks[0].clips {
            let len = effective_len(c, (data.len() / 2) as u64);
            t.sources.push(Box::new(TrimmedClip { data: data.clone(), start: c.start, offset: c.offset, len, gain: c.gain, fade_in: c.fade_in, fade_out: c.fade_out, reverse: c.reverse }));
        }
        m.tracks.push(t);
        let mut out = vec![0.0; frames * 2];
        m.render(0, &mut out);
        out
    }

    fn file() -> Arc<Vec<f32>> {
        Arc::new((0..4000).map(|i| (i as f32 * 0.013).sin() * 0.5).collect()) // 2000 stereo frames
    }

    #[test]
    fn split_plays_exactly_what_the_original_did_forward_and_reversed() {
        for reverse in [false, true] {
            let data = file();
            let mut p = Project::default();
            p.tracks[0].clips.push(ClipModel { file: "audio/a.wav".into(), start: 300, offset: 200, len: Some(1000), reverse, gain: 0.8, ..Default::default() });
            let before = render(&p, &data, 2000);
            for at in [301u64, 777, 1299] {
                let mut q = p.clone();
                let e = split(&q, 0, 0, at, 2000).unwrap();
                e.apply(&mut q).unwrap();
                assert_eq!(q.tracks[0].clips.len(), 2);
                assert_eq!(render(&q, &data, 2000), before, "reverse={reverse} split at {at}");
            }
        }
    }

    #[test]
    fn split_rejects_points_outside_and_keeps_fades_on_the_right_halves() {
        let mut p = Project::default();
        p.tracks[0].clips.push(ClipModel { file: "a".into(), start: 100, len: Some(100), fade_in: 5, fade_out: 7, ..Default::default() });
        assert!(split(&p, 0, 0, 100, 1000).is_err());
        assert!(split(&p, 0, 0, 200, 1000).is_err());
        split(&p, 0, 0, 150, 1000).unwrap().apply(&mut p).unwrap();
        let c = &p.tracks[0].clips;
        assert_eq!((c[0].fade_in, c[0].fade_out, c[1].fade_in, c[1].fade_out), (5, 0, 0, 7));
        assert_eq!((c[0].len, c[1].start, c[1].len, c[1].offset), (Some(50), 150, Some(50), 50));
    }

    #[test]
    fn fades_reverse_and_trim_render_correctly() {
        let data = Arc::new((0..2000).map(|_| 1.0f32).collect::<Vec<_>>()); // constant 1.0 so gain is visible
        let mut p = Project::default();
        p.tracks[0].clips.push(ClipModel { file: "a".into(), start: 10, offset: 0, len: Some(100), fade_in: 10, fade_out: 20, ..Default::default() });
        let o = render(&p, &data, 200);
        let l = |i: usize| o[2 * i] / std::f32::consts::FRAC_1_SQRT_2; // undo the -3 dB centre pan
        assert_eq!(o[2 * 9], 0.0);
        assert!(l(10).abs() < 1e-6, "first frame of a fade-in is silent");
        assert!((l(15) - 0.5).abs() < 1e-5);
        assert!((l(30) - 1.0).abs() < 1e-5);
        assert!(l(109).abs() < 1e-6, "last frame of a fade-out is silent");
        assert_eq!(o[2 * 110], 0.0);
        // trim past the file end is silence, not a panic
        let mut q = Project::default();
        q.tracks[0].clips.push(ClipModel { file: "a".into(), start: 0, offset: 990, len: Some(100), ..Default::default() });
        render(&q, &data, 100);
    }

    #[test]
    fn slice_on_grid_cuts_at_every_grid_line_and_undoes_in_one_step() {
        let data = file();
        let mut p = Project::default();
        p.tracks[0].clips.push(ClipModel { file: "a".into(), start: 0, len: Some(2000), ..Default::default() });
        let mut tr = Transport::new(44100);
        tr.bpm = 60.0 * 44100.0 / 500.0; // 500 frames per beat
        assert!((tr.frames_per_beat() - 500.0).abs() < 1e-6);
        let before = render(&p, &data, 2000);
        let orig = p.clone();
        let e = slice_on_grid(&p, &tr, 0, 0, 1, 2000).unwrap();
        let mut h = History::default();
        h.apply(&mut p, e).unwrap();
        let starts: Vec<u64> = p.tracks[0].clips.iter().map(|c| c.start).collect();
        assert_eq!(starts, vec![0, 500, 1000, 1500]);
        assert_eq!(render(&p, &data, 2000), before, "slicing changes nothing audible");
        // rearrange: swap the first two slices on the timeline
        let (a, b) = (p.tracks[0].clips[0].clone(), p.tracks[0].clips[1].clone());
        h.apply(&mut p, Edit::Batch(vec![
            Edit::SetClip { track: 0, clip: 0, new: ClipModel { start: b.start, ..a } },
            Edit::SetClip { track: 0, clip: 1, new: ClipModel { start: 0, ..b } },
        ])).unwrap();
        assert_ne!(render(&p, &data, 2000), before);
        h.undo(&mut p).unwrap();
        h.undo(&mut p).unwrap();
        assert_eq!(p, orig, "the slice is a single undo step");
        // a clip that no grid line falls inside cannot be sliced
        let mut short = Project::default();
        short.tracks[0].clips.push(ClipModel { file: "a".into(), start: 10, len: Some(100), ..Default::default() });
        assert!(slice_on_grid(&short, &tr, 0, 0, 1, 2000).is_err());
    }

    #[test]
    fn normalize_gain_targets_the_peak() {
        assert!((normalize_gain(0.25, 0.0) - 4.0).abs() < 1e-5);
        assert!((normalize_gain(0.5, -6.0206) - 1.0).abs() < 1e-3);
        assert_eq!(normalize_gain(0.0, 0.0), 1.0);
    }

    // keep the Source trait in scope for the compiler even if tests change
    #[allow(dead_code)]
    fn _assert_source<T: Source>() {}
}
