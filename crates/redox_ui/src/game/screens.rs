//! Screen stack: main menu, game HUD, pause, dialogs. Only the top screen takes input; screens
//! under a translucent top screen stay visible.

/// One entry of the stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    /// Name used by the game to build the screen's widget tree.
    pub name: String,
    /// An opaque screen hides everything below it (a menu); a translucent one does not (a pause
    /// dialog over the game).
    pub opaque: bool,
}

/// The stack of screens.
#[derive(Clone, Debug, Default)]
pub struct ScreenStack {
    screens: Vec<Screen>,
}

impl ScreenStack {
    /// Puts a screen on top.
    pub fn push(&mut self, name: &str, opaque: bool) {
        self.screens.push(Screen {
            name: name.into(),
            opaque,
        });
    }

    /// Removes and returns the top screen.
    pub fn pop(&mut self) -> Option<Screen> {
        self.screens.pop()
    }

    /// Replaces the top screen (or pushes when empty).
    pub fn replace(&mut self, name: &str, opaque: bool) {
        self.screens.pop();
        self.push(name, opaque);
    }

    /// Removes everything and shows one screen.
    pub fn reset(&mut self, name: &str, opaque: bool) {
        self.screens.clear();
        self.push(name, opaque);
    }

    /// The screen that receives input.
    pub fn top(&self) -> Option<&Screen> {
        self.screens.last()
    }

    /// Number of screens.
    pub fn len(&self) -> usize {
        self.screens.len()
    }

    /// Whether the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.screens.is_empty()
    }

    /// Screens to draw, bottom to top: from the highest opaque screen up.
    pub fn visible(&self) -> Vec<&Screen> {
        let start = self.screens.iter().rposition(|s| s.opaque).unwrap_or(0);
        self.screens[start..].iter().collect()
    }

    /// Whether a screen with this name is on the stack.
    pub fn contains(&self, name: &str) -> bool {
        self.screens.iter().any(|s| s.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(stack: &ScreenStack) -> Vec<&str> {
        stack.visible().iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn an_opaque_screen_hides_the_ones_below() {
        let mut stack = ScreenStack::default();
        stack.push("hud", true);
        stack.push("pause", false);
        assert_eq!(names(&stack), vec!["hud", "pause"], "pause is translucent");
        stack.push("settings", true);
        assert_eq!(names(&stack), vec!["settings"]);
        assert_eq!(stack.top().unwrap().name, "settings");
        stack.pop();
        assert_eq!(names(&stack), vec!["hud", "pause"]);
    }

    #[test]
    fn replace_and_reset() {
        let mut stack = ScreenStack::default();
        stack.replace("menu", true);
        stack.push("settings", true);
        stack.replace("credits", true);
        assert_eq!(stack.len(), 2);
        assert!(stack.contains("menu") && !stack.contains("settings"));
        stack.reset("hud", true);
        assert_eq!(stack.len(), 1);
        assert!(stack.pop().is_some() && stack.pop().is_none() && stack.is_empty());
        assert!(stack.visible().is_empty());
    }
}
