//! Text entry: typed characters and IME composition.
//!
//! Gameplay input ignores text. A UI field that has focus calls [`TextInput::set_wants_text`]
//! (the app then enables the platform IME for the window); while it is set, typed characters
//! and IME results accumulate in [`TextInput::typed`] until the next frame.

use winit::event::{Ime, WindowEvent};

/// An in-progress IME composition (not committed yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preedit {
    /// The composition string to display.
    pub text: String,
    /// Selected byte range inside `text` (the IME's cursor), if any.
    pub cursor: Option<(usize, usize)>,
}

/// Text entry state (part of [`crate::InputState`]).
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    typed: String,
    preedit: Option<Preedit>,
    wants_text: bool,
}

impl TextInput {
    /// Creates an idle state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a text field has focus. Characters are dropped while this is `false`.
    pub fn wants_text(&self) -> bool {
        self.wants_text
    }

    /// Declares that a text field gained (`true`) or lost (`false`) focus.
    pub fn set_wants_text(&mut self, wants: bool) {
        self.wants_text = wants;
        if !wants {
            self.typed.clear();
            self.preedit = None;
        }
    }

    /// Characters entered since the last frame (committed text, no control characters).
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// Takes the typed text, leaving the buffer empty.
    pub fn take_typed(&mut self) -> String {
        std::mem::take(&mut self.typed)
    }

    /// The composition being edited by the IME, if any.
    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }

    /// Call at the start of each frame.
    pub fn begin_frame(&mut self) {
        self.typed.clear();
    }

    /// Appends text from a key press; control characters are dropped.
    pub fn type_text(&mut self, text: &str) {
        if self.wants_text {
            self.typed.extend(text.chars().filter(|c| !c.is_control()));
        }
    }

    /// Replaces the composition (an empty string clears it).
    pub fn set_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) {
        if !self.wants_text {
            return;
        }
        self.preedit = (!text.is_empty()).then(|| Preedit {
            text: text.to_owned(),
            cursor,
        });
    }

    /// Commits IME text: the composition ends and the text is typed.
    pub fn commit(&mut self, text: &str) {
        self.preedit = None;
        self.type_text(text);
    }

    /// Feeds a window event (`KeyboardInput` text and `Ime` events).
    pub fn process_window_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
                if let Some(text) = &event.text {
                    self.type_text(text);
                }
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => self.set_preedit(text, *cursor),
            WindowEvent::Ime(Ime::Commit(text)) => self.commit(text),
            WindowEvent::Ime(Ime::Disabled) => self.preedit = None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_ignored_without_a_focused_field() {
        let mut t = TextInput::new();
        t.type_text("hello");
        t.commit("ignored");
        assert_eq!(t.typed(), "");
    }

    #[test]
    fn typing_filters_control_characters_and_clears_each_frame() {
        let mut t = TextInput::new();
        t.set_wants_text(true);
        t.type_text("a\u{8}b\r\n\u{7f}c");
        assert_eq!(t.typed(), "abc");
        t.begin_frame();
        assert_eq!(t.typed(), "");
        t.type_text("é世");
        assert_eq!(t.take_typed(), "é世");
        assert_eq!(t.typed(), "");
    }

    #[test]
    fn ime_composition_lifecycle() {
        let mut t = TextInput::new();
        t.set_wants_text(true);
        t.process_window_event(&WindowEvent::Ime(Ime::Preedit("にほ".into(), Some((0, 3)))));
        assert_eq!(
            t.preedit(),
            Some(&Preedit {
                text: "にほ".into(),
                cursor: Some((0, 3))
            })
        );
        t.process_window_event(&WindowEvent::Ime(Ime::Commit("日本".into())));
        assert!(t.preedit().is_none());
        assert_eq!(t.typed(), "日本");

        t.set_preedit("x", None);
        t.process_window_event(&WindowEvent::Ime(Ime::Disabled));
        assert!(t.preedit().is_none());
        t.set_preedit("y", None);
        t.set_preedit("", None);
        assert!(t.preedit().is_none(), "empty preedit clears");
    }

    #[test]
    fn losing_focus_discards_pending_text_and_composition() {
        let mut t = TextInput::new();
        t.set_wants_text(true);
        t.type_text("abc");
        t.set_preedit("x", None);
        t.set_wants_text(false);
        assert_eq!(t.typed(), "");
        assert!(t.preedit().is_none());
    }
}
