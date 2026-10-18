use crate::paths::{UserStateDirectoryUnavailable, user_journal_store};
use dirule_fs::{JournalDetails, JournalError, JournalRun, JournalStore, UndoReport};

/// Frontend-neutral access to applied-run history and undo operations.
pub struct JournalService {
    store: JournalStore,
}

impl JournalService {
    /// Create a service backed by a specific journal store.
    pub fn new(store: JournalStore) -> Self {
        Self { store }
    }

    /// Create a service backed by the journal files in `directory`.
    pub fn at(directory: impl Into<std::path::PathBuf>) -> Self {
        Self::new(JournalStore::new(directory))
    }

    /// Create a service using the platform's user state directory.
    pub fn user() -> Result<Self, UserStateDirectoryUnavailable> {
        user_journal_store().map(Self::new)
    }

    /// List recorded runs in reverse chronological order.
    pub fn list_runs(&self) -> Result<Vec<JournalRun>, JournalError> {
        self.store.list()
    }

    /// Read the recorded operations for one run.
    pub fn run_details(&self, run_id: &str) -> Result<JournalDetails, JournalError> {
        self.store.get(run_id)
    }

    /// Undo a selected run, subject to journal identity and conflict checks.
    pub fn undo_authorized(&self, run_id: &str) -> Result<UndoReport, JournalError> {
        self.store.undo(run_id)
    }

    pub(super) fn store(&self) -> &JournalStore {
        &self.store
    }
}
