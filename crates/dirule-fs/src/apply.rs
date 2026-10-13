use crate::ScanResult;
use dirule_core::{Conflict, EntryKind, Plan, PlannedEntry};
use std::path::Component;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApplyReport {
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub skipped: Vec<Conflict>,
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
    #[error("planned path is not a safe target-relative path: {0}")]
    UnsafePath(PathBuf),
    #[error("filesystem does not support atomic no-replace renames at: {0}")]
    AtomicRenameUnsupported(PathBuf),
    #[error("source type changed since planning: {0}")]
    ChangedSource(PathBuf),
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
    let mut checked_plan = plan.clone();
    validate_destinations(target, &mut checked_plan)?;
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

    let target = target.canonicalize().map_err(|source| ApplyError::Io {
        path: target.to_path_buf(),
        source,
    })?;
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
    let mut moved_directories = Vec::<(PathBuf, PathBuf)>::new();
    let mut report = ApplyReport {
        skipped: if policy == ConflictPolicy::Skip {
            plan.conflicts.clone()
        } else {
            Vec::new()
        },
        ..ApplyReport::default()
    };
    let mut created_directories = Vec::new();

    for entry in entries {
        if let Err(operation) = apply_entry(
            &target,
            entry,
            snapshot,
            &mut moved_directories,
            &mut created_directories,
            &mut report,
        ) {
            return Err(rollback_after_failure(
                operation,
                &target,
                &report.moved,
                &created_directories,
            ));
        }
    }

    Ok(report)
}

fn apply_entry(
    target: &Path,
    entry: &PlannedEntry,
    snapshot: &ScanResult,
    moved_directories: &mut Vec<(PathBuf, PathBuf)>,
    created_directories: &mut Vec<PathBuf>,
    report: &mut ApplyReport,
) -> Result<(), ApplyError> {
    ensure_relative(&entry.source)?;
    ensure_relative(&entry.destination)?;
    if inherited_move(entry, moved_directories) {
        return Ok(());
    }

    let source_relative = remap(&entry.source, moved_directories);
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
        .is_some_and(|identity| identity.matches(&metadata, entry.kind));
    if !source_kind_matches || !identity_matches {
        return Err(ApplyError::ChangedSource(source));
    }

    if destination_exists(&destination)? {
        return Err(ApplyError::ExistingDestination(destination));
    }
    if let Some(parent) = destination.parent() {
        create_parent_directories(target, parent, created_directories)?;
    }

    renamore::rename_exclusive(&source, &destination).map_err(|source_error| {
        if source_error.kind() == std::io::ErrorKind::AlreadyExists {
            ApplyError::ExistingDestination(destination.clone())
        } else {
            ApplyError::Io {
                path: source.clone(),
                source: source_error,
            }
        }
    })?;
    if entry.kind == EntryKind::Directory {
        moved_directories.push((entry.source.clone(), entry.destination.clone()));
    }
    report
        .moved
        .push((source_relative, entry.destination.clone()));

    Ok(())
}

fn create_parent_directories(
    root: &Path,
    parent: &Path,
    created_directories: &mut Vec<PathBuf>,
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
                match std::fs::create_dir(&current) {
                    Ok(()) => created_directories.push(current.clone()),
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
    moved: &[(PathBuf, PathBuf)],
    created_directories: &[PathBuf],
) -> ApplyError {
    let mut failures = Vec::new();

    for (source, destination) in moved.iter().rev() {
        let current = target.join(destination);
        let original = target.join(source);
        if let Err(error) = renamore::rename_exclusive(&current, &original) {
            failures.push(format!(
                "{} -> {}: {error}",
                destination.display(),
                source.display()
            ));
        }
    }

    for directory in created_directories.iter().rev() {
        match std::fs::remove_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("{}: {error}", directory.display())),
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
            .is_some_and(|identity| identity.matches(&metadata, entry.kind));
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
    Ok(())
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
                || (entry.kind == EntryKind::Directory && relative.starts_with(&entry.source)))
    })
}
