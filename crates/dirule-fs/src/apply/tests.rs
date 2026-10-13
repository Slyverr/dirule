use super::{
    ApplyError, ConflictPolicy, EntryKind, JournalRecorder, JournalStatus, JournalStore,
    MutationRecorder, NoopRecorder, Plan, PlannedEntry, finish_failed_apply,
    rollback_after_failure,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("dirule-rollback-test-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temporary directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn rollback_failure_preserves_the_operation_error_and_occupied_paths() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("original.txt"), "external file")
        .expect("write new file at original path");
    std::fs::write(temp.0.join("renamed.txt"), "moved file")
        .expect("write moved file at destination");
    let moved = [(
        0,
        PathBuf::from("original.txt"),
        PathBuf::from("renamed.txt"),
        EntryKind::File,
    )];
    let operation = ApplyError::Io {
        path: PathBuf::from("later.txt"),
        source: std::io::Error::other("simulated later operation failure"),
    };

    let error = rollback_after_failure(operation, &temp.0, &moved, &[], &mut NoopRecorder);

    match error {
        ApplyError::RollbackFailed {
            operation,
            failures,
        } => {
            assert!(
                operation
                    .to_string()
                    .contains("simulated later operation failure")
            );
            assert!(failures.contains("renamed.txt -> original.txt"));
        }
        other => panic!("expected a rollback failure, got {other}"),
    }
    assert_eq!(
        std::fs::read_to_string(temp.0.join("original.txt")).expect("read external file"),
        "external file"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("renamed.txt")).expect("read moved file"),
        "moved file"
    );
}

struct JournalInterference<'a> {
    recorder: JournalRecorder<'a>,
    target: PathBuf,
    injected: bool,
}

impl MutationRecorder for JournalInterference<'_> {
    fn move_intent(
        &mut self,
        object: u64,
        source: &std::path::Path,
        destination: &std::path::Path,
        kind: EntryKind,
    ) -> Result<Option<u64>, ApplyError> {
        self.recorder.move_intent(object, source, destination, kind)
    }

    fn move_completed(
        &mut self,
        sequence: Option<u64>,
        destination: &std::path::Path,
        kind: EntryKind,
    ) -> Result<(), ApplyError> {
        self.recorder.move_completed(sequence, destination, kind)?;

        if !self.injected && destination == self.target.join("moved.txt") {
            std::fs::write(self.target.join("a-source.txt"), "external file")
                .expect("occupy the original path");
            std::fs::remove_file(self.target.join("z-source.txt"))
                .expect("remove the later source to fail apply");
            self.injected = true;
        }

        Ok(())
    }

    fn directory_intent(&mut self, directory: &std::path::Path) -> Result<Option<u64>, ApplyError> {
        self.recorder.directory_intent(directory)
    }

    fn directory_created(
        &mut self,
        sequence: Option<u64>,
        directory: &std::path::Path,
    ) -> Result<(), ApplyError> {
        self.recorder.directory_created(sequence, directory)
    }

    fn directory_remove_intent(
        &mut self,
        directory: &std::path::Path,
    ) -> Result<Option<u64>, ApplyError> {
        self.recorder.directory_remove_intent(directory)
    }

    fn directory_removed(&mut self, sequence: Option<u64>) -> Result<(), ApplyError> {
        self.recorder.directory_removed(sequence)
    }
}

#[test]
fn apply_journal_records_a_real_rollback_failure_as_partially_recovered() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project directory");
    std::fs::write(target.join("a-source.txt"), "first file").expect("write first source");
    std::fs::write(target.join("z-source.txt"), "second file").expect("write second source");
    let plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("a-source.txt"),
                destination: PathBuf::from("moved.txt"),
                kind: EntryKind::File,
                rule: Some("test".into()),
            },
            PlannedEntry {
                source: PathBuf::from("z-source.txt"),
                destination: PathBuf::from("z-moved.txt"),
                kind: EntryKind::File,
                rule: Some("test".into()),
            },
        ],
        ..Plan::default()
    };
    let snapshot = crate::scan(&target).expect("scan project");
    let store = JournalStore::new(temp.0.join("state/runs"));
    let mut recorder = JournalInterference {
        recorder: JournalRecorder::new(&store, &target, &plan),
        target: target.clone(),
        injected: false,
    };

    let error = super::apply_recorded(
        &target,
        &plan,
        &snapshot,
        ConflictPolicy::Abort,
        &mut recorder,
    )
    .expect_err("missing second source should fail apply");
    let run_id = recorder
        .recorder
        .id()
        .expect("first move starts a journal run")
        .to_owned();
    let error = finish_failed_apply(&mut recorder.recorder, run_id.clone(), error);
    drop(recorder);

    let runs = store.list().expect("list failed run");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, run_id);
    assert_eq!(runs[0].status, JournalStatus::PartiallyRecovered);
    let details = store.get(&run_id).expect("read partial recovery details");
    assert_eq!(details.operations.len(), 2);
    assert!(details.operations.iter().any(|operation| {
        operation.source.as_path() == std::path::Path::new("a-source.txt")
            && operation.destination.as_path() == std::path::Path::new("moved.txt")
    }));
    assert!(details.operations.iter().any(|operation| {
        operation.source.as_path() == std::path::Path::new("moved.txt")
            && operation.destination.as_path() == std::path::Path::new("a-source.txt")
    }));
    assert!(error.to_string().contains("rollback also failed"));
    assert_eq!(
        std::fs::read_to_string(target.join("a-source.txt")).expect("read external file"),
        "external file"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("moved.txt")).expect("read moved file"),
        "first file"
    );
}
