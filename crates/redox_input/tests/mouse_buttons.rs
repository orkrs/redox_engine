//! Specification (tests first) for the mouse Back/Forward bug.
//!
//! Today `MouseButton::from(winit Back)` and `from(winit Forward)` both become `Other(0)`, so a
//! binding cannot tell them apart. Fix: add the variants `MouseButton::Back` and
//! `MouseButton::Forward` to the enum in `crates/redox_input/src/mouse.rs` and map the winit
//! buttons to them. Keep `Other(u16)` for the rest.
//!
//! Also fix the unit tests in `mouse.rs`: delete `unrecognised_winit_buttons_fall_back_to_other_zero`
//! (it characterised the bug) and remove `#[ignore]` from `back_and_forward_buttons_are_distinct`.
//! Check every exhaustive `match` on `MouseButton` in the workspace
//! (`grep -rn "MouseButton" crates`: input rebinding, editor, courier example) still compiles and
//! add the new variants where the code lists all buttons.

#![allow(clippy::unwrap_used)]

use redox_input::mouse::{MouseButton, MouseState};

#[test]
fn back_and_forward_are_their_own_buttons() {
    let back = MouseButton::from(winit::event::MouseButton::Back);
    let forward = MouseButton::from(winit::event::MouseButton::Forward);
    assert_eq!(back, MouseButton::Back);
    assert_eq!(forward, MouseButton::Forward);
    assert_ne!(back, forward);
}

#[test]
fn the_other_buttons_still_convert() {
    use winit::event::MouseButton as Winit;
    assert_eq!(MouseButton::from(Winit::Left), MouseButton::Left);
    assert_eq!(MouseButton::from(Winit::Right), MouseButton::Right);
    assert_eq!(MouseButton::from(Winit::Middle), MouseButton::Middle);
    assert_eq!(MouseButton::from(Winit::Other(7)), MouseButton::Other(7));
}

#[test]
fn the_buttons_are_tracked_independently() {
    let mut mouse = MouseState::new();
    mouse.press(MouseButton::Back);
    assert!(mouse.is_pressed(MouseButton::Back));
    assert!(!mouse.is_pressed(MouseButton::Forward));
    assert!(!mouse.is_pressed(MouseButton::Other(0)));
}

#[test]
fn the_new_variants_survive_a_ron_round_trip() {
    for button in [MouseButton::Back, MouseButton::Forward] {
        let text = ron::to_string(&button).unwrap();
        assert_eq!(ron::from_str::<MouseButton>(&text).unwrap(), button);
    }
}
