use crate::ScanResult;
use crate::journal::{JournalRecorder, JournalStatus, JournalStore};
use crate::scan::SourceIdentity;
use dirule_core::{Conflict, EntryKind, Plan, PlannedEntry};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

#[path = "apply/dependencies.rs"]
mod dependencies;
#[path = "apply/entry.rs"]
mod entry;
#[path = "apply/path.rs"]
mod path;

use entry::apply_entry;

static NEXT_TEMPORARY_RENAME: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApplyReport {
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub skipped: Vec<Conflict>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JournaledApplyReport {
    pub report: ApplyReport,
    pub run_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConflictPolicy {
    #[default]
    Abort,
    Skip,
}

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error("plan has {0} conflict(s); resolve them before applying")]
    Conflicts(usize),
    #[error("source no longer exists: {0}")]
    MissingSource(PathBuf),
    #[error("destination already exists: {0}")]
    ExistingDestination(PathBuf),
    #[error("planned path is unsafe or unsupported on this filesystem: {0}")]
    UnsafePath(PathBuf),
    #[error("filesystem state changed after a journal intent: {0}")]
    UnresolvedOperation(#[source] Box<ApplyError>),
    #[error("filesystem does not support atomic no-replace renames at: {0}")]
    AtomicRenameUnsupported(PathBuf),
    #[error("source type changed since planning: {0}")]
    ChangedSource(PathBuf),
    #[error("target directory changed since planning: {0}")]
    ChangedTarget(PathBuf),
    #[error("journal operation failed: {0}")]
    Journal(#[source] Box<crate::journal::JournalError>),
    #[error("apply failed in run {run_id}: {source}")]
    RunFailed {
        run_id: String,
        #[source]
        source: Box<ApplyError>,
    },
    #[error(
        "apply failed in run {run_id}: {operation}; could not record the final journal status: {journal}"
    )]
    RunFinalizationFailed {
        run_id: String,
        operation: Box<ApplyError>,
        #[source]
        journal: Box<crate::journal::JournalError>,
    },
    #[error(
        "filesystem changes completed in run {run_id}, but recording the final journal status failed: {source}"
    )]
    AppliedRunFinalizationFailed {
        run_id: String,
        #[source]
        source: Box<crate::journal::JournalError>,
    },
    #[error("filesystem operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{operation}; rollback also failed: {failures}")]
    RollbackFailed {
        #[source]
        operation: Box<ApplyError>,
        failures: String,
    },
}

pub fn validate_destinations(target: &Path, plan: &mut Plan) -> Result<(), ApplyError> {
    let target = target.canonicalize().map_err(|source| ApplyError::Io {
        path: target.to_path_buf(),
        source,
    })?;
    let vacated_paths = path::VacatedPaths::new(&target, &plan.entries);

    for entry in &plan.entries {
        path::ensure_relative(&entry.source)?;
        path::ensure_relative(&entry.destination)?;
        if entry.source == entry.destination {
            continue;
        }
        let destination = target.join(&entry.destination);
        match path::ensure_parent_safe(&target, destination.parent().unwrap_or(&target)) {
            Ok(()) => {}
            Err(ApplyError::ExistingDestination(path))
                if vacated_paths.contains(&target, &path) => {}
            Err(error @ (ApplyError::ExistingDestination(_) | ApplyError::UnsafePath(_))) => {
                if !plan
                    .conflicts
                    .iter()
                    .any(|conflict| conflict.path == entry.destination)
                {
                    plan.conflicts.push(Conflict {
                        path: entry.destination.clone(),
                        reason: error.to_string(),
                    });
                }
                continue;
            }
            Err(error) => return Err(error),
        }
        if path::destination_exists(&destination)?
            && !vacated_paths.contains(&target, &destination)
            && !plan
                .conflicts
                .iter()
                .any(|conflict| conflict.path == entry.destination)
        {
            plan.conflicts.push(Conflict {
                path: entry.destination.clone(),
                reason: "destination already exists on disk".into(),
            });
        }
    }

    dependencies::add_dependent_move_conflicts(&target, plan);

    Ok(())
}

pub fn apply(target: &Path, plan: &Plan, snapshot: &ScanResult) -> Result<ApplyReport, ApplyError> {
    apply_with_conflict_policy(target, plan, snapshot, ConflictPolicy::Abort)
}

pub fn apply_with_conflict_policy(
    target: &Path,
    plan: &Plan,
    snapshot: &ScanResult,
    policy: ConflictPolicy,
) -> Result<ApplyReport, ApplyError> {
    let mut recorder = NoopRecorder;
    apply_recorded(target, plan, snapshot, policy, &mut recorder)
}

pub fn apply_journaled(
    target: &Path,
    plan: &Plan,
    snapshot: &ScanResult,
    policy: ConflictPolicy,
    journal_store: &JournalStore,
) -> Result<JournaledApplyReport, ApplyError> {
    let mut recorder = JournalRecorder::new(journal_store, target, plan);
    let result = apply_recorded(target, plan, snapshot, policy, &mut recorder);
    let Some(run_id) = recorder.id().map(str::to_owned) else {
        return result.map(|report| JournaledApplyReport {
            report,
            run_id: None,
        });
    };

    match result {
        Ok(report) => {
            applied_run_result(run_id.clone(), recorder.finish(JournalStatus::Applied))?;
            Ok(JournaledApplyReport {
                report,
                run_id: Some(run_id),
            })
        }
        Err(source) => Err(finish_failed_apply(&mut recorder, run_id, source)),
    }
}

fn finish_failed_apply(
    recorder: &mut JournalRecorder<'_>,
    run_id: String,
    source: ApplyError,
) -> ApplyError {
    let status = failed_apply_status(&source);
    let finalization = recorder.finish(status);

    failed_apply_result(run_id, source, finalization)
}

fn failed_apply_status(source: &ApplyError) -> JournalStatus {
    if matches!(
        source,
        ApplyError::RollbackFailed { .. } | ApplyError::UnresolvedOperation(_)
    ) {
        JournalStatus::PartiallyRecovered
    } else {
        JournalStatus::FailedRolledBack
    }
}

fn failed_apply_result(
    run_id: String,
    operation: ApplyError,
    finalization: Result<(), crate::journal::JournalError>,
) -> ApplyError {
    match finalization {
        Ok(()) => ApplyError::RunFailed {
            run_id,
            source: Box::new(operation),
        },
        Err(journal) => ApplyError::RunFinalizationFailed {
            run_id,
            operation: Box::new(operation),
            journal: Box::new(journal),
        },
    }
}

fn applied_run_result(
    run_id: String,
    finalization: Result<(), crate::journal::JournalError>,
) -> Result<(), ApplyError> {
    finalization.map_err(|source| ApplyError::AppliedRunFinalizationFailed {
        run_id,
        source: Box::new(source),
    })
}

pub(crate) trait MutationRecorder {
    fn move_intent(
        &mut self,
        object: u64,
        source: &Path,
        destination: &Path,
        kind: EntryKind,
    ) -> Result<Option<u64>, ApplyError>;

    fn move_completed(
        &mut self,
        sequence: Option<u64>,
        destination: &Path,
        kind: EntryKind,
    ) -> Result<(), ApplyError>;

    fn directory_intent(&mut self, directory: &Path) -> Result<Option<u64>, ApplyError>;

    fn directory_created(
        &mut self,
        sequence: Option<u64>,
        directory: &Path,
    ) -> Result<(), ApplyError>;

    fn directory_remove_intent(&mut self, directory: &Path) -> Result<Option<u64>, ApplyError>;

    fn directory_removed(&mut self, sequence: Option<u64>) -> Result<(), ApplyError>;
}

struct NoopRecorder;

impl MutationRecorder for NoopRecorder {
    fn move_intent(
        &mut self,
        _object: u64,
        _source: &Path,
        _destination: &Path,
        _kind: EntryKind,
    ) -> Result<Option<u64>, ApplyError> {
        Ok(None)
    }

    fn move_completed(
        &mut self,
        _sequence: Option<u64>,
        _destination: &Path,
        _kind: EntryKind,
    ) -> Result<(), ApplyError> {
        Ok(())
    }

    fn directory_intent(&mut self, _directory: &Path) -> Result<Option<u64>, ApplyError> {
        Ok(None)
    }

    fn directory_created(
        &mut self,
        _sequence: Option<u64>,
        _directory: &Path,
    ) -> Result<(), ApplyError> {
        Ok(())
    }

    fn directory_remove_intent(&mut self, _directory: &Path) -> Result<Option<u64>, ApplyError> {
        Ok(None)
    }

    fn directory_removed(&mut self, _sequence: Option<u64>) -> Result<(), ApplyError> {
        Ok(())
    }
}

fn apply_recorded(
    target: &Path,
    plan: &Plan,
    snapshot: &ScanResult,
    policy: ConflictPolicy,
    recorder: &mut dyn MutationRecorder,
) -> Result<ApplyReport, ApplyError> {
    let target = target.canonicalize().map_err(|source| ApplyError::Io {
        path: target.to_path_buf(),
        source,
    })?;
    let target_metadata = std::fs::symlink_metadata(&target).map_err(|source| ApplyError::Io {
        path: target.clone(),
        source,
    })?;
    let target_identity = snapshot
        .target_identity
        .as_ref()
        .ok_or_else(|| ApplyError::ChangedTarget(target.clone()))?;
    if !target_metadata.is_dir()
        || !target_identity.matches_object(&target_metadata, EntryKind::Directory, &target)
    {
        return Err(ApplyError::ChangedTarget(target));
    }
    let target_guard = path::TargetGuard::new(&target, target_identity);

    let mut checked_plan = plan.clone();
    validate_destinations(&target, &mut checked_plan)?;
    let plan = &checked_plan;

    if policy == ConflictPolicy::Abort && !plan.conflicts.is_empty() {
        return Err(ApplyError::Conflicts(plan.conflicts.len()));
    }

    let conflict_paths = plan
        .conflicts
        .iter()
        .map(|conflict| conflict.path.as_path())
        .collect::<HashSet<_>>();
    let blockers = plan.entries.iter().filter(|entry| {
        policy == ConflictPolicy::Skip
            && entry.source != entry.destination
            && conflict_paths.contains(entry.destination.as_path())
    });
    let blocked_directories = blockers
        .clone()
        .filter(|entry| entry.kind == EntryKind::Directory)
        .map(|entry| entry.source.as_path())
        .collect::<HashSet<_>>();
    let blocked_files = blockers
        .filter(|entry| entry.kind == EntryKind::File)
        .map(|entry| entry.destination.as_path())
        .collect::<HashSet<_>>();

    let mut entries = plan
        .entries
        .iter()
        .filter(|entry| {
            !conflict_paths.contains(entry.destination.as_path())
                && !entry
                    .source
                    .ancestors()
                    .skip(1)
                    .any(|ancestor| blocked_directories.contains(ancestor))
                && !entry
                    .destination
                    .ancestors()
                    .any(|ancestor| blocked_files.contains(ancestor))
        })
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        depth(&left.source)
            .cmp(&depth(&right.source))
            .then_with(|| left.source.cmp(&right.source))
    });

    for entry in &entries {
        if entry.source == entry.destination {
            continue;
        }

        let destination = target.join(&entry.destination);
        let parent = path::nearest_existing_directory(&target, &destination)?;
        match renamore::rename_exclusive_is_atomic(parent) {
            Ok(true) => {}
            Ok(false) => {
                return Err(ApplyError::AtomicRenameUnsupported(
                    entry.destination.clone(),
                ));
            }
            Err(source) => {
                return Err(ApplyError::Io {
                    path: destination,
                    source,
                });
            }
        }
    }

    validate_sources(&target, &entries, snapshot)?;
    let mut progress = ApplyProgress {
        report: ApplyReport {
            skipped: if policy == ConflictPolicy::Skip {
                plan.conflicts.clone()
            } else {
                Vec::new()
            },
            ..ApplyReport::default()
        },
        ..ApplyProgress::default()
    };

    let staged_sources = match dependencies::stage_sources(
        &target_guard,
        &entries,
        &snapshot.identities,
        &mut progress.moved_operations,
        recorder,
    ) {
        Ok(staged_sources) => staged_sources,
        Err(operation) => {
            return Err(rollback_after_failure(
                operation,
                &target_guard,
                &progress.moved_operations,
                &progress.created_directories,
                recorder,
            ));
        }
    };

    for (object, entry) in entries.into_iter().enumerate() {
        if let Err(operation) = apply_entry(
            &target_guard,
            entry,
            object as u64,
            snapshot,
            &staged_sources,
            &mut progress,
            recorder,
        ) {
            return Err(rollback_after_failure(
                operation,
                &target_guard,
                &progress.moved_operations,
                &progress.created_directories,
                recorder,
            ));
        }
    }

    Ok(progress.report)
}

#[derive(Default)]
struct ApplyProgress {
    moved_directories: Vec<(PathBuf, PathBuf)>,
    created_directories: Vec<PathBuf>,
    report: ApplyReport,
    moved_operations: Vec<dependencies::MovedOperation>,
}

#[derive(Clone, Copy)]
struct MoveIdentity<'a> {
    object: u64,
    kind: EntryKind,
    expected_source: &'a SourceIdentity,
}

fn rollback_after_failure(
    operation: ApplyError,
    target_guard: &path::TargetGuard<'_>,
    moved: &[(u64, PathBuf, PathBuf, EntryKind, SourceIdentity)],
    created_directories: &[PathBuf],
    recorder: &mut dyn MutationRecorder,
) -> ApplyError {
    let target = target_guard.root();
    let mut failures = Vec::new();

    for (object, source, destination, kind, identity) in moved.iter().rev() {
        let current = target.join(destination);
        let original = target.join(source);
        if let Err(error) = rename_path_exclusive(
            target_guard,
            &current,
            &original,
            path::same_existing_path(&current, &original),
            MoveIdentity {
                object: *object,
                kind: *kind,
                expected_source: identity,
            },
            recorder,
        ) {
            failures.push(format!(
                "{} -> {}: {error}",
                destination.display(),
                source.display()
            ));
        }
    }

    for directory in created_directories.iter().rev() {
        match std::fs::symlink_metadata(directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("{}: {error}", directory.display())),
            Ok(_) => {
                let sequence = match recorder.directory_remove_intent(directory) {
                    Ok(sequence) => sequence,
                    Err(error) => {
                        failures.push(format!("{}: {error}", directory.display()));
                        continue;
                    }
                };
                if let Err(error) = target_guard.ensure_safe(directory.parent().unwrap_or(target)) {
                    failures.push(format!("{}: {error}", directory.display()));
                    continue;
                }
                match std::fs::remove_dir(directory) {
                    Ok(()) => {
                        if let Err(error) = recorder.directory_removed(sequence) {
                            failures.push(format!("{}: {error}", directory.display()));
                        }
                    }
                    Err(error) => failures.push(format!("{}: {error}", directory.display())),
                }
            }
        }
    }

    if failures.is_empty() {
        operation
    } else {
        ApplyError::RollbackFailed {
            operation: Box::new(operation),
            failures: failures.join("; "),
        }
    }
}

fn validate_sources(
    target: &Path,
    entries: &[&PlannedEntry],
    snapshot: &ScanResult,
) -> Result<(), ApplyError> {
    let moved_directories = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Directory && entry.source != entry.destination)
        .map(|entry| entry.source.as_path())
        .collect::<HashSet<_>>();

    for entry in entries {
        path::ensure_relative(&entry.source)?;
        path::ensure_relative(&entry.destination)?;
        let moved_parent = entry
            .source
            .ancestors()
            .skip(1)
            .any(|ancestor| moved_directories.contains(ancestor));
        if entry.source == entry.destination && !moved_parent {
            continue;
        }
        let source = target.join(&entry.source);
        path::ensure_parent_safe(target, source.parent().unwrap_or(target))?;
        let metadata = std::fs::symlink_metadata(&source).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ApplyError::MissingSource(source.clone())
            } else {
                ApplyError::Io {
                    path: source.clone(),
                    source: error,
                }
            }
        })?;
        let source_kind_matches = match entry.kind {
            EntryKind::File => metadata.is_file(),
            EntryKind::Directory => metadata.is_dir(),
            EntryKind::Symlink => false,
        };
        let identity_matches = snapshot
            .identities
            .get(&entry.source)
            .is_some_and(|identity| identity.matches(&metadata, entry.kind, &source));
        if !source_kind_matches || !identity_matches {
            return Err(ApplyError::ChangedSource(source));
        }
    }
    Ok(())
}

fn depth(path: &Path) -> usize {
    path.components().count()
}

fn rename_path_exclusive(
    target_guard: &path::TargetGuard<'_>,
    source: &Path,
    destination: &Path,
    same_path: bool,
    identity: MoveIdentity<'_>,
    recorder: &mut dyn MutationRecorder,
) -> Result<(), ApplyError> {
    if !same_path {
        return recorded_rename(target_guard, source, destination, identity, recorder);
    }

    let temporary = temporary_sibling(source)?;
    recorded_rename(target_guard, source, &temporary, identity, recorder)?;
    match recorded_rename(target_guard, &temporary, destination, identity, recorder) {
        Ok(()) => Ok(()),
        Err(error) => match recorded_rename(target_guard, &temporary, source, identity, recorder) {
            Ok(()) => Err(error),
            Err(rollback) => Err(ApplyError::RollbackFailed {
                operation: Box::new(error),
                failures: format!(
                    "{} -> {}: {rollback}",
                    temporary.display(),
                    source.display()
                ),
            }),
        },
    }
}

fn recorded_rename(
    target_guard: &path::TargetGuard<'_>,
    source: &Path,
    destination: &Path,
    identity: MoveIdentity<'_>,
    recorder: &mut dyn MutationRecorder,
) -> Result<(), ApplyError> {
    let target = target_guard.root();
    let sequence = recorder.move_intent(identity.object, source, destination, identity.kind)?;
    target_guard
        .ensure_source_safe(source, identity.expected_source, identity.kind)
        .map_err(unresolved_operation)?;
    target_guard
        .ensure_safe(destination.parent().unwrap_or(target))
        .map_err(unresolved_operation)?;
    renamore::rename_exclusive(source, destination)
        .map_err(|error| rename_error(source, destination, error))?;
    if let Err(operation) = recorder.move_completed(sequence, destination, identity.kind) {
        let rollback = target_guard
            .ensure_safe(destination.parent().unwrap_or(target))
            .and_then(|()| {
                target_guard.ensure_source_safe(
                    destination,
                    identity.expected_source,
                    identity.kind,
                )
            })
            .and_then(|()| target_guard.ensure_safe(source.parent().unwrap_or(target)))
            .and_then(|()| {
                renamore::rename_exclusive(destination, source)
                    .map_err(|error| rename_error(destination, source, error))
            });
        return match rollback {
            Ok(()) => Err(operation),
            Err(error) => Err(ApplyError::RollbackFailed {
                operation: Box::new(operation),
                failures: format!("{} -> {}: {error}", destination.display(), source.display()),
            }),
        };
    }
    Ok(())
}

fn unresolved_operation(operation: ApplyError) -> ApplyError {
    ApplyError::UnresolvedOperation(Box::new(operation))
}

fn rename_error(source: &Path, destination: &Path, error: std::io::Error) -> ApplyError {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        ApplyError::ExistingDestination(destination.to_path_buf())
    } else {
        ApplyError::Io {
            path: source.to_path_buf(),
            source: error,
        }
    }
}

fn temporary_sibling(source: &Path) -> Result<PathBuf, ApplyError> {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    loop {
        let id = NEXT_TEMPORARY_RENAME.fetch_add(1, Ordering::Relaxed);
        let name = format!(".dirule-tmp-{}-{id}", std::process::id());
        let path = parent.join(name);
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(path),
            Ok(_) => continue,
            Err(source) => {
                return Err(ApplyError::Io { path, source });
            }
        }
    }
}

#[cfg(test)]
mod tests;
