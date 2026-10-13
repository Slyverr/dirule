use super::{ApplyError, MutationRecorder, path, rename_path_exclusive, temporary_sibling};
use dirule_core::{Conflict, EntryKind, Plan, PlannedEntry};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(super) type StagedSources = HashMap<PathBuf, PathBuf>;
pub(super) type MovedOperation = (u64, PathBuf, PathBuf, EntryKind);

pub(super) fn add_dependent_move_conflicts(target: &Path, plan: &mut Plan) {
    if plan.conflicts.is_empty() {
        return;
    }

    let (entries, conflicts) = (&plan.entries, &mut plan.conflicts);
    let mut by_destination = HashMap::<PathBuf, Vec<usize>>::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.source != entry.destination {
            for key in dependency_keys(target, &entry.destination) {
                by_destination.entry(key).or_default().push(index);
            }
        }
    }

    let mut conflict_paths = conflicts
        .iter()
        .map(|conflict| conflict.path.clone())
        .collect::<HashSet<_>>();
    let mut blocked = vec![false; entries.len()];
    let mut queue = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.source != entry.destination && conflict_paths.contains(&entry.destination) {
            blocked[index] = true;
            queue.push(index);
        }
    }

    let mut cursor = 0;
    while cursor < queue.len() {
        let blocked_entry = &entries[queue[cursor]];
        cursor += 1;

        for key in dependency_keys(target, &blocked_entry.source) {
            if let Some(dependents) = by_destination.get(&key) {
                for &dependent in dependents {
                    if blocked[dependent] {
                        continue;
                    }

                    blocked[dependent] = true;
                    queue.push(dependent);
                    let entry = &entries[dependent];
                    if conflict_paths.insert(entry.destination.clone()) {
                        conflicts.push(Conflict {
                            path: entry.destination.clone(),
                            reason: "depends on a move skipped because of a conflict".into(),
                        });
                    }
                }
            }
        }
    }
}

fn dependency_keys(target: &Path, path: &Path) -> Vec<PathBuf> {
    let mut keys = vec![path.to_path_buf()];
    if let Ok(canonical) = std::fs::canonicalize(target.join(path)) {
        keys.push(canonical);
    }

    keys
}

pub(super) fn stage_sources(
    target: &Path,
    entries: &[&PlannedEntry],
    moved_operations: &mut Vec<MovedOperation>,
    recorder: &mut dyn MutationRecorder,
) -> Result<StagedSources, ApplyError> {
    let mut reserved_paths = entries
        .iter()
        .flat_map(|entry| [entry.source.clone(), entry.destination.clone()])
        .collect::<HashSet<_>>();
    let mut destination_owners = HashMap::<PathBuf, Vec<usize>>::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.source == entry.destination {
            continue;
        }

        destination_owners
            .entry(entry.destination.clone())
            .or_default()
            .push(index);

        let destination = target.join(&entry.destination);
        match std::fs::symlink_metadata(&destination) {
            Ok(metadata) if !metadata.file_type().is_symlink() => {
                match std::fs::canonicalize(&destination) {
                    Ok(canonical) => {
                        destination_owners.entry(canonical).or_default().push(index);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => {
                        return Err(ApplyError::Io {
                            path: destination,
                            source,
                        });
                    }
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ApplyError::Io {
                    path: destination,
                    source,
                });
            }
        }
    }
    let candidates = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            if entry.source == entry.destination {
                return None;
            }

            let source = target.join(&entry.source);
            let canonical = match std::fs::canonicalize(&source) {
                Ok(canonical) => canonical,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Some(Err(ApplyError::MissingSource(entry.source.clone())));
                }
                Err(source_error) => {
                    return Some(Err(ApplyError::Io {
                        path: source,
                        source: source_error,
                    }));
                }
            };
            let is_dependency = destination_owners
                .get(&entry.source)
                .into_iter()
                .chain(destination_owners.get(&canonical))
                .flatten()
                .any(|owner| *owner != index);
            is_dependency.then_some(Ok((index, entry)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let candidate_directories = candidates
        .iter()
        .filter(|(_, entry)| entry.kind == EntryKind::Directory)
        .map(|(_, entry)| entry.source.as_path())
        .collect::<HashSet<_>>();
    let staging = candidates
        .iter()
        .filter(|(_, entry)| {
            !entry
                .source
                .ancestors()
                .skip(1)
                .any(|ancestor| candidate_directories.contains(ancestor))
        })
        .collect::<Vec<_>>();

    for (_, entry) in &staging {
        let source = target.join(&entry.source);
        let parent = source.parent().unwrap_or(target);
        match renamore::rename_exclusive_is_atomic(parent) {
            Ok(true) => {}
            Ok(false) => {
                return Err(ApplyError::AtomicRenameUnsupported(entry.source.clone()));
            }
            Err(source) => {
                return Err(ApplyError::Io {
                    path: parent.to_path_buf(),
                    source,
                });
            }
        }
    }

    let mut staged_sources = HashMap::with_capacity(staging.len());
    for (object, entry) in staging {
        let source = target.join(&entry.source);
        let (temporary, temporary_relative) = loop {
            let temporary = temporary_sibling(&source)?;
            let temporary_relative = temporary
                .strip_prefix(target)
                .map_err(|_| ApplyError::UnsafePath(temporary.clone()))?
                .to_path_buf();
            if reserved_paths.insert(temporary_relative.clone()) {
                break (temporary, temporary_relative);
            }
        };
        path::ensure_relative(&temporary_relative)?;
        rename_path_exclusive(
            &source,
            &temporary,
            false,
            *object as u64,
            entry.kind,
            recorder,
        )?;
        staged_sources.insert(entry.source.clone(), temporary_relative.clone());
        moved_operations.push((
            *object as u64,
            entry.source.clone(),
            temporary_relative,
            entry.kind,
        ));
    }

    Ok(staged_sources)
}
