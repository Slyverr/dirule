use crate::paths::{UserStateDirectoryUnavailable, user_journal_store};
use dirule_fs::{JournalDetails, JournalError, JournalRun, JournalStore, UndoReport};

pub struct JournalService {
    store: JournalStore,
}

impl JournalService {
    pub fn new(store: JournalStore) -> Self {
        Self { store }
    }

    pub fn user() -> Result<Self, UserStateDirectoryUnavailable> {
        user_journal_store().map(Self::new)
    }

    pub fn list_runs(&self) -> Result<Vec<JournalRun>, JournalError> {
        self.store.list()
    }

    pub fn run_details(&self, run_id: &str) -> Result<JournalDetails, JournalError> {
        self.store.get(run_id)
    }

    pub fn undo_authorized(&self, run_id: &str) -> Result<UndoReport, JournalError> {
        self.store.undo(run_id)
    }

    pub(super) fn store(&self) -> &JournalStore {
        &self.store
    }
}
