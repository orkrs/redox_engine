//! Pure rules of the file dialogs: when the unsaved-changes prompt is needed, what Save does,
//! which File menu items are enabled and how the window title looks. No window and no files
//! are involved, so every rule is tested here without a UI.

/// An action that asks about unsaved changes before it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingAction {
    /// Start a new empty level.
    NewLevel,
    /// Open another level.
    OpenLevel,
    /// Close the current level.
    CloseLevel,
    /// Open another project.
    OpenProject,
    /// Create a new project.
    NewProject,
    /// Close the editor window.
    Quit,
}

/// The button the user pressed in the unsaved-changes prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptAnswer {
    /// Save, then continue.
    Save,
    /// Continue without saving.
    DontSave,
    /// Keep the level and the pending action is dropped.
    Cancel,
}

/// What the host does after a prompt answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptOutcome {
    /// Save the level first; continue only if the save succeeded.
    SaveThenContinue,
    /// Continue without saving.
    Continue,
    /// Do nothing.
    Abort,
}

/// Where Save writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    /// The level has a path; write to it.
    Direct,
    /// The level has no path yet; ask for one (Save As).
    AskPath,
}

/// Which File menu items are enabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileAvailability {
    /// Save is enabled.
    pub save: bool,
    /// Save As is enabled.
    pub save_as: bool,
    /// Close Level is enabled.
    pub close: bool,
}

/// Whether the prompt must be shown before `action` runs.
pub fn needs_save_prompt(is_dirty: bool) -> bool {
    is_dirty
}

/// Maps the prompt answer to what the host does next.
pub fn resolve_prompt(answer: PromptAnswer) -> PromptOutcome {
    match answer {
        PromptAnswer::Save => PromptOutcome::SaveThenContinue,
        PromptAnswer::DontSave => PromptOutcome::Continue,
        PromptAnswer::Cancel => PromptOutcome::Abort,
    }
}

/// Save writes directly when the level has a path, otherwise it asks for one.
pub fn save_target(has_path: bool) -> SaveTarget {
    if has_path {
        SaveTarget::Direct
    } else {
        SaveTarget::AskPath
    }
}

/// The level items need an open level.
pub fn availability(level_open: bool) -> FileAvailability {
    FileAvailability {
        save: level_open,
        save_as: level_open,
        close: level_open,
    }
}

/// Window title: the level name, with ` *` while there are unsaved changes.
pub fn window_title(level_title: &str, is_dirty: bool) -> String {
    if is_dirty {
        format!("{level_title} *")
    } else {
        level_title.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_needed_only_for_dirty_levels() {
        assert!(needs_save_prompt(true));
        assert!(!needs_save_prompt(false));
    }

    #[test]
    fn prompt_answers_map_to_outcomes() {
        assert_eq!(
            resolve_prompt(PromptAnswer::Save),
            PromptOutcome::SaveThenContinue
        );
        assert_eq!(resolve_prompt(PromptAnswer::DontSave), PromptOutcome::Continue);
        assert_eq!(resolve_prompt(PromptAnswer::Cancel), PromptOutcome::Abort);
    }

    #[test]
    fn save_asks_for_a_path_only_when_there_is_none() {
        assert_eq!(save_target(true), SaveTarget::Direct);
        assert_eq!(save_target(false), SaveTarget::AskPath);
    }

    #[test]
    fn level_items_are_disabled_without_a_level() {
        let closed = availability(false);
        assert!(!closed.save && !closed.save_as && !closed.close);
        let open = availability(true);
        assert!(open.save && open.save_as && open.close);
    }

    #[test]
    fn title_shows_a_star_while_dirty() {
        assert_eq!(window_title("main", true), "main *");
        assert_eq!(window_title("main", false), "main");
    }
}
