//! The Arrange screen's drum browser: style, pattern and kit of the selected section.

use hill_core::{App, Btn, MemHost};
use hilldaw_app::status::Status;
use hilldaw_app::{HillDaw, View};
use std::sync::{Arc, Mutex};

fn press(a: &mut HillDaw, b: Btn) {
    a.event(b, true);
    a.event(b, false);
}

fn app() -> HillDaw {
    HillDaw::new(Box::new(MemHost::default()), Arc::new(Mutex::new(Status::default())))
}

fn to_arrange(a: &mut HillDaw) {
    for _ in 0..8 {
        if a.view() == View::Arrange {
            return;
        }
        press(a, Btn::R1);
    }
    panic!("never reached Arrange");
}

fn first_section(a: &HillDaw) -> hilldaw_core::project::DrumSectionModel {
    a.project.drums.sections[0].clone()
}

#[test]
fn the_browser_changes_the_kit_pattern_and_style_of_the_section_live() {
    let mut a = app();
    to_arrange(&mut a);
    let start = first_section(&a);
    assert_eq!(start.kit, "", "automatic kit to begin with");

    press(&mut a, Btn::B); // open on the pattern column
    // kit column, down once = the first kit in the list; with no sample kits that is "8-Bit NES"
    press(&mut a, Btn::Right);
    press(&mut a, Btn::Down);
    assert_eq!(first_section(&a).kit, "8-Bit NES");
    press(&mut a, Btn::Down);
    assert_eq!(first_section(&a).kit, "8-Bit GB");
    press(&mut a, Btn::Down); // end of the list stays put
    assert_eq!(first_section(&a).kit, "8-Bit GB");
    press(&mut a, Btn::Up);
    press(&mut a, Btn::Up);
    assert_eq!(first_section(&a).kit, "", "back to automatic");

    // pattern column: down = the next pattern of the same style
    press(&mut a, Btn::Left);
    press(&mut a, Btn::Down);
    let next = first_section(&a);
    assert_ne!(next.preset, start.preset);
    let pres = hill_core::presets::builtin();
    let style_of = |id: &str| pres.patterns.iter().find(|p| p.id == id).unwrap().style.clone();
    assert_eq!(style_of(&next.preset), style_of(&start.preset), "stays inside the style");

    // style column: down = the first pattern of the next style
    press(&mut a, Btn::Left);
    press(&mut a, Btn::Down);
    assert_ne!(style_of(&first_section(&a).preset), style_of(&start.preset));

    // A closes the browser and the next A adds a section as before
    press(&mut a, Btn::A);
    let n = a.project.drums.sections.len();
    press(&mut a, Btn::A);
    assert_eq!(a.project.drums.sections.len(), n + 1);
}

#[test]
fn a_kit_can_be_given_to_every_section_and_undo_takes_it_back() {
    let mut a = app();
    to_arrange(&mut a);
    press(&mut a, Btn::A); // a second section
    press(&mut a, Btn::B);
    press(&mut a, Btn::Right);
    press(&mut a, Btn::Right);
    press(&mut a, Btn::Down); // 8-Bit NES on the selected section only
    assert_eq!(a.project.drums.sections[1].kit, "8-Bit NES");
    assert_eq!(a.project.drums.sections[0].kit, "");
    press(&mut a, Btn::X);
    assert!(a.project.drums.sections.iter().all(|s| s.kit == "8-Bit NES"));
    a.event(Btn::Y, true);
    press(&mut a, Btn::B); // Y+B: undo
    a.event(Btn::Y, false);
    assert_eq!(a.project.drums.sections[0].kit, "", "undo");
}

#[test]
fn the_kit_survives_saving_and_the_8_bit_kit_makes_different_sound() {
    let mut a = app();
    to_arrange(&mut a);
    let render = |a: &mut HillDaw| {
        press(a, Btn::Start);
        let mut out = vec![0.0f32; 44_100 * 2];
        for c in out.chunks_mut(1024) {
            a.audio(c);
        }
        press(a, Btn::Start);
        out
    };
    let before = render(&mut a);
    press(&mut a, Btn::B);
    press(&mut a, Btn::Right);
    press(&mut a, Btn::Down);
    press(&mut a, Btn::A);
    let after = render(&mut a);
    assert!(after.iter().any(|x| x.abs() > 0.01), "the 8-bit kit is audible");
    assert_ne!(before, after, "and it is a different sound");
    let json = a.project.to_json();
    assert!(json.contains("8-Bit NES"));
    assert_eq!(hilldaw_core::project::Project::from_json(&json).unwrap().drums.sections[0].kit, "8-Bit NES");
}
