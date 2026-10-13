use crate::ScanResult;
use crate::journal::{JournalRecorder, JournalStatus, JournalStore};
use dirule_core::{Conflict, EntryKind, Plan, PlannedEntry};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

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
    for entry in &plan.entries {
        ensure_relative(&entry.source)?;
        ensure_relative(&entry.destination)?;
        if entry.source == entry.destination {
            continue;
        }
        let destination = target.join(&entry.destination);
        match ensure_parent_safe(&target, destination.parent().unwrap_or(&target)) {
            Ok(()) => {}
            Err(ApplyError::ExistingDestination(path)) if will_be_vacated(&target, &path, plan) => {
            }
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
        if destination_exists(&destination)?
            && !will_be_vacated(&target, &destination, plan)
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
            recorder
                .finish(JournalStatus::Applied)
                .map_err(|source| ApplyError::RunFailed {
                    run_id: run_id.clone(),
                    source: Box::new(ApplyError::Journal(Box::new(source))),
                })?;
            Ok(JournaledApplyReport {
                report,
                run_id: Some(run_id),
            })
        }
        Err(source) => {
            let status = if matches!(source, ApplyError::RollbackFailed { .. }) {
                JournalStatus::PartiallyRecovered
            } else {
                JournalStatus::FailedRolledBack
            };
            let source = match recorder.finish(status) {
                Ok(()) => source,
                Err(journal_error) => ApplyError::Journal(Box::new(journal_error)),
            };
            Err(ApplyError::RunFailed {
                run_id,
                source: Box::new(source),
            })
        }
    }
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
    if !target_metadata.is_dir()
        || !snapshot.target_identity.as_ref().is_some_and(|identity| {
            identity.matches_object(&target_metadata, EntryKind::Directory, &target)
        })
    {
        return Err(ApplyError::ChangedTarget(target));
    }

    let mut checked_plan = plan.clone();
    validate_destinations(&target, &mut checked_plan)?;
    let plan = &checked_plan;

    if policy == ConflictPolicy::Abort && !plan.conflicts.is_empty() {
        return Err(ApplyError::Conflicts(plan.conflicts.len()));
    }

    let blockers = if policy == ConflictPolicy::Skip {
        plan.entries
            .iter()
            .filter(|entry| {
                entry.source != entry.destination
                    && plan
                        .conflicts
                        .iter()
                        .any(|conflict| conflict.path == entry.destination)
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let mut entries = plan
        .entries
        .iter()
        .filter(|entry| {
            !plan
                .conflicts
                .iter()
                .any(|conflict| entry.destination == conflict.path)
                && !blockers.iter().any(|blocker| match blocker.kind {
                    EntryKind::Directory => {
                        entry.source != blocker.source && entry.source.starts_with(&blocker.source)
                    }
                    EntryKind::File => entry.destination.starts_with(&blocker.destination),
                    EntryKind::Symlink => false,
                })
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
        let parent = nearest_existing_directory(&target, &destination)?;
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
    for (object, entry) in entries.into_iter().enumerate() {
        if let Err(operation) = apply_entry(
            &target,
            entry,
            object as u64,
            snapshot,
            &mut progress,
            recorder,
        ) {
            return Err(rollback_after_failure(
                operation,
                &target,
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
    moved_operations: Vec<(u64, PathBuf, PathBuf, EntryKind)>,
}

fn apply_entry(
    target: &Path,
    entry: &PlannedEntry,
    object: u64,
    snapshot: &ScanResult,
    progress: &mut ApplyProgress,
    recorder: &mut dyn MutationRecorder,
) -> Result<(), ApplyError> {
    ensure_relative(&entry.source)?;
    ensure_relative(&entry.destination)?;
    if inherited_move(entry, &progress.moved_directories) {
        return Ok(());
    }

    let source_relative = remap(&entry.source, &progress.moved_directories);
    if source_relative == entry.destination {
        return Ok(());
    }

    let source = target.join(&source_relative);
    let destination = target.join(&entry.destination);
    ensure_parent_safe(target, source.parent().unwrap_or(target))?;
    ensure_parent_safe(target, destination.parent().unwrap_or(target))?;
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

    let destination_exists = destination_exists(&destination)?;
    let destination_is_source = destination_exists && same_existing_path(&source, &destination);
    if destination_exists && !destination_is_source {
        return Err(ApplyError::ExistingDestination(destination));
    }
    if !destination_is_source && let Some(parent) = destination.parent() {
        create_parent_directories(target, parent, &mut progress.created_directories, recorder)?;
    }

    rename_path_exclusive(
        &source,
        &destination,
        destination_is_source,
        object,
        entry.kind,
        recorder,
    )?;
    if entry.kind == EntryKind::Directory {
        progress
            .moved_directories
            .push((entry.source.clone(), entry.destination.clone()));
    }
    progress
        .report
        .moved
        .push((source_relative.clone(), entry.destination.clone()));
    progress.moved_operations.push((
        object,
        source_relative,
        entry.destination.clone(),
        entry.kind,
    ));

    Ok(())
}

fn create_parent_directories(
    root: &Path,
    parent: &Path,
    created_directories: &mut Vec<PathBuf>,
    recorder: &mut dyn MutationRecorder,
) -> Result<(), ApplyError> {
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| ApplyError::UnsafePath(parent.to_path_buf()))?;
    let mut current = root.to_path_buf();

    for component in relative.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(ApplyError::UnsafePath(current)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let sequence = recorder.directory_intent(&current)?;
                match std::fs::create_dir(&current) {
                    Ok(()) => {
                        created_directories.push(current.clone());
                        recorder.directory_created(sequence, &current)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        ensure_parent_safe(root, &current)?;
                    }
                    Err(source) => {
                        return Err(ApplyError::Io {
                            path: current,
                            source,
                        });
                    }
                }
            }
            Err(source) => {
                return Err(ApplyError::Io {
                    path: current,
                    source,
                });
            }
        }
    }

    Ok(())
}

fn rollback_after_failure(
    operation: ApplyError,
    target: &Path,
    moved: &[(u64, PathBuf, PathBuf, EntryKind)],
    created_directories: &[PathBuf],
    recorder: &mut dyn MutationRecorder,
) -> ApplyError {
    let mut failures = Vec::new();

    for (object, source, destination, kind) in moved.iter().rev() {
        let current = target.join(destination);
        let original = target.join(source);
        if let Err(error) = rename_path_exclusive(
            &current,
            &original,
            same_existing_path(&current, &original),
            *object,
            *kind,
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
    for entry in entries {
        ensure_relative(&entry.source)?;
        ensure_relative(&entry.destination)?;
        let moved_parent = entries.iter().any(|parent| {
            parent.kind == EntryKind::Directory
                && parent.source != parent.destination
                && entry.source != parent.source
                && entry.source.starts_with(&parent.source)
        });
        if entry.source == entry.destination && !moved_parent {
            continue;
        }
        let source = target.join(&entry.source);
        ensure_parent_safe(target, source.parent().unwrap_or(target))?;
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

fn inherited_move(entry: &PlannedEntry, moved_directories: &[(PathBuf, PathBuf)]) -> bool {
    moved_directories.iter().any(|(source, destination)| {
        entry
            .source
            .strip_prefix(source)
            .ok()
            .is_some_and(|suffix| destination.join(suffix) == entry.destination)
    })
}

fn remap(path: &Path, moves: &[(PathBuf, PathBuf)]) -> PathBuf {
    moves
        .iter()
        .filter_map(|(source, destination)| {
            path.strip_prefix(source)
                .ok()
                .map(|suffix| (depth(source), destination, suffix))
        })
        .max_by_key(|(source_depth, _, _)| *source_depth)
        .map_or_else(
            || path.to_path_buf(),
            |(_, destination, suffix)| destination.join(suffix),
        )
}

fn depth(path: &Path) -> usize {
    path.components().count()
}

fn ensure_relative(path: &Path) -> Result<(), ApplyError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ApplyError::UnsafePath(path.to_path_buf()));
    }

    #[cfg(windows)]
    if path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_none_or(invalid_windows_component)
    }) {
        return Err(ApplyError::UnsafePath(path.to_path_buf()));
    }

    Ok(())
}

#[cfg(windows)]
fn invalid_windows_component(value: &str) -> bool {
    if value.ends_with(' ') || value.ends_with('.') {
        return true;
    }

    if value.chars().any(|character| {
        character <= '\u{001f}'
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    }) {
        return true;
    }

    let device_name = value
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }

    ["COM", "LPT"].iter().any(|prefix| {
        device_name
            .strip_prefix(prefix)
            .and_then(|suffix| {
                let mut characters = suffix.chars();
                let digit = characters.next()?;
                (characters.next().is_none() && matches!(digit, '1'..='9' | '¹' | '²' | '³'))
                    .then_some(())
            })
            .is_some()
    })
}

fn ensure_parent_safe(root: &Path, path: &Path) -> Result<(), ApplyError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| ApplyError::UnsafePath(path.to_path_buf()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ApplyError::UnsafePath(current));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(ApplyError::ExistingDestination(current));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(ApplyError::Io {
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn destination_exists(path: &Path) -> Result<bool, ApplyError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ApplyError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn nearest_existing_directory(root: &Path, destination: &Path) -> Result<PathBuf, ApplyError> {
    let mut current = destination.parent().unwrap_or(root).to_path_buf();
    loop {
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                return Ok(current);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ApplyError::Io {
                    path: current,
                    source,
                });
            }
        }
        if current == root || !current.pop() {
            return Err(ApplyError::UnsafePath(destination.to_path_buf()));
        }
    }
}

fn will_be_vacated(target: &Path, path: &Path, plan: &Plan) -> bool {
    let Ok(relative) = path.strip_prefix(target) else {
        return false;
    };

    plan.entries.iter().any(|entry| {
        entry.source != entry.destination
            && (entry.source == relative
                || (entry.kind == EntryKind::Directory && relative.starts_with(&entry.source))
                || same_existing_path(&target.join(&entry.source), path))
    })
}

fn same_existing_path(left: &Path, right: &Path) -> bool {
    let Ok(left_metadata) = std::fs::symlink_metadata(left) else {
        return false;
    };
    let Ok(right_metadata) = std::fs::symlink_metadata(right) else {
        return false;
    };
    if left_metadata.file_type().is_symlink() || right_metadata.file_type().is_symlink() {
        return false;
    }

    matches!(
        (std::fs::canonicalize(left), std::fs::canonicalize(right)),
        (Ok(left), Ok(right)) if left == right
    )
}

fn rename_path_exclusive(
    source: &Path,
    destination: &Path,
    same_path: bool,
    object: u64,
    kind: EntryKind,
    recorder: &mut dyn MutationRecorder,
) -> Result<(), ApplyError> {
    if !same_path {
        return recorded_rename(source, destination, object, kind, recorder);
    }

    let temporary = temporary_sibling(source)?;
    recorded_rename(source, &temporary, object, kind, recorder)?;
    match recorded_rename(&temporary, destination, object, kind, recorder) {
        Ok(()) => Ok(()),
        Err(error) => match recorded_rename(&temporary, source, object, kind, recorder) {
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
    source: &Path,
    destination: &Path,
    object: u64,
    kind: EntryKind,
    recorder: &mut dyn MutationRecorder,
) -> Result<(), ApplyError> {
    let sequence = recorder.move_intent(object, source, destination, kind)?;
    renamore::rename_exclusive(source, destination)
        .map_err(|error| rename_error(source, destination, error))?;
    if let Err(operation) = recorder.move_completed(sequence, destination, kind) {
        return match renamore::rename_exclusive(destination, source) {
            Ok(()) => Err(operation),
            Err(error) => Err(ApplyError::RollbackFailed {
                operation: Box::new(operation),
                failures: format!("{} -> {}: {error}", destination.display(), source.display()),
            }),
        };
    }
    Ok(())
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
