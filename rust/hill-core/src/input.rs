/// Logical buttons. The 13-input set (everything except L3/R3) is what every
/// app is designed around; L3/R3 are accepted but unused.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Btn {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    X,
    Y,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    Select,
    Start,
    Fn,
}

impl Btn {
    pub const ALL: [Btn; 17] = [
        Btn::Up,
        Btn::Down,
        Btn::Left,
        Btn::Right,
        Btn::A,
        Btn::B,
        Btn::X,
        Btn::Y,
        Btn::L1,
        Btn::R1,
        Btn::L2,
        Btn::R2,
        Btn::L3,
        Btn::R3,
        Btn::Select,
        Btn::Start,
        Btn::Fn,
    ];

    fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

/// Set of currently held buttons.
#[derive(Copy, Clone, Default)]
pub struct Held(u32);

impl Held {
    pub fn set(&mut self, b: Btn, down: bool) {
        if down {
            self.0 |= b.bit();
        } else {
            self.0 &= !b.bit();
        }
    }
    pub fn is(&self, b: Btn) -> bool {
        self.0 & b.bit() != 0
    }
    pub fn any_dpad(&self) -> bool {
        self.is(Btn::Up) || self.is(Btn::Down) || self.is(Btn::Left) || self.is(Btn::Right)
    }
}

/// Auto-repeat for a held button (BPM, swing).
#[derive(Default)]
pub struct Repeat {
    btn: Option<Btn>,
    held_ms: u32,
    next_ms: u32,
}

impl Repeat {
    const DELAY: u32 = 400;
    const EVERY: u32 = 70;

    pub fn start(&mut self, b: Btn) {
        self.btn = Some(b);
        self.held_ms = 0;
        self.next_ms = Self::DELAY;
    }
    pub fn stop(&mut self, b: Btn) {
        if self.btn == Some(b) {
            self.btn = None;
        }
    }
    pub fn clear(&mut self) {
        self.btn = None;
    }
    /// Returns the button to re-fire this tick, if any.
    pub fn tick(&mut self, dt: u32) -> Option<Btn> {
        let b = self.btn?;
        self.held_ms += dt;
        if self.held_ms >= self.next_ms {
            self.next_ms += Self::EVERY;
            Some(b)
        } else {
            None
        }
    }
}

/// Tap-tempo estimator driven by the UI millisecond clock.
#[derive(Default)]
pub struct TapTempo {
    taps: Vec<u64>,
}

impl TapTempo {
    pub fn tap(&mut self, now_ms: u64) -> Option<f32> {
        if let Some(&last) = self.taps.last() {
            if now_ms.saturating_sub(last) > 2000 {
                self.taps.clear();
            }
        }
        self.taps.push(now_ms);
        if self.taps.len() > 5 {
            self.taps.remove(0);
        }
        if self.taps.len() < 2 {
            return None;
        }
        let span = (self.taps[self.taps.len() - 1] - self.taps[0]) as f32;
        let avg = span / (self.taps.len() - 1) as f32;
        if avg <= 0.0 {
            return None;
        }
        Some((60000.0 / avg).clamp(40.0, 240.0))
    }
}
