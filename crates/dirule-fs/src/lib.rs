#![warn(clippy::undocumented_unsafe_blocks)]

mod apply;
mod journal;
mod scan;
#[cfg(windows)]
mod windows_identity;

pub use apply::{
    ApplyError, ApplyReport, ConflictPolicy, JournaledApplyReport, apply, apply_journaled,
    apply_with_conflict_policy, validate_destinations,
};
pub use journal::{
    CURRENT_JOURNAL_VERSION, JournalDetails, JournalError, JournalOperation, JournalRun,
    JournalStatus, JournalStore, UndoReport,
};
pub use scan::{
    ScanError, ScanResult, scan, scan_with_entry_limit, scan_with_ignore_set, scan_with_ignores,
};
