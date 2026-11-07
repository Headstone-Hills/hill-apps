//! FM-1 effect controls: which MIDI CC does what, loaded from `data/fm1_fx_cc.json` (data, not code, so a firmware that moves a
//! number only needs the file changed). See `docs/FM1_MIDI.md` for how each entry was checked on a real unit.
//!
//! On firmware V16 the FX CCs are 70-93 on the FX channel (default channel 2, separate from the note channel). Values behave as the
//! parameter's own units: a CC value above a parameter's maximum acts as the maximum (filter type 64 and 127 sounded identical),
//! so [`FxParam::clamp`] keeps a sent value inside the range instead of assuming 0-127 is scaled across it.

use serde::Deserialize;
use std::sync::OnceLock;

pub const FX_JSON: &str = include_str!("../data/fm1_fx_cc.json");

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct FxParam {
    pub cc: u8,
    pub effect: String,
    pub param: String,
    pub min: u8,
    pub max: u8,
    /// `behaviour`, `group` or `order`; see the data file's `_about`
    pub evidence: String,
}

#[derive(Deserialize)]
struct File {
    fx_channel_default: u8,
    params: Vec<FxParam>,
}

pub struct FxMap {
    /// 1-based MIDI channel
    pub fx_channel_default: u8,
    pub params: Vec<FxParam>,
}

pub fn fx_map() -> &'static FxMap {
    static MAP: OnceLock<FxMap> = OnceLock::new();
    MAP.get_or_init(|| {
        let f: File = serde_json::from_str(FX_JSON).expect("fm1_fx_cc.json is valid");
        FxMap { fx_channel_default: f.fx_channel_default, params: f.params }
    })
}

impl FxMap {
    pub fn find(&self, effect: &str, param: &str) -> Option<&FxParam> {
        self.params.iter().find(|p| p.effect.eq_ignore_ascii_case(effect) && p.param.eq_ignore_ascii_case(param))
    }
    pub fn by_cc(&self, cc: u8) -> Option<&FxParam> {
        self.params.iter().find(|p| p.cc == cc)
    }
    /// The effects in order, each with its parameters (the switch first).
    pub fn effects(&self) -> Vec<(&str, Vec<&FxParam>)> {
        let mut out: Vec<(&str, Vec<&FxParam>)> = Vec::new();
        for p in &self.params {
            match out.last_mut() {
                Some((e, v)) if *e == p.effect => v.push(p),
                _ => out.push((&p.effect, vec![p])),
            }
        }
        out
    }
}

impl FxParam {
    pub fn is_switch(&self) -> bool {
        self.param == "Switch"
    }
    /// Keep a value inside this parameter's own range.
    pub fn clamp(&self, v: u8) -> u8 {
        v.clamp(self.min, self.max)
    }
    /// 0.0..=1.0 across this parameter's range (what a knob or a slider in the UI holds).
    pub fn from_fraction(&self, f: f32) -> u8 {
        let f = f.clamp(0.0, 1.0);
        self.min + ((self.max - self.min) as f32 * f).round() as u8
    }
    pub fn to_fraction(&self, v: u8) -> f32 {
        if self.max == self.min { 0.0 } else { (self.clamp(v) - self.min) as f32 / (self.max - self.min) as f32 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_covers_cc_70_to_93_once_in_six_groups_of_four_switch_first() {
        let m = fx_map();
        assert_eq!(m.fx_channel_default, 2);
        assert_eq!(m.params.iter().map(|p| p.cc).collect::<Vec<_>>(), (70..=93).collect::<Vec<u8>>());
        let fx = m.effects();
        assert_eq!(fx.iter().map(|(e, _)| *e).collect::<Vec<_>>(), ["Filter", "Reverb", "Delay", "Distortion", "Chorus", "Phaser"]);
        for (e, ps) in &fx {
            assert_eq!(ps.len(), 4, "{e}");
            assert!(ps[0].is_switch() && (ps[0].min, ps[0].max) == (0, 1), "{e} starts with its switch");
            assert!(ps[1..].iter().all(|p| !p.is_switch()));
        }
        assert!(m.params.iter().all(|p| p.min < p.max && ["behaviour", "group", "order"].contains(&p.evidence.as_str())));
    }

    #[test]
    fn lookups_match_what_was_measured_on_the_unit() {
        let m = fx_map();
        assert_eq!(m.find("Filter", "Switch").unwrap().cc, 70);
        assert_eq!(m.find("filter", "cutoff").unwrap().cc, 72);
        assert_eq!(m.find("Distortion", "Level").unwrap().cc, 85);
        assert_eq!(m.by_cc(93).unwrap().effect, "Phaser");
        assert!(m.find("Flanger", "Mix").is_none() && m.by_cc(0).is_none());
        // the four CCs whose effect could not be measured are marked as order-only, so a UI can say so
        let order: Vec<u8> = m.params.iter().filter(|p| p.evidence == "order").map(|p| p.cc).collect();
        assert_eq!(order, vec![76, 79, 91, 92]);
    }

    #[test]
    fn values_stay_inside_each_parameters_own_range() {
        let m = fx_map();
        let ty = m.find("Filter", "Type").unwrap();
        assert_eq!((ty.clamp(64), ty.clamp(127), ty.clamp(1)), (2, 2, 1), "type 64 and 127 are the same");
        let q = m.find("Filter", "Q").unwrap();
        assert_eq!(q.clamp(127), 10);
        let cutoff = m.find("Filter", "Cutoff").unwrap();
        assert_eq!((cutoff.from_fraction(0.0), cutoff.from_fraction(1.0), cutoff.from_fraction(0.5)), (0, 107, 54));
        assert!((cutoff.to_fraction(107) - 1.0).abs() < 1e-6 && cutoff.to_fraction(200) == 1.0);
    }
}
