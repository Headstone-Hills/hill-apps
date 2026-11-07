//! Sample-position transport. Pure arithmetic; the audio callback owns one and calls [`Transport::advance`].

#[derive(Debug, Clone)]
pub struct Transport {
    pub rate: u32,
    pub bpm: f64,
    pub beats_per_bar: u32,
    pub pos: u64,
    pub playing: bool,
    /// `[start, end)` in frames. While playing, reaching `end` wraps to `start` sample-exactly.
    pub loop_region: Option<(u64, u64)>,
}

impl Transport {
    pub fn new(rate: u32) -> Self {
        Transport { rate, bpm: 120.0, beats_per_bar: 4, pos: 0, playing: false, loop_region: None }
    }

    pub fn frames_per_beat(&self) -> f64 {
        self.rate as f64 * 60.0 / self.bpm
    }

    /// Frame of a (fractional) beat position from the start of the song.
    pub fn beat_to_frame(&self, beat: f64) -> u64 {
        (beat * self.frames_per_beat()).round().max(0.0) as u64
    }

    /// 1-based (bar, beat) at the current position.
    pub fn bar_beat(&self) -> (u32, u32) {
        let beat = (self.pos as f64 / self.frames_per_beat()).floor() as u64;
        let bpb = self.beats_per_bar.max(1) as u64;
        ((beat / bpb) as u32 + 1, (beat % bpb) as u32 + 1)
    }

    /// Snap a frame position to the nearest 1/`div` of a beat (div=1 beat, 2 eighths, 4 sixteenths ...).
    pub fn snap(&self, frame: u64, div: u32) -> u64 {
        let step = self.frames_per_beat() / div.max(1) as f64;
        ((frame as f64 / step).round() * step).round() as u64
    }

    /// Advance by `frames` and describe what to render as `(start_position, length)` spans.
    /// Several spans only occur when a loop boundary falls inside the chunk. Nothing while stopped.
    pub fn advance(&mut self, frames: usize, spans: &mut Vec<(u64, usize)>) {
        spans.clear();
        if !self.playing {
            return;
        }
        let mut left = frames;
        while left > 0 {
            let take = match self.loop_region {
                Some((s, e)) if e > s && self.pos < e => left.min((e - self.pos) as usize),
                _ => left,
            };
            spans.push((self.pos, take));
            self.pos += take as u64;
            left -= take;
            if let Some((s, e)) = self.loop_region {
                if e > s && self.pos == e {
                    self.pos = s;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &mut Transport, chunk: usize, total: usize) -> Vec<u64> {
        // the absolute position of every rendered frame, in order
        let mut out = Vec::new();
        let mut spans = Vec::new();
        let mut done = 0;
        while done < total {
            let n = chunk.min(total - done);
            t.advance(n, &mut spans);
            for &(s, l) in &spans {
                out.extend(s..s + l as u64);
            }
            done += n;
        }
        out
    }

    #[test]
    fn loop_wraps_sample_exactly_for_any_chunk_size() {
        let make = || {
            let mut t = Transport::new(44100);
            t.playing = true;
            t.loop_region = Some((1000, 1700));
            t.pos = 900;
            t
        };
        let a = run(&mut make(), 64, 3000);
        let b = run(&mut make(), 1000, 3000);
        let c = run(&mut make(), 7, 3000);
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(&a[..3], &[900, 901, 902]);
        assert_eq!(a[99], 999);
        assert_eq!(a[100], 1000);
        assert_eq!(a[100 + 700], 1000); // wrapped back to the loop start
        assert!(a.iter().skip(100).all(|&p| (1000..1700).contains(&p)));
    }

    #[test]
    fn stopped_renders_nothing_and_does_not_move() {
        let mut t = Transport::new(44100);
        let mut s = vec![(1u64, 1usize)];
        t.advance(512, &mut s);
        assert!(s.is_empty());
        assert_eq!(t.pos, 0);
    }

    #[test]
    fn position_past_the_loop_end_plays_on() {
        let mut t = Transport::new(44100);
        t.playing = true;
        t.loop_region = Some((0, 100));
        t.pos = 500;
        let mut s = Vec::new();
        t.advance(50, &mut s);
        assert_eq!(s, vec![(500, 50)]);
    }

    #[test]
    fn bars_beats_and_snap() {
        let mut t = Transport::new(44100);
        t.bpm = 120.0;
        assert_eq!(t.frames_per_beat(), 22050.0);
        t.pos = 22050 * 5 + 10;
        assert_eq!(t.bar_beat(), (2, 2));
        assert_eq!(t.beat_to_frame(2.5), 55125);
        assert_eq!(t.snap(22050 + 400, 1), 22050);
        assert_eq!(t.snap(22050 + 400, 4), 22050); // 400 < half a 16th (2756): snaps back
        assert_eq!(t.snap(22050 + 3000, 4), 22050 + 5513); // past half a 16th: snaps forward (5512.5 rounds to 5513)
    }
}
