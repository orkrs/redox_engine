//! Import log model fed from import queue events (EDT-B4 step 4).

use std::path::PathBuf;

use redox_asset::AssetGuid;

use crate::import_queue::ImportEvent;

/// Status filter for the import log list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImportLogFilter {
    /// Every row.
    #[default]
    All,
    /// Only active or finished imports.
    Active,
    /// Only failures and cancellations.
    Problems,
}

/// Status column of one log row.
#[derive(Clone, Debug, PartialEq)]
pub enum ImportLogStatus {
    /// Worker picked up the job.
    Started,
    /// Fraction in `0.0..=1.0`.
    Progress(f32),
    /// Import succeeded.
    Finished,
    /// Import failed.
    Failed(String),
    /// User or queue cancelled the job.
    Cancelled,
}

/// One import log entry.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportLogEntry {
    /// Path relative to the assets root.
    pub path: String,
    /// Asset identity.
    pub guid: AssetGuid,
    /// Latest status.
    pub status: ImportLogStatus,
    /// Monotonic sequence for stable sorting.
    pub seq: u64,
}

/// Import log with filters; UI maps rows to Open source / Retry actions.
#[derive(Clone, Debug, Default)]
pub struct ImportLogModel {
    rows: Vec<ImportLogEntry>,
    by_path: std::collections::HashMap<String, usize>,
    filter: ImportLogFilter,
    next_seq: u64,
}

impl ImportLogModel {
    /// Creates an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current status filter.
    pub fn filter(&self) -> ImportLogFilter {
        self.filter
    }

    /// Sets the status filter.
    pub fn set_filter(&mut self, filter: ImportLogFilter) {
        self.filter = filter;
    }

    /// Rows after the active filter, newest activity first within a path group.
    pub fn visible(&self) -> Vec<&ImportLogEntry> {
        let mut rows: Vec<&ImportLogEntry> = self
            .rows
            .iter()
            .filter(|row| self.matches_filter(&row.status))
            .collect();
        rows.sort_by(|a, b| b.seq.cmp(&a.seq));
        rows
    }

    /// Every stored row.
    pub fn all(&self) -> &[ImportLogEntry] {
        &self.rows
    }

    /// Ingests one queue event (absolute paths are normalised by the caller).
    pub fn ingest(&mut self, rel_path: &str, guid: AssetGuid, event: &ImportEvent) {
        self.next_seq += 1;
        let status = match event {
            ImportEvent::Started { .. } => ImportLogStatus::Started,
            ImportEvent::Progress { fraction, .. } => ImportLogStatus::Progress(*fraction),
            ImportEvent::Finished { .. } => ImportLogStatus::Finished,
            ImportEvent::Failed { message, .. } => ImportLogStatus::Failed(message.clone()),
            ImportEvent::Cancelled { .. } => ImportLogStatus::Cancelled,
        };
        if let Some(&index) = self.by_path.get(rel_path) {
            self.rows[index].status = status;
            self.rows[index].seq = self.next_seq;
        } else {
            let index = self.rows.len();
            self.rows.push(ImportLogEntry {
                path: rel_path.to_owned(),
                guid,
                status,
                seq: self.next_seq,
            });
            self.by_path.insert(rel_path.to_owned(), index);
        }
    }

    /// Paths whose last status is failed or cancelled (for retry buttons).
    pub fn retryable_paths(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|row| {
                matches!(
                    row.status,
                    ImportLogStatus::Failed(_) | ImportLogStatus::Cancelled
                )
            })
            .map(|row| row.path.clone())
            .collect()
    }

    /// Absolute source path for Reveal when the assets root is known.
    pub fn source_path<'a>(&self, row: &ImportLogEntry, root: &'a std::path::Path) -> PathBuf {
        root.join(&row.path)
    }

    fn matches_filter(&self, status: &ImportLogStatus) -> bool {
        match self.filter {
            ImportLogFilter::All => true,
            ImportLogFilter::Active => matches!(
                status,
                ImportLogStatus::Started | ImportLogStatus::Progress(_) | ImportLogStatus::Finished
            ),
            ImportLogFilter::Problems => matches!(
                status,
                ImportLogStatus::Failed(_) | ImportLogStatus::Cancelled
            ),
        }
    }
}
