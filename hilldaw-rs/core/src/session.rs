//! Record session: decides which captured frames belong to which take.
//!
//! Pure logic, driven by the same `(position, length)` spans that `Transport::advance` produces for the audio
//! callback. Handles count-in, punch in/out, loop passes (each pass is its own take, so nothing is overwritten)
//! and jumps. The FM-1's audio reaches us `latency_frames` late, so each span of timeline maps to capture frames
//! shifted later by that amount; a take's file then starts exactly at its `timeline_start`.

#[derive(Clone, Debug, Default)]
pub struct RecordConfig {
    /// the metronome plays this many frames before recording starts (the transport does not move meanwhile)
    pub count_in_frames: u64,
    /// record only `[in, out)` of the timeline
    pub punch: Option<(u64, u64)>,
    /// how late captured audio arrives (frames), from latency calibration
    pub latency_frames: u64,
    /// end the recording when the loop wraps instead of starting another take
    pub stop_on_wrap: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    CountIn,
    Recording,
    Stopped,
}

/// A run of capture frames to write into one take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub take: usize,
    /// index into the capture stream (frames since the session began recording)
    pub cap_start: u64,
    pub len: u64,
    pub timeline_start: u64,
}

impl Segment {
    pub fn cap_end(&self) -> u64 {
        self.cap_start + self.len
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TakePlan {
    pub index: usize,
    /// where the take's first frame belongs on the timeline
    pub timeline_start: u64,
    /// capture index of its first frame
    pub cap_start: u64,
    pub frames: u64,
}

pub struct RecordSession {
    cfg: RecordConfig,
    phase: Phase,
    count_in_left: u64,
    pub takes: Vec<TakePlan>,
    cur: Option<usize>,
    spent: u64,
    last_end: Option<u64>,
}

impl RecordSession {
    pub fn new(cfg: RecordConfig) -> Self {
        RecordSession { count_in_left: cfg.count_in_frames, cfg, phase: Phase::Idle, takes: Vec::new(), cur: None, spent: 0, last_end: None }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Arm and start: goes to count-in if one is configured, otherwise straight to recording.
    pub fn begin(&mut self) {
        self.phase = if self.count_in_left > 0 { Phase::CountIn } else { Phase::Recording };
    }

    pub fn count_in_left(&self) -> u64 {
        self.count_in_left
    }

    /// Account for `frames` of count-in click. Returns how many of them were count-in; the rest (if the count-in
    /// ended inside this chunk) are the first frames of real recording and should be fed to [`Self::on_span`].
    pub fn tick_count_in(&mut self, frames: u64) -> u64 {
        if self.phase != Phase::CountIn {
            return 0;
        }
        let used = frames.min(self.count_in_left);
        self.count_in_left -= used;
        if self.count_in_left == 0 {
            self.phase = Phase::Recording;
        }
        used
    }

    /// Feed one transport span. Returns the capture segment to record for it, if any.
    pub fn on_span(&mut self, pos: u64, len: u64) -> Option<Segment> {
        if self.phase != Phase::Recording || len == 0 {
            return None;
        }
        if self.last_end.is_some_and(|e| e != pos) {
            // the loop wrapped (or the user seeked): the current take ends here
            self.cur = None;
            if self.cfg.stop_on_wrap {
                self.phase = Phase::Stopped;
                return None;
            }
        }
        let (w0, w1) = self.cfg.punch.unwrap_or((0, u64::MAX));
        let (from, to) = (pos.max(w0), (pos + len).min(w1));
        let mut seg = None;
        if from < to {
            let cap_start = self.spent + (from - pos) + self.cfg.latency_frames;
            let idx = match self.cur {
                Some(i) => i,
                None => {
                    self.takes.push(TakePlan { index: self.takes.len(), timeline_start: from, cap_start, frames: 0 });
                    self.cur = Some(self.takes.len() - 1);
                    self.takes.len() - 1
                }
            };
            self.takes[idx].frames += to - from;
            seg = Some(Segment { take: idx, cap_start, len: to - from, timeline_start: from });
            if to < pos + len {
                self.cur = None; // the punch-out point was inside this span
            }
        } else {
            self.cur = None;
        }
        self.spent += len;
        self.last_end = Some(pos + len);
        seg
    }

    /// End the session. Returns the takes that were recorded and the capture index the writer must reach
    /// (the capture stream lags, so keep reading until it has this many frames).
    pub fn stop(&mut self) -> (Vec<TakePlan>, u64) {
        self.phase = Phase::Stopped;
        self.cur = None;
        let need = self.takes.iter().map(|t| t.cap_start + t.frames).max().unwrap_or(0);
        (self.takes.clone(), need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Transport;

    /// run a transport through the session in `chunk`-sized callbacks for `total` frames
    fn run(s: &mut RecordSession, t: &mut Transport, chunk: usize, total: usize) -> Vec<Segment> {
        let (mut segs, mut spans, mut done) = (Vec::new(), Vec::new(), 0);
        while done < total {
            let n = chunk.min(total - done);
            t.advance(n, &mut spans);
            for &(p, l) in &spans {
                segs.extend(s.on_span(p, l as u64));
            }
            done += n;
        }
        segs
    }
    fn playing(pos: u64) -> Transport {
        let mut t = Transport::new(44100);
        t.playing = true;
        t.pos = pos;
        t
    }

    #[test]
    fn plain_recording_is_one_take_with_contiguous_capture_shifted_by_the_latency() {
        let mut s = RecordSession::new(RecordConfig { latency_frames: 563, ..Default::default() });
        s.begin();
        let segs = run(&mut s, &mut playing(10_000), 1024, 3 * 44100);
        assert_eq!(s.takes.len(), 1);
        let t = &s.takes[0];
        assert_eq!((t.timeline_start, t.cap_start, t.frames), (10_000, 563, 3 * 44100));
        assert!(segs.windows(2).all(|w| w[0].cap_end() == w[1].cap_start), "contiguous in the capture stream");
        let (takes, need) = s.stop();
        assert_eq!(takes.len(), 1);
        assert_eq!(need, 563 + 3 * 44100, "keep reading the capture until the delayed audio has arrived");
    }

    #[test]
    fn result_does_not_depend_on_the_callback_size() {
        let mk = |chunk| {
            let mut s = RecordSession::new(RecordConfig { latency_frames: 100, punch: Some((5000, 90_000)), ..Default::default() });
            s.begin();
            run(&mut s, &mut playing(0), chunk, 120_000);
            s.stop().0
        };
        assert_eq!(mk(1024), mk(77));
        assert_eq!(mk(1024), mk(120_000));
    }

    #[test]
    fn punch_in_and_out_record_only_the_window_even_mid_chunk() {
        let mut s = RecordSession::new(RecordConfig { punch: Some((5000, 8200)), ..Default::default() });
        s.begin();
        run(&mut s, &mut playing(0), 1024, 20_000);
        assert_eq!(s.takes.len(), 1);
        assert_eq!((s.takes[0].timeline_start, s.takes[0].frames), (5000, 3200));
        // the capture index of a punched take counts from the start of the session, not from the punch-in
        assert_eq!(s.takes[0].cap_start, 5000);
        // a second pass of the punch window (after a jump back) is a new take
        let mut s = RecordSession::new(RecordConfig { punch: Some((5000, 8200)), ..Default::default() });
        s.begin();
        let mut t = playing(0);
        run(&mut s, &mut t, 512, 10_000);
        t.pos = 4000;
        run(&mut s, &mut t, 512, 6000);
        assert_eq!(s.takes.len(), 2);
    }

    #[test]
    fn each_loop_pass_is_its_own_take_so_nothing_is_overwritten() {
        let mut t = playing(1000);
        t.loop_region = Some((1000, 4000));
        let mut s = RecordSession::new(RecordConfig::default());
        s.begin();
        run(&mut s, &mut t, 700, 3 * 3000);
        assert_eq!(s.takes.len(), 3);
        assert!(s.takes.iter().all(|k| k.timeline_start == 1000 && k.frames == 3000));
        assert_eq!(s.takes.iter().map(|k| k.cap_start).collect::<Vec<_>>(), vec![0, 3000, 6000]);
        // same result with a chunk that straddles the wrap
        let mut t2 = playing(1000);
        t2.loop_region = Some((1000, 4000));
        let mut s2 = RecordSession::new(RecordConfig::default());
        s2.begin();
        run(&mut s2, &mut t2, 1024, 3 * 3000);
        assert_eq!(s2.takes, s.takes);
    }

    #[test]
    fn stop_on_wrap_ends_after_the_first_pass() {
        let mut t = playing(1000);
        t.loop_region = Some((1000, 4000));
        let mut s = RecordSession::new(RecordConfig { stop_on_wrap: true, ..Default::default() });
        s.begin();
        run(&mut s, &mut t, 500, 10_000);
        assert_eq!((s.takes.len(), s.takes[0].frames, s.phase()), (1, 3000, Phase::Stopped));
    }

    #[test]
    fn count_in_precedes_recording_and_hands_over_mid_chunk() {
        let mut s = RecordSession::new(RecordConfig { count_in_frames: 88_200, ..Default::default() });
        assert_eq!(s.phase(), Phase::Idle);
        s.begin();
        assert_eq!(s.phase(), Phase::CountIn);
        assert_eq!(s.on_span(0, 1024), None, "spans are ignored during the count-in");
        let mut used = 0;
        while s.phase() == Phase::CountIn {
            used += s.tick_count_in(1024);
        }
        assert_eq!(used, 88_200);
        assert_eq!(s.count_in_left(), 0);
        assert_eq!(s.tick_count_in(100), 0);
        assert!(s.on_span(0, 1024).is_some());
        assert_eq!(s.takes[0].cap_start, 0);
    }

    #[test]
    fn nothing_is_recorded_when_idle_or_stopped_and_stop_with_no_take_needs_no_capture() {
        let mut s = RecordSession::new(RecordConfig::default());
        assert_eq!(s.on_span(0, 100), None);
        s.begin();
        assert_eq!(s.on_span(0, 0), None);
        let (takes, need) = s.stop();
        assert!((takes.len(), need) == (0, 0));
        assert_eq!(s.on_span(0, 100), None);
    }
}
