//! Structured console log model: levels, categories, filters and export.

use serde::{Deserialize, Serialize};
use std::io;

/// Severity of a console line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogLevel {
    /// Informational.
    #[default]
    Info,
    /// Warning.
    Warn,
    /// Error.
    Error,
}

impl LogLevel {
    fn parse_prefix(line: &str) -> Option<(Self, &str)> {
        if let Some(rest) = line.strip_prefix("[INFO]") {
            return Some((Self::Info, rest.trim()));
        }
        if let Some(rest) = line.strip_prefix("[WARN]") {
            return Some((Self::Warn, rest.trim()));
        }
        if let Some(rest) = line.strip_prefix("[ERROR]") {
            return Some((Self::Error, rest.trim()));
        }
        None
    }
}

/// One console entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleEntry {
    /// Severity.
    pub level: LogLevel,
    /// Optional category (text inside `[category]` when present).
    pub category: Option<String>,
    /// Human-readable message.
    pub message: String,
    /// Entity GUID mentioned in the message (32 hex digits), if any.
    pub entity_guid: Option<String>,
}

impl ConsoleEntry {
    /// Parses a legacy line from [`redox_editor_core::EditorLog`].
    pub fn parse_line(line: &str) -> Self {
        let (level, mut text) = LogLevel::parse_prefix(line).unwrap_or((LogLevel::Info, line));
        let mut category = None;
        if let Some(rest) = text.strip_prefix('[') {
            if let Some((cat, after)) = rest.split_once(']') {
                category = Some(cat.trim().to_string());
                text = after.trim();
            }
        }
        let entity_guid = extract_guid(text);
        Self {
            level,
            category,
            message: text.to_string(),
            entity_guid,
        }
    }
}

/// Filter for the console list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConsoleFilter {
    /// Minimum level to show.
    pub min_level: LogLevel,
    /// When set, only this category is shown.
    pub category: Option<String>,
    /// Case-insensitive substring on the message body.
    pub text: String,
}

impl ConsoleFilter {
    /// Returns whether `entry` passes the filter.
    pub fn matches(&self, entry: &ConsoleEntry) -> bool {
        if entry.level.level_weight() < self.min_level_weight() {
            return false;
        }
        if let Some(cat) = &self.category
            && entry.category.as_deref() != Some(cat.as_str())
        {
            return false;
        }
        if !self.text.is_empty() {
            let needle = self.text.to_ascii_lowercase();
            let hay = entry.message.to_ascii_lowercase();
            if !hay.contains(&needle) {
                return false;
            }
        }
        true
    }
}

impl LogLevel {
    fn level_weight(self) -> u8 {
        match self {
            Self::Info => 0,
            Self::Warn => 1,
            Self::Error => 2,
        }
    }
}

impl ConsoleFilter {
    fn min_level_weight(&self) -> u8 {
        self.min_level.level_weight()
    }
}

/// Indexed view over filtered console lines.
#[derive(Clone, Debug, Default)]
pub struct ConsoleModel {
    entries: Vec<ConsoleEntry>,
}

impl ConsoleModel {
    /// Rebuilds from raw log lines.
    pub fn from_lines(lines: &[String]) -> Self {
        Self {
            entries: lines.iter().map(|l| ConsoleEntry::parse_line(l)).collect(),
        }
    }

    /// All entries.
    pub fn entries(&self) -> &[ConsoleEntry] {
        &self.entries
    }

    /// Filtered entries.
    pub fn filtered<'a>(
        &'a self,
        filter: &ConsoleFilter,
    ) -> impl Iterator<Item = &'a ConsoleEntry> {
        self.entries.iter().filter(|e| filter.matches(e))
    }

    /// Writes filtered lines to `path` as plain text.
    pub fn export_filtered(
        &self,
        filter: &ConsoleFilter,
        path: &std::path::Path,
    ) -> io::Result<()> {
        let mut out = String::new();
        for entry in self.filtered(filter) {
            let level = match entry.level {
                LogLevel::Info => "INFO",
                LogLevel::Warn => "WARN",
                LogLevel::Error => "ERROR",
            };
            if let Some(cat) = &entry.category {
                out.push_str(&format!("[{level}][{cat}] {}\n", entry.message));
            } else {
                out.push_str(&format!("[{level}] {}\n", entry.message));
            }
        }
        std::fs::write(path, out)
    }

    /// GUID from entry at `index` in the filtered view, if any.
    pub fn filtered_guid_at(&self, filter: &ConsoleFilter, index: usize) -> Option<&str> {
        self.filtered(filter)
            .nth(index)
            .and_then(|e| e.entity_guid.as_deref())
    }
}

fn extract_guid(text: &str) -> Option<String> {
    for token in text.split_whitespace() {
        if token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(token.to_ascii_lowercase());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn filter_respects_level_category_and_text() {
        let model = ConsoleModel::from_lines(&[
            "[INFO][physics] tick".into(),
            "[WARN][physics] slow".into(),
            "[ERROR][render] failed guid 0123456789abcdef0123456789abcdef".into(),
        ]);
        let filter = ConsoleFilter {
            min_level: LogLevel::Warn,
            category: Some("physics".into()),
            text: String::new(),
        };
        let hits: Vec<_> = model.filtered(&filter).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message, "slow");
    }

    #[test]
    fn export_writes_matching_lines() {
        let dir = std::env::temp_dir().join(format!("redox_console_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("out.log");
        let model = ConsoleModel::from_lines(&["[INFO] hello".into(), "[ERROR] boom".into()]);
        let filter = ConsoleFilter {
            min_level: LogLevel::Error,
            category: None,
            text: String::new(),
        };
        model.export_filtered(&filter, &path).expect("export");
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.contains("boom"));
        assert!(!text.contains("hello"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_extracts_guid_for_selection() {
        let entry =
            ConsoleEntry::parse_line("[INFO] Selected entity 0123456789abcdef0123456789abcdef");
        assert_eq!(
            entry.entity_guid.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
    }
}
