use super::{
    ApplyError, ApplyProgress, MoveIdentity, MutationRecorder, dependencies, depth, path,
    rename_path_exclusive, unresolved_operation,
};
use crate::ScanResult;
use dirule_core::{EntryKind, PlannedEntry};
use std::path::{Path, PathBuf};

pub(super) fn apply_entry(
    target_guard: &path::TargetGuard<'_>,
    entry: &PlannedEntry,
    object: u64,
    snapshot: &ScanResult,
    staged_sources: &dependencies::StagedSources,
    progress: &mut ApplyProgress,
    recorder: &mut dyn MutationRecorder,
) -> Result<(), ApplyError> {
    let target = target_guard.root();
    path::ensure_relative(&entry.source)?;
    path::ensure_relative(&entry.destination)?;
    if inherited_move(entry, &progress.moved_directories) {
        return Ok(());
    }

    let source_relative = staged_sources
        .get(&entry.source)
        .cloned()
        .unwrap_or_else(|| remap(&entry.source, &progress.moved_directories));
    if source_relative == entry.destination {
        return Ok(());
    }

    let source = target.join(&source_relative);
    let destination = target.join(&entry.destination);
    let source_identity = snapshot
        .identities
        .get(&entry.source)
        .ok_or_else(|| ApplyError::ChangedSource(source.clone()))?;
    path::ensure_parent_safe(target, source.parent().unwrap_or(target))?;
    path::ensure_parent_safe(target, destination.parent().unwrap_or(target))?;
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

    let destination_exists = path::destination_exists(&destination)?;
    let destination_is_source =
        destination_exists && path::same_existing_path(&source, &destination);
    if destination_exists && !destination_is_source {
        return Err(ApplyError::ExistingDestination(destination));
    }
    if !destination_is_source && let Some(parent) = destination.parent() {
        create_parent_directories(
            target_guard,
            parent,
            &mut progress.created_directories,
            recorder,
        )?;
    }

    rename_path_exclusive(
        target_guard,
        &source,
        &destination,
        destination_is_source,
        MoveIdentity {
            object,
            kind: entry.kind,
            expected_source: source_identity,
        },
        recorder,
    )?;
    if entry.kind == EntryKind::Directory {
        progress
            .moved_directories
            .push((entry.source.clone(), entry.destination.clone()));
    }
    let report_source = if staged_sources.contains_key(&entry.source) {
        entry.source.clone()
    } else {
        source_relative.clone()
    };
    progress
        .report
        .moved
        .push((report_source, entry.destination.clone()));
    progress.moved_operations.push((
        object,
        source_relative,
        entry.destination.clone(),
        entry.kind,
        source_identity.clone(),
    ));

    Ok(())
}

fn create_parent_directories(
    target_guard: &path::TargetGuard<'_>,
    parent: &Path,
    created_directories: &mut Vec<PathBuf>,
    recorder: &mut dyn MutationRecorder,
) -> Result<(), ApplyError> {
    let root = target_guard.root();
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
                target_guard
                    .ensure_safe(current.parent().unwrap_or(root))
                    .map_err(unresolved_operation)?;
                match std::fs::create_dir(&current) {
                    Ok(()) => {
                        created_directories.push(current.clone());
                        target_guard
                            .ensure_safe(&current)
                            .map_err(unresolved_operation)?;
                        recorder.directory_created(sequence, &current)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        return Err(unresolved_operation(ApplyError::UnsafePath(current)));
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
