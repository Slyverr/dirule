use super::{
    ApplyError, ConflictPolicy, EntryKind, FileTextEdit, JournalRecorder, JournalStatus,
    JournalStore, MutationRecorder, NoopRecorder, Plan, PlannedEntry, applied_run_result,
    apply_recorded, failed_apply_result, finish_failed_apply, rollback_after_failure,
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

struct FailSecondStaging {
    intents: usize,
}

struct BreakTextEditWrite {
    path: PathBuf,
}

impl MutationRecorder for BreakTextEditWrite {
    fn move_intent(
        &mut self,
        _object: u64,
        _source: &std::path::Path,
        _destination: &std::path::Path,
        _kind: EntryKind,
    ) -> Result<Option<u64>, ApplyError> {
        Ok(None)
    }

    fn move_completed(
        &mut self,
        _sequence: Option<u64>,
        _destination: &std::path::Path,
        _kind: EntryKind,
    ) -> Result<(), ApplyError> {
        Ok(())
    }

    fn directory_intent(
        &mut self,
        _directory: &std::path::Path,
    ) -> Result<Option<u64>, ApplyError> {
        Ok(None)
    }

    fn directory_created(
        &mut self,
        _sequence: Option<u64>,
        _directory: &std::path::Path,
    ) -> Result<(), ApplyError> {
        Ok(())
    }

    fn directory_remove_intent(
        &mut self,
        _directory: &std::path::Path,
    ) -> Result<Option<u64>, ApplyError> {
        Ok(None)
    }

    fn directory_removed(&mut self, _sequence: Option<u64>) -> Result<(), ApplyError> {
        Ok(())
    }

    fn text_edit_intent(
        &mut self,
        path: &std::path::Path,
        _before: &str,
        _after: &str,
    ) -> Result<Option<u64>, ApplyError> {
        if path == self.path {
            std::fs::remove_file(path).expect("remove the pending text document");
            std::fs::create_dir(path).expect("replace the pending text document with a directory");
        }
        Ok(None)
    }
}

impl MutationRecorder for FailSecondStaging {
    fn move_intent(
        &mut self,
        _object: u64,
        source: &std::path::Path,
        _destination: &std::path::Path,
        _kind: EntryKind,
    ) -> Result<Option<u64>, ApplyError> {
        self.intents += 1;
        if self.intents == 2 {
            return Err(ApplyError::Io {
                path: source.to_path_buf(),
                source: std::io::Error::other("simulated second staging failure"),
            });
        }

        Ok(None)
    }

    fn move_completed(
        &mut self,
        _sequence: Option<u64>,
        _destination: &std::path::Path,
        _kind: EntryKind,
    ) -> Result<(), ApplyError> {
        Ok(())
    }

    fn directory_intent(
        &mut self,
        _directory: &std::path::Path,
    ) -> Result<Option<u64>, ApplyError> {
        Ok(None)
    }

    fn directory_created(
        &mut self,
        _sequence: Option<u64>,
        _directory: &std::path::Path,
    ) -> Result<(), ApplyError> {
        Ok(())
    }

    fn directory_remove_intent(
        &mut self,
        _directory: &std::path::Path,
    ) -> Result<Option<u64>, ApplyError> {
        Ok(None)
    }

    fn directory_removed(&mut self, _sequence: Option<u64>) -> Result<(), ApplyError> {
        Ok(())
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum ReplaceDuring {
    Intent,
    Completion,
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum ReplaceTarget {
    Parent,
    Source,
}

#[cfg(unix)]
struct JournalPathInterference<'a> {
    recorder: JournalRecorder<'a>,
    target: PathBuf,
    outside: PathBuf,
    replace_during: ReplaceDuring,
    replace_target: ReplaceTarget,
    replaced: bool,
}

#[cfg(unix)]
impl JournalPathInterference<'_> {
    fn replace_path(&mut self) {
        match self.replace_target {
            ReplaceTarget::Parent => {
                std::fs::rename(self.target.join("nested"), self.target.join("nested.saved"))
                    .expect("move the original source parent");
                std::os::unix::fs::symlink(&self.outside, self.target.join("nested"))
                    .expect("replace the source parent with a symlink");
            }
            ReplaceTarget::Source => {
                std::fs::rename(
                    self.target.join("nested/file.txt"),
                    self.target.join("nested/original.saved"),
                )
                .expect("move the original source file");
                std::fs::write(self.target.join("nested/file.txt"), "replacement")
                    .expect("replace the original source file");
            }
        }

        self.replaced = true;
    }
}

#[cfg(unix)]
impl MutationRecorder for JournalPathInterference<'_> {
    fn move_intent(
        &mut self,
        object: u64,
        source: &std::path::Path,
        destination: &std::path::Path,
        kind: EntryKind,
    ) -> Result<Option<u64>, ApplyError> {
        let sequence = self
            .recorder
            .move_intent(object, source, destination, kind)?;
        if self.replace_during == ReplaceDuring::Intent && !self.replaced {
            self.replace_path();
        }

        Ok(sequence)
    }

    fn move_completed(
        &mut self,
        sequence: Option<u64>,
        destination: &std::path::Path,
        kind: EntryKind,
    ) -> Result<(), ApplyError> {
        if self.replace_during == ReplaceDuring::Completion && !self.replaced {
            self.replace_path();

            return Err(ApplyError::Io {
                path: destination.to_path_buf(),
                source: std::io::Error::other("simulated journal completion failure"),
            });
        }

        self.recorder.move_completed(sequence, destination, kind)
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

#[cfg(unix)]
#[test]
fn apply_rejects_a_symlinked_parent_installed_during_journal_intent() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    let outside = temp.0.join("outside");
    std::fs::create_dir_all(target.join("nested")).expect("create source parent");
    std::fs::create_dir(&outside).expect("create outside directory");
    std::fs::write(target.join("nested/file.txt"), "inside").expect("write source");
    std::fs::write(outside.join("file.txt"), "outside").expect("write outside file");
    let plan = Plan {
        entries: vec![PlannedEntry {
            source: PathBuf::from("nested/file.txt"),
            destination: PathBuf::from("renamed.txt"),
            kind: EntryKind::File,
            rule: Some("test".into()),
        }],
        ..Plan::default()
    };
    let snapshot = crate::scan(&target).expect("scan project");
    let store = JournalStore::new(temp.0.join("state/runs"));
    let mut recorder = JournalPathInterference {
        recorder: JournalRecorder::new(&store, &target, &plan),
        target: target.clone(),
        outside: outside.clone(),
        replace_during: ReplaceDuring::Intent,
        replace_target: ReplaceTarget::Parent,
        replaced: false,
    };

    let error = super::apply_recorded(
        &target,
        &plan,
        &snapshot,
        ConflictPolicy::Abort,
        &[],
        &mut recorder,
    )
    .expect_err("apply must reject the replaced source parent");

    assert!(matches!(
        &error,
        ApplyError::UnresolvedOperation(operation)
            if matches!(operation.as_ref(), ApplyError::UnsafePath(_))
    ));

    let run_id = recorder
        .recorder
        .id()
        .expect("move intent starts a journal run")
        .to_owned();
    let error = finish_failed_apply(&mut recorder.recorder, run_id.clone(), error);
    drop(recorder);

    let runs = store.list().expect("list partially recovered run");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, JournalStatus::PartiallyRecovered);
    let details = store.get(&run_id).expect("read unresolved run details");
    assert_eq!(details.operations.len(), 1);
    assert!(
        store.undo(&run_id).is_err(),
        "undo must reject the ambiguous source operation"
    );
    assert!(matches!(&error, ApplyError::RunFailed { .. }));
    assert_eq!(std::fs::read(outside.join("file.txt")).unwrap(), b"outside");
    assert_eq!(
        std::fs::read(target.join("nested.saved/file.txt")).unwrap(),
        b"inside"
    );
    assert!(!target.join("renamed.txt").exists());
}

#[cfg(unix)]
#[test]
fn apply_does_not_follow_a_replaced_parent_during_journal_completion_rollback() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    let outside = temp.0.join("outside");
    std::fs::create_dir_all(target.join("nested")).expect("create source parent");
    std::fs::create_dir(&outside).expect("create outside directory");
    std::fs::write(target.join("nested/file.txt"), "inside").expect("write source");
    let plan = Plan {
        entries: vec![PlannedEntry {
            source: PathBuf::from("nested/file.txt"),
            destination: PathBuf::from("renamed.txt"),
            kind: EntryKind::File,
            rule: Some("test".into()),
        }],
        ..Plan::default()
    };
    let snapshot = crate::scan(&target).expect("scan project");
    let store = JournalStore::new(temp.0.join("state/runs"));
    let mut recorder = JournalPathInterference {
        recorder: JournalRecorder::new(&store, &target, &plan),
        target: target.clone(),
        outside: outside.clone(),
        replace_during: ReplaceDuring::Completion,
        replace_target: ReplaceTarget::Parent,
        replaced: false,
    };

    let error = super::apply_recorded(
        &target,
        &plan,
        &snapshot,
        ConflictPolicy::Abort,
        &[],
        &mut recorder,
    )
    .expect_err("journal completion failure must stop the apply");

    assert!(matches!(&error, ApplyError::RollbackFailed { .. }));

    let run_id = recorder
        .recorder
        .id()
        .expect("move intent starts a journal run")
        .to_owned();
    let error = finish_failed_apply(&mut recorder.recorder, run_id.clone(), error);
    drop(recorder);

    let runs = store.list().expect("list partially recovered run");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, JournalStatus::PartiallyRecovered);
    let details = store.get(&run_id).expect("read unresolved run details");
    assert_eq!(details.operations.len(), 1);
    assert!(
        store.undo(&run_id).is_err(),
        "undo must reject the operation whose source parent is now a symlink"
    );
    assert!(matches!(
        &error,
        ApplyError::RunFailed { source, .. }
            if matches!(source.as_ref(), ApplyError::RollbackFailed { .. })
    ));

    assert!(!outside.join("file.txt").exists());
    assert_eq!(
        std::fs::read(target.join("renamed.txt")).unwrap(),
        b"inside"
    );
}

#[cfg(unix)]
#[test]
fn apply_rejects_a_replaced_source_after_journal_intent() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    let outside = temp.0.join("outside");
    std::fs::create_dir_all(target.join("nested")).expect("create source parent");
    std::fs::create_dir(&outside).expect("create outside directory");
    std::fs::write(target.join("nested/file.txt"), "inside").expect("write source");
    let plan = Plan {
        entries: vec![PlannedEntry {
            source: PathBuf::from("nested/file.txt"),
            destination: PathBuf::from("renamed.txt"),
            kind: EntryKind::File,
            rule: Some("test".into()),
        }],
        ..Plan::default()
    };
    let snapshot = crate::scan(&target).expect("scan project");
    let store = JournalStore::new(temp.0.join("state/runs"));
    let mut recorder = JournalPathInterference {
        recorder: JournalRecorder::new(&store, &target, &plan),
        target: target.clone(),
        outside,
        replace_during: ReplaceDuring::Intent,
        replace_target: ReplaceTarget::Source,
        replaced: false,
    };

    let error = super::apply_recorded(
        &target,
        &plan,
        &snapshot,
        ConflictPolicy::Abort,
        &[],
        &mut recorder,
    )
    .expect_err("apply must reject a replaced source file");

    assert!(matches!(
        &error,
        ApplyError::UnresolvedOperation(operation)
            if matches!(operation.as_ref(), ApplyError::ChangedSource(_))
    ));

    let run_id = recorder
        .recorder
        .id()
        .expect("move intent starts a journal run")
        .to_owned();
    let error = finish_failed_apply(&mut recorder.recorder, run_id.clone(), error);
    drop(recorder);

    let runs = store.list().expect("list partially recovered run");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, JournalStatus::PartiallyRecovered);
    assert_eq!(
        store
            .get(&run_id)
            .expect("read unresolved run details")
            .operations
            .len(),
        1
    );
    assert!(store.undo(&run_id).is_err());
    assert!(matches!(&error, ApplyError::RunFailed { .. }));
    assert_eq!(
        std::fs::read(target.join("nested/file.txt")).unwrap(),
        b"replacement"
    );
    assert_eq!(
        std::fs::read(target.join("nested/original.saved")).unwrap(),
        b"inside"
    );
    assert!(!target.join("renamed.txt").exists());
}

#[test]
fn partial_cycle_staging_rolls_back_before_apply_returns() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("first"), "first").expect("write first source");
    std::fs::write(temp.0.join("second"), "second").expect("write second source");
    let target = temp.0.clone();
    let plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("first"),
                destination: PathBuf::from("second"),
                kind: EntryKind::File,
                rule: Some("swap".into()),
            },
            PlannedEntry {
                source: PathBuf::from("second"),
                destination: PathBuf::from("first"),
                kind: EntryKind::File,
                rule: Some("swap".into()),
            },
        ],
        ..Plan::default()
    };
    let snapshot = crate::scan(&target).expect("scan project");
    let mut recorder = FailSecondStaging { intents: 0 };

    let error = super::apply_recorded(
        &target,
        &plan,
        &snapshot,
        ConflictPolicy::Abort,
        &[],
        &mut recorder,
    )
    .expect_err("second staging operation should fail");

    assert!(
        error
            .to_string()
            .contains("simulated second staging failure")
    );
    assert_eq!(std::fs::read(target.join("first")).unwrap(), b"first");
    assert_eq!(std::fs::read(target.join("second")).unwrap(), b"second");
    assert_eq!(std::fs::read_dir(target).unwrap().count(), 2);
}

#[test]
fn rollback_failure_preserves_the_operation_error_and_occupied_paths() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("original.txt"), "external file")
        .expect("write new file at original path");
    std::fs::write(temp.0.join("renamed.txt"), "moved file")
        .expect("write moved file at destination");
    let snapshot = crate::scan(&temp.0).expect("scan rollback target");
    let moved = [(
        0,
        PathBuf::from("original.txt"),
        PathBuf::from("renamed.txt"),
        EntryKind::File,
        snapshot
            .identities
            .get(std::path::Path::new("renamed.txt"))
            .expect("identity of moved file")
            .clone(),
    )];
    let operation = ApplyError::Io {
        path: PathBuf::from("later.txt"),
        source: std::io::Error::other("simulated later operation failure"),
    };
    let target_guard = super::path::TargetGuard::new(
        &temp.0,
        snapshot.target_identity.as_ref().expect("target identity"),
    );

    let error = rollback_after_failure(operation, &target_guard, &moved, &[], &mut NoopRecorder);

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
        if !self.injected
            && destination
                .file_name()
                .is_some_and(|name| name == "z-moved.txt")
        {
            std::fs::write(self.target.join("a-source.txt"), "external file")
                .expect("occupy the original path");
            self.injected = true;

            return Err(ApplyError::Io {
                path: source.to_path_buf(),
                source: std::io::Error::other("simulated later operation failure"),
            });
        }

        self.recorder.move_intent(object, source, destination, kind)
    }

    fn move_completed(
        &mut self,
        sequence: Option<u64>,
        destination: &std::path::Path,
        kind: EntryKind,
    ) -> Result<(), ApplyError> {
        self.recorder.move_completed(sequence, destination, kind)
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
        &[],
        &mut recorder,
    )
    .expect_err("simulated later operation failure should trigger rollback");
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

    assert!(
        store.undo(&run_id).is_err(),
        "do not overwrite the external file"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("a-source.txt")).expect("read external file"),
        "external file"
    );
    let external = temp.0.join("external.txt");
    std::fs::rename(target.join("a-source.txt"), &external)
        .expect("move the conflicting file out of the way");

    store
        .undo(&run_id)
        .expect("retry recovery after resolving the path conflict");

    assert_eq!(
        std::fs::read_to_string(target.join("a-source.txt")).expect("read restored source"),
        "first file"
    );
    assert!(!target.join("moved.txt").exists());
    assert_eq!(
        std::fs::read_to_string(external).expect("read preserved external file"),
        "external file"
    );
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
}

#[test]
fn journal_finalization_errors_preserve_apply_outcomes() {
    let failed_operation = ApplyError::MissingSource(PathBuf::from("later.txt"));
    let error = failed_apply_result(
        "failed-run".into(),
        failed_operation,
        Err(crate::journal::JournalError::Io {
            path: PathBuf::from("failed-run.jsonl"),
            source: std::io::Error::other("simulated status write failure"),
        }),
    );

    match error {
        ApplyError::RunFinalizationFailed {
            run_id,
            operation,
            journal,
        } => {
            assert_eq!(run_id, "failed-run");
            assert!(operation.to_string().contains("later.txt"));
            assert!(
                journal
                    .to_string()
                    .contains("simulated status write failure")
            );
        }
        other => panic!("expected both failures to be preserved, got {other}"),
    }

    let error = applied_run_result(
        "applied-run".into(),
        Err(crate::journal::JournalError::Io {
            path: PathBuf::from("applied-run.jsonl"),
            source: std::io::Error::other("simulated status sync failure"),
        }),
    )
    .expect_err("failed status write must report the completed filesystem change");
    let display = error.to_string();

    match error {
        ApplyError::AppliedRunFinalizationFailed { run_id, source } => {
            assert_eq!(run_id, "applied-run");
            assert!(source.to_string().contains("simulated status sync failure"));
        }
        other => panic!("expected completed apply to be distinguished, got {other}"),
    }
    assert!(display.contains("filesystem changes completed in run applied-run"));
}

#[test]
fn apply_rejects_changed_text_before_mutating_the_path_plan() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project target");
    std::fs::write(target.join("HTTPClient.rs"), "pub struct Client;\n")
        .expect("write renamed source");
    std::fs::write(target.join("references.rs"), "mod HTTPClient;\n")
        .expect("write reference document");

    let snapshot = crate::scan(&target).expect("scan project before planning");
    let plan = Plan {
        entries: vec![PlannedEntry {
            source: PathBuf::from("HTTPClient.rs"),
            destination: PathBuf::from("http_client.rs"),
            kind: EntryKind::File,
            rule: Some("normalize Rust module names".into()),
        }],
        ..Plan::default()
    };
    let edit = FileTextEdit {
        path: PathBuf::from("references.rs"),
        before: "mod HTTPClient;\n".into(),
        after: "mod http_client;\n".into(),
    };
    std::fs::write(target.join("references.rs"), "mod externally_changed;\n")
        .expect("change reference document after planning");

    let store = JournalStore::new(temp.0.join("state/runs"));
    let error = super::apply_journaled_with_text_edits(
        &target,
        &plan,
        &snapshot,
        ConflictPolicy::Abort,
        &[edit],
        &store,
    )
    .expect_err("stale reference edits must fail before path mutations");

    assert!(
        matches!(&error, ApplyError::ChangedText(path) if path == std::path::Path::new("references.rs")),
        "expected the stale text precondition error directly, got {error}"
    );
    assert!(target.join("HTTPClient.rs").is_file());
    assert!(!target.join("http_client.rs").exists());
    assert_eq!(
        std::fs::read_to_string(target.join("references.rs")).unwrap(),
        "mod externally_changed;\n"
    );
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn text_edit_write_failure_rolls_back_prior_edits_and_path_moves() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir_all(target.join("src")).expect("create source directory");
    std::fs::write(target.join("src/HTTPClient.rs"), "pub struct Client;\n")
        .expect("write renamed source");
    std::fs::write(target.join("src/api.rs"), "use crate::HTTPClient;\n")
        .expect("write first reference document");
    std::fs::write(target.join("src/lib.rs"), "pub mod HTTPClient;\n")
        .expect("write failing reference document");

    let snapshot = crate::scan(&target).expect("scan project");
    let plan = Plan {
        entries: vec![PlannedEntry {
            source: PathBuf::from("src/HTTPClient.rs"),
            destination: PathBuf::from("src/http_client.rs"),
            kind: EntryKind::File,
            rule: Some("normalize Rust module names".into()),
        }],
        ..Plan::default()
    };
    let edits = [
        FileTextEdit {
            path: PathBuf::from("src/api.rs"),
            before: "use crate::HTTPClient;\n".into(),
            after: "use crate::http_client;\n".into(),
        },
        FileTextEdit {
            path: PathBuf::from("src/lib.rs"),
            before: "pub mod HTTPClient;\n".into(),
            after: "pub mod http_client;\n".into(),
        },
    ];
    let mut recorder = BreakTextEditWrite {
        path: target.join("src/lib.rs"),
    };

    let error = apply_recorded(
        &target,
        &plan,
        &snapshot,
        ConflictPolicy::Abort,
        &edits,
        &mut recorder,
    )
    .expect_err("simulated text document write failure should abort apply");

    assert!(matches!(error, ApplyError::Io { .. }));
    assert!(target.join("src/HTTPClient.rs").is_file());
    assert!(!target.join("src/http_client.rs").exists());
    assert_eq!(
        std::fs::read_to_string(target.join("src/api.rs")).unwrap(),
        "use crate::HTTPClient;\n"
    );
    assert!(target.join("src/lib.rs").is_dir());
}
