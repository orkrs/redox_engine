//! `InputState` with contexts, gamepads and text entry together.

#![allow(clippy::unwrap_used)]

use redox_input::{
    ActionBinding, ActionKind, ActionMap, GamepadAxis, GamepadButton, GamepadId, InputState,
    Rebinder,
};
use winit::keyboard::KeyCode;

fn digital(state: &InputState, name: &str) -> bool {
    matches!(state.action(name), ActionKind::Digital(true))
}

#[test]
fn contexts_override_the_base_map_and_pads_feed_actions() {
    let mut input = InputState::new();
    input
        .actions
        .add("Jump", ActionBinding::Key(KeyCode::Space));
    input
        .actions
        .add("Jump", ActionBinding::PadButton(GamepadButton::South));

    let mut menu = ActionMap::new();
    menu.add("Select", ActionBinding::PadButton(GamepadButton::South));
    input.contexts.define("menu", menu, true);

    input.gamepads.press(GamepadId(0), GamepadButton::South);
    assert!(digital(&input, "Jump"));

    input.contexts.push("menu");
    assert!(digital(&input, "Select"));
    assert!(!digital(&input, "Jump"), "the blocking menu hides gameplay");

    input.contexts.pop();
    assert!(digital(&input, "Jump"));
}

#[test]
fn begin_frame_clears_pad_edges_and_typed_text() {
    let mut input = InputState::new();
    input.text.set_wants_text(true);
    input.text.type_text("hi");
    input.gamepads.press(GamepadId(0), GamepadButton::Start);
    input
        .gamepads
        .set_axis(GamepadId(0), GamepadAxis::LeftStickX, 1.0);
    assert!(input.gamepads.any_just_pressed(GamepadButton::Start));

    input.begin_frame();
    assert_eq!(input.text.typed(), "");
    assert!(!input.gamepads.any_just_pressed(GamepadButton::Start));
    assert!(
        input.gamepads.any_pressed(GamepadButton::Start),
        "still held"
    );
    assert!(input.gamepads.any_axis(GamepadAxis::LeftStickX) > 0.9);
}

#[test]
fn pad_bindings_survive_ron_and_can_be_rebound() {
    let mut map = ActionMap::new();
    map.add(
        "Fire",
        ActionBinding::PadButton(GamepadButton::RightTrigger),
    );
    map.add("Look", ActionBinding::PadAxis(GamepadAxis::RightStickX));
    let restored = ActionMap::from_ron(&map.to_ron().unwrap()).unwrap();
    assert_eq!(
        restored.bindings_for("Fire"),
        [ActionBinding::PadButton(GamepadButton::RightTrigger)]
    );
    assert_eq!(
        restored.bindings_for("Look"),
        [ActionBinding::PadAxis(GamepadAxis::RightStickX)]
    );

    let mut map = restored;
    map.add("Reload", ActionBinding::PadButton(GamepadButton::West));
    let mut rebinder = Rebinder::default();
    rebinder.start("Fire", 0);
    let outcome = rebinder
        .feed_pad_button(&mut map, GamepadButton::West)
        .unwrap();
    assert_eq!(outcome.conflicts, ["Reload"]);
    assert_eq!(
        map.bindings_for("Fire"),
        [ActionBinding::PadButton(GamepadButton::West)]
    );
}
