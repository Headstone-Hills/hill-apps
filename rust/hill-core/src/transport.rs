//! Sample-accurate step clock: 16th-note steps with optional swing.

pub struct Transport {
    pub bpm: f32,
    /// Fraction of a step by which odd steps are delayed, 0.0..=0.25 (50%..75% swing).
    pub swing: f32,
    pub swing_on: bool,
    pub playing: bool,
    pub steps: usize,
    next: usize,
    pos: f64,
    rate: f32,
    last: Option<usize>,
}

impl Transport {
    pub fn new(rate: f32, steps: usize) -> Self {
        Transport { bpm: 120.0, swing: 0.1, swing_on: false, playing: false, steps, next: 0, pos: 0.0, rate, last: None }
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
    }

    fn step_len(&self) -> f64 {
        self.rate as f64 * 60.0 / self.bpm as f64 / 4.0
    }

    /// Change tempo, keeping the current phase within the bar.
    pub fn set_bpm(&mut self, bpm: f32) {
        let bpm = bpm.clamp(40.0, 240.0);
        let old = self.step_len();
        self.bpm = bpm;
        self.pos *= self.step_len() / old;
    }

    pub fn swing_percent(&self) -> i32 {
        50 + (self.swing * 100.0).round() as i32
    }

    fn due_time(&self) -> f64 {
        let sl = self.step_len();
        if self.next >= self.steps {
            return self.steps as f64 * sl;
        }
        let off = if self.swing_on && self.next % 2 == 1 { self.swing as f64 * sl } else { 0.0 };
        self.next as f64 * sl + off
    }

    /// Frames until the next step fires (u64::MAX when stopped).
    pub fn until_due(&self) -> u64 {
        if !self.playing {
            return u64::MAX;
        }
        (self.due_time() - self.pos).max(0.0).ceil() as u64
    }

    pub fn advance(&mut self, n: u64) {
        if self.playing {
            self.pos += n as f64;
        }
    }

    /// If a step is due, consume and return its index.
    pub fn take_due(&mut self) -> Option<usize> {
        if !self.playing {
            return None;
        }
        if self.next >= self.steps {
            let end = self.due_time();
            if self.pos + 1e-9 >= end {
                self.pos -= end;
                self.next = 0;
            } else {
                return None;
            }
        }
        if self.pos + 1e-9 >= self.due_time() {
            let s = self.next;
            self.next += 1;
            self.last = Some(s);
            Some(s)
        } else {
            None
        }
    }

    /// Index of the step most recently fired (for the playhead).
    pub fn playhead(&self) -> Option<usize> {
        self.last
    }

    pub fn toggle(&mut self) {
        self.playing = !self.playing;
    }

    /// Stop and rewind to step 1.
    pub fn stop_reset(&mut self) {
        self.playing = false;
        self.pos = 0.0;
        self.next = 0;
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run for `frames` frames, returning (frame, step) for each fired step.
    fn run(t: &mut Transport, frames: u64) -> Vec<(u64, usize)> {
        let mut out = Vec::new();
        let mut now = 0u64;
        while now < frames {
            while let Some(s) = t.take_due() {
                out.push((now, s));
            }
            let chunk = t.until_due().min(frames - now).max(1);
            t.advance(chunk);
            now += chunk;
        }
        out
    }

    #[test]
    fn steps_are_evenly_spaced() {
        let mut t = Transport::new(44100.0, 16);
        t.bpm = 120.0; // 16th = 5512.5 frames
        t.playing = true;
        let ev = run(&mut t, 44100 * 2);
        assert_eq!(ev[0], (0, 0));
        assert_eq!(ev[1].1, 1);
        let gap = ev[2].0 as i64 - ev[1].0 as i64;
        assert!((gap - 5512).abs() <= 1, "gap {}", gap);
        assert_eq!(ev.len(), 16); // exactly two seconds = one bar at 120 (16 steps = 1 bar of 4 beats)
    }

    #[test]
    fn wraps_with_no_drift() {
        let mut t = Transport::new(44100.0, 16);
        t.playing = true;
        let ev = run(&mut t, 44100 * 2 * 10 + 10);
        let starts: Vec<u64> = ev.iter().filter(|e| e.1 == 0).map(|e| e.0).collect();
        assert_eq!(starts.len(), 11);
        for (i, s) in starts.iter().enumerate() {
            let exp = (i as u64) * 88200;
            assert!((*s as i64 - exp as i64).abs() <= 1, "bar {} at {} expected {}", i, s, exp);
        }
    }

    #[test]
    fn swing_delays_odd_steps() {
        let mut t = Transport::new(44100.0, 16);
        t.swing_on = true;
        t.swing = 0.2;
        t.playing = true;
        let ev = run(&mut t, 20000);
        let sl = 5512.5;
        assert!((ev[1].0 as f64 - (sl * 1.2)).abs() < 2.0);
        assert!((ev[2].0 as f64 - (sl * 2.0)).abs() < 2.0);
    }

    #[test]
    fn stopped_never_fires() {
        let mut t = Transport::new(44100.0, 16);
        assert!(run(&mut t, 50000).is_empty());
        assert_eq!(t.until_due(), u64::MAX);
    }

    #[test]
    fn bpm_change_keeps_phase() {
        let mut t = Transport::new(44100.0, 16);
        t.playing = true;
        let _ = run(&mut t, 8000); // into step 1
        t.set_bpm(240.0);
        let ev = run(&mut t, 20000);
        // next step arrives soon, at the faster tempo spacing
        let gap = ev[1].0 as i64 - ev[0].0 as i64;
        assert!((gap - 2756).abs() <= 2, "gap {}", gap);
    }
}
