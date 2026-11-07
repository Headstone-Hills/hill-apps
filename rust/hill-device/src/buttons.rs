//! Physical button -> logical button mapping, configurable with a small
//! `buttons.cfg` file written by the `hillmap` wizard.

use hill_core::Btn;
use sdl2::keyboard::Keycode;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct Map {
    /// joystick button index -> logical
    pub joy: HashMap<u8, Btn>,
    /// hat bits (1 up, 2 right, 4 down, 8 left) are always the D-pad;
    /// these are extra D-pad-as-buttons indices (some handhelds report so)
    pub stick: bool,
}

pub const NAMES: [(&str, Btn); 17] = [
    ("A", Btn::A),
    ("B", Btn::B),
    ("X", Btn::X),
    ("Y", Btn::Y),
    ("L1", Btn::L1),
    ("R1", Btn::R1),
    ("L2", Btn::L2),
    ("R2", Btn::R2),
    ("L3", Btn::L3),
    ("R3", Btn::R3),
    ("SELECT", Btn::Select),
    ("START", Btn::Start),
    ("FN", Btn::Fn),
    ("UP", Btn::Up),
    ("DOWN", Btn::Down),
    ("LEFT", Btn::Left),
    ("RIGHT", Btn::Right),
];

impl Map {
    /// Indices used by the Python apps on the RG35XXSP (muOS).
    pub fn defaults() -> Self {
        let mut joy = HashMap::new();
        for (i, b) in [
            (3, Btn::A),
            (4, Btn::B),
            (5, Btn::Y),
            (6, Btn::X),
            (7, Btn::L1),
            (8, Btn::R1),
            (9, Btn::Select),
            (10, Btn::Start),
            (11, Btn::Fn),
            (12, Btn::L2),
            (13, Btn::R2),
        ] {
            joy.insert(i, b);
        }
        Map { joy, stick: false }
    }

    pub fn parse(text: &str) -> Self {
        let mut joy = HashMap::new();
        let mut stick = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim().to_uppercase(), v.trim());
            if k == "STICK" {
                stick = v == "1";
            } else if let (Some((_, b)), Ok(i)) = (NAMES.iter().find(|(n, _)| *n == k), v.parse::<u8>()) {
                joy.insert(i, *b);
            }
        }
        Map { joy, stick }
    }

    pub fn load(dir: &Path) -> Self {
        let path = std::env::var("HILL_BUTTONS").map(std::path::PathBuf::from).unwrap_or_else(|_| dir.join("buttons.cfg"));
        match fs::read_to_string(path) {
            Ok(t) => {
                let m = Self::parse(&t);
                if m.joy.is_empty() {
                    Self::defaults()
                } else {
                    m
                }
            }
            Err(_) => Self::defaults(),
        }
    }

    pub fn render(map: &[(Btn, u8)], stick: bool) -> String {
        let mut s = String::from("# written by hillmap\n");
        for (b, i) in map {
            let name = NAMES.iter().find(|(_, x)| x == b).map(|(n, _)| *n).unwrap_or("?");
            s.push_str(&format!("{}={}\n", name, i));
        }
        if stick {
            s.push_str("STICK=1\n");
        }
        s
    }
}

/// Hat bitmask -> D-pad buttons.
pub fn hat_buttons(state: u8) -> [bool; 4] {
    [state & 1 != 0, state & 4 != 0, state & 8 != 0, state & 2 != 0] // up, down, left, right
}

/// Desktop development keys (same as the Python apps).
pub fn key(k: Keycode) -> Option<Btn> {
    Some(match k {
        Keycode::Up => Btn::Up,
        Keycode::Down => Btn::Down,
        Keycode::Left => Btn::Left,
        Keycode::Right => Btn::Right,
        Keycode::A => Btn::A,
        Keycode::B => Btn::B,
        Keycode::X => Btn::X,
        Keycode::Y => Btn::Y,
        Keycode::Q => Btn::L1,
        Keycode::W => Btn::R1,
        Keycode::E => Btn::L2,
        Keycode::R => Btn::R2,
        Keycode::O => Btn::Select,
        Keycode::P => Btn::Start,
        Keycode::U => Btn::Fn,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_defaults() {
        let m = Map::parse("# c\nA=0\nB=1\nSTART=9\nSTICK=1\nbogus\nL1=x\n");
        assert_eq!(m.joy.get(&0), Some(&Btn::A));
        assert_eq!(m.joy.get(&9), Some(&Btn::Start));
        assert!(m.stick);
        assert_eq!(m.joy.len(), 3);
        let d = Map::defaults();
        assert_eq!(d.joy.get(&10), Some(&Btn::Start));
        assert_eq!(d.joy.len(), 11);
    }

    #[test]
    fn render_roundtrip() {
        let text = Map::render(&[(Btn::A, 4), (Btn::Fn, 12)], true);
        let m = Map::parse(&text);
        assert_eq!(m.joy.get(&4), Some(&Btn::A));
        assert_eq!(m.joy.get(&12), Some(&Btn::Fn));
        assert!(m.stick);
    }

    #[test]
    fn hat() {
        assert_eq!(hat_buttons(1 | 2), [true, false, false, true]);
        assert_eq!(hat_buttons(0), [false; 4]);
    }
}
