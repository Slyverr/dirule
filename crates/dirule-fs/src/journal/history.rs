use super::identity::ObjectIdentity;
use super::{
    CURRENT_JOURNAL_VERSION, DirectoryAction, JournalError, JournalRecord, JournalStatus,
    JournalStatusRecord,
};
use dirule_core::EntryKind;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

enum IntentKind<'a> {
    Move {
        identity: &'a ObjectIdentity,
    },
    Directory {
        action: DirectoryAction,
        path: &'a Path,
        identity: Option<&'a ObjectIdentity>,
    },
    TextEdit,
}

struct Intent<'a> {
    kind: IntentKind<'a>,
    completed: bool,
}

pub(super) struct JournalHistory {
    pub(super) id: String,
    pub(super) target: PathBuf,
    pub(super) target_identity: ObjectIdentity,
    pub(super) started_at: u64,
    pub(super) status: JournalStatus,
    pub(super) records: Vec<JournalRecord>,
}

pub(super) enum UndoMutation {
    Move {
        object: u64,
        source: PathBuf,
        destination: PathBuf,
        identity: ObjectIdentity,
    },
    Directory {
        path: PathBuf,
        action: DirectoryAction,
        identity: ObjectIdentity,
    },
    TextEdit {
        sequence: u64,
        path: PathBuf,
        before: String,
        after: String,
    },
}

impl JournalHistory {
    pub(super) fn from_records(
        path: &Path,
        records: Vec<JournalRecord>,
    ) -> Result<Self, JournalError> {
        let Some(JournalRecord::Header {
            version,
            id,
            target,
            target_identity,
            started_at,
            ..
        }) = records.first()
        else {
            return Err(JournalError::InvalidRunId(path.display().to_string()));
        };
        if version != CURRENT_JOURNAL_VERSION {
            return Err(JournalError::UnsupportedVersion {
                path: path.to_path_buf(),
                version: version.clone(),
            });
        }

        validate_events(path, &records[1..])?;

        let mut status = JournalStatus::Applying;
        for record in &records[1..] {
            if let JournalRecord::Status { status: value } = record {
                status = (*value).into();
            }
        }
        if status == JournalStatus::Applying {
            status = JournalStatus::Interrupted;
        }
        Ok(Self {
            id: id.clone(),
            target: target.to_path()?,
            target_identity: target_identity.clone(),
            started_at: *started_at,
            status,
            records,
        })
    }

    pub(super) fn validate_undoable(&self, requested_id: &str) -> Result<(), JournalError> {
        if self.id != requested_id {
            return Err(JournalError::InvalidRunId(requested_id.to_owned()));
        }
        match self.status {
            JournalStatus::Applied
            | JournalStatus::Interrupted
            | JournalStatus::PartiallyRecovered
            | JournalStatus::Undoing => Ok(()),
            JournalStatus::Undone => Err(JournalError::InvalidRunId(format!(
                "{requested_id} is already undone"
            ))),
            status => Err(JournalError::NotUndoable(
                requested_id.to_owned(),
                format!("run status is {status:?}"),
            )),
        }
    }

    pub(super) fn mutations(&self) -> Result<Vec<UndoMutation>, JournalError> {
        let mut intents = std::collections::BTreeMap::new();
        let mut moves = std::collections::BTreeMap::new();
        let mut directories = std::collections::BTreeMap::new();
        let mut directory_changes = std::collections::BTreeMap::new();
        let mut text_edits = std::collections::BTreeMap::new();
        let mut completed_text_edits = BTreeMap::<u64, Option<ObjectIdentity>>::new();
        let mut undone_text_edits = BTreeMap::<u64, Option<ObjectIdentity>>::new();
        let mut retained_directories = std::collections::BTreeSet::new();
        let mut max_sequence = None;
        for record in &self.records[1..] {
            match record {
                JournalRecord::MoveIntent {
                    sequence,
                    object,
                    source,
                    destination,
                    identity,
                } => {
                    max_sequence = Some(*sequence);
                    intents.insert(
                        *sequence,
                        UndoMutation::Move {
                            object: *object,
                            source: source.clone(),
                            destination: destination.clone(),
                            identity: identity.clone(),
                        },
                    );
                }
                JournalRecord::MoveCompleted { sequence, identity } => {
                    moves.insert(*sequence, identity.clone());
                }
                JournalRecord::DirectoryIntent {
                    sequence,
                    path,
                    action,
                    identity,
                } => {
                    max_sequence = Some(*sequence);
                    if let Some(identity) = identity {
                        directories.insert(
                            *sequence,
                            UndoMutation::Directory {
                                path: path.clone(),
                                action: *action,
                                identity: identity.clone(),
                            },
                        );
                    } else {
                        directories.insert(
                            *sequence,
                            UndoMutation::Directory {
                                path: path.clone(),
                                action: *action,
                                identity: ObjectIdentity::from_kind(EntryKind::Directory),
                            },
                        );
                    }
                }
                JournalRecord::DirectoryChanged {
                    sequence,
                    action,
                    identity,
                } => {
                    directory_changes.insert(*sequence, (*action, identity.clone()));
                }
                JournalRecord::DirectoryRetained { path, .. } => {
                    retained_directories.insert(path.clone());
                }
                JournalRecord::TextEditIntent {
                    sequence,
                    path,
                    before,
                    after,
                } => {
                    max_sequence = Some(*sequence);
                    text_edits.insert(
                        *sequence,
                        UndoMutation::TextEdit {
                            sequence: *sequence,
                            path: path.clone(),
                            before: before.clone(),
                            after: after.clone(),
                        },
                    );
                }
                JournalRecord::TextEditCompleted { sequence, identity } => {
                    completed_text_edits.insert(*sequence, identity.clone());
                }
                JournalRecord::TextEditUndone { sequence, identity } => {
                    undone_text_edits.insert(*sequence, identity.clone());
                }
                JournalRecord::Header { .. } | JournalRecord::Status { .. } => {}
            }
        }

        let mut mutations = Vec::new();
        let mut sequences = intents
            .keys()
            .chain(directories.keys())
            .chain(text_edits.keys())
            .copied()
            .collect::<Vec<_>>();
        sequences.sort_unstable();

        let mut text_edit_identities = HashMap::<PathBuf, ObjectIdentity>::new();
        for (sequence, mutation) in &text_edits {
            let UndoMutation::TextEdit { path, .. } = mutation else {
                unreachable!()
            };
            if let Some(Some(identity)) = undone_text_edits.get(sequence) {
                text_edit_identities.insert(path.clone(), identity.clone());
            } else if let Some(Some(identity)) = completed_text_edits.get(sequence) {
                text_edit_identities.insert(path.clone(), identity.clone());
            }
        }

        for sequence in sequences {
            if let Some(mutation) = text_edits.remove(&sequence) {
                if undone_text_edits.contains_key(&sequence) {
                    continue;
                }
                let UndoMutation::TextEdit {
                    sequence,
                    path,
                    before,
                    after,
                } = mutation
                else {
                    unreachable!()
                };
                let absolute = checked_target_path(&self.target, &path)?;
                match fs::read_to_string(&absolute) {
                    Ok(contents) if contents == after => {
                        if let Some(Some(identity)) = completed_text_edits.get(&sequence) {
                            let metadata = fs::symlink_metadata(&absolute)
                                .map_err(|source| JournalError::io(&absolute, source))?;
                            if !identity.matches(&metadata, EntryKind::File, &absolute) {
                                return Err(JournalError::ChangedObject {
                                    run_id: self.id.clone(),
                                    path: absolute,
                                });
                            }
                        }
                        mutations.push(UndoMutation::TextEdit {
                            sequence,
                            path,
                            before,
                            after,
                        });
                    }
                    Ok(contents) if contents == before => {
                        if completed_text_edits.contains_key(&sequence)
                            && self.status == JournalStatus::Undoing
                        {
                            mutations.push(UndoMutation::TextEdit {
                                sequence,
                                path,
                                before,
                                after,
                            });
                        }
                    }
                    Ok(_) => {
                        return Err(JournalError::ChangedObject {
                            run_id: self.id.clone(),
                            path: absolute,
                        });
                    }
                    Err(source) => return Err(JournalError::io(absolute, source)),
                }
                continue;
            }
            if let Some(mutation) = intents.remove(&sequence) {
                let identity = moves.get(&sequence);
                match (mutation, identity) {
                    (
                        UndoMutation::Move {
                            object,
                            source,
                            destination,
                            identity: original,
                        },
                        Some(completed),
                    ) => {
                        if original != *completed {
                            return Err(JournalError::AmbiguousOperation {
                                run_id: self.id.clone(),
                                path: destination,
                            });
                        }
                        let identity = text_edit_identities
                            .get(&destination)
                            .cloned()
                            .unwrap_or(original);
                        mutations.push(UndoMutation::Move {
                            object,
                            source,
                            destination,
                            identity,
                        });
                    }
                    (
                        UndoMutation::Move {
                            object,
                            source,
                            destination,
                            identity,
                        },
                        None,
                    ) => {
                        let identity = text_edit_identities
                            .get(&destination)
                            .cloned()
                            .unwrap_or(identity);
                        if max_sequence == Some(sequence)
                            && operation_reached_destination(
                                &self.target,
                                &source,
                                &destination,
                                &identity,
                            )?
                        {
                            mutations.push(UndoMutation::Move {
                                object,
                                source,
                                destination,
                                identity,
                            });
                        } else if max_sequence != Some(sequence)
                            || !operation_still_at_source(
                                &self.target,
                                &source,
                                &destination,
                                &identity,
                            )?
                        {
                            return Err(JournalError::AmbiguousOperation {
                                run_id: self.id.clone(),
                                path: destination,
                            });
                        }
                    }
                    _ => unreachable!(),
                }
                continue;
            }

            if let Some(mutation) = directories.remove(&sequence) {
                let completed = directory_changes.get(&sequence);
                match (mutation, completed) {
                    (
                        UndoMutation::Directory {
                            path,
                            action,
                            identity,
                        },
                        Some((found_action, found_identity)),
                    ) => {
                        if action == DirectoryAction::Create && retained_directories.contains(&path)
                        {
                            continue;
                        }
                        if action != *found_action {
                            return Err(JournalError::AmbiguousOperation {
                                run_id: self.id.clone(),
                                path,
                            });
                        }
                        let identity = if action == DirectoryAction::Create {
                            found_identity.clone()
                        } else {
                            identity
                        };
                        mutations.push(UndoMutation::Directory {
                            path,
                            action,
                            identity,
                        });
                    }
                    (
                        UndoMutation::Directory {
                            path,
                            action: DirectoryAction::Remove,
                            identity,
                        },
                        None,
                    ) => {
                        let absolute = checked_target_path(&self.target, &path)?;
                        match fs::symlink_metadata(&absolute) {
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => mutations
                                .push(UndoMutation::Directory {
                                    path,
                                    action: DirectoryAction::Remove,
                                    identity,
                                }),
                            Ok(metadata)
                                if identity.matches(&metadata, EntryKind::Directory, &absolute) => {
                            }
                            _ => {
                                return Err(JournalError::AmbiguousOperation {
                                    run_id: self.id.clone(),
                                    path,
                                });
                            }
                        }
                    }
                    (
                        UndoMutation::Directory {
                            path,
                            action: DirectoryAction::Create,
                            ..
                        },
                        None,
                    ) => {
                        let absolute = checked_target_path(&self.target, &path)?;
                        if path_exists(&absolute)? {
                            return Err(JournalError::AmbiguousOperation {
                                run_id: self.id.clone(),
                                path,
                            });
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(cancel_inverse_mutations(mutations))
    }
}

fn validate_events(path: &Path, records: &[JournalRecord]) -> Result<(), JournalError> {
    let mut intents = BTreeMap::new();
    let mut last_sequence = None;
    let mut status = None;
    let mut mutations_allowed = true;

    for record in records {
        match record {
            JournalRecord::Status { status: next } => {
                validate_status_transition(path, status, *next)?;
                status = Some(*next);
                mutations_allowed = matches!(
                    next,
                    JournalStatusRecord::Applying | JournalStatusRecord::Undoing
                );
            }
            JournalRecord::Header { .. } => {
                return Err(invalid_events(path, "unexpected header record".into()));
            }
            _ if !mutations_allowed => {
                return Err(invalid_events(
                    path,
                    format!("mutation follows terminal status record {status:?}"),
                ));
            }
            JournalRecord::MoveIntent {
                sequence, identity, ..
            } => {
                insert_intent(
                    path,
                    &mut intents,
                    &mut last_sequence,
                    *sequence,
                    IntentKind::Move { identity },
                )?;
            }
            JournalRecord::DirectoryIntent {
                sequence,
                path: directory,
                action,
                identity,
            } => {
                insert_intent(
                    path,
                    &mut intents,
                    &mut last_sequence,
                    *sequence,
                    IntentKind::Directory {
                        action: *action,
                        path: directory,
                        identity: identity.as_ref(),
                    },
                )?;
            }
            JournalRecord::TextEditIntent { sequence, .. } => {
                insert_intent(
                    path,
                    &mut intents,
                    &mut last_sequence,
                    *sequence,
                    IntentKind::TextEdit,
                )?;
            }
            JournalRecord::MoveCompleted { sequence, identity } => {
                match find_intent(path, &mut intents, *sequence)?.kind {
                    IntentKind::Move { identity: expected } if expected == identity => {}
                    _ => {
                        return Err(invalid_events(
                            path,
                            format!("sequence {sequence} has a mismatched move completion"),
                        ));
                    }
                }
                complete_intent(path, &mut intents, *sequence, CompletionKind::Move)?;
            }
            JournalRecord::DirectoryChanged {
                sequence,
                action,
                identity,
            } => {
                let intent = find_intent(path, &mut intents, *sequence)?;
                match intent.kind {
                    IntentKind::Directory {
                        action: expected,
                        identity: Some(expected_identity),
                        ..
                    } if expected == *action && expected_identity == identity => {}
                    IntentKind::Directory {
                        action: expected,
                        identity: None,
                        ..
                    } if expected == *action && *action == DirectoryAction::Create => {}
                    _ => {
                        return Err(invalid_events(
                            path,
                            format!("sequence {sequence} has a mismatched directory completion"),
                        ));
                    }
                }
                complete_intent(path, &mut intents, *sequence, CompletionKind::Directory)?;
            }
            JournalRecord::TextEditCompleted { sequence, .. } => {
                complete_intent(path, &mut intents, *sequence, CompletionKind::TextEdit)?;
            }
            JournalRecord::TextEditUndone { sequence, .. } => {
                if status != Some(JournalStatusRecord::Undoing) {
                    return Err(invalid_events(
                        path,
                        format!("text-edit undo marker for sequence {sequence} is outside undo"),
                    ));
                }
                let intent = find_intent(path, &mut intents, *sequence)?;
                if !matches!(intent.kind, IntentKind::TextEdit) || !intent.completed {
                    return Err(invalid_events(
                        path,
                        format!("sequence {sequence} has an invalid text-edit undo marker"),
                    ));
                }
            }
            JournalRecord::DirectoryRetained {
                sequence,
                path: retained_path,
                identity,
            } => {
                let intent = find_intent(path, &mut intents, *sequence)?;
                match intent.kind {
                    IntentKind::Directory {
                        action: DirectoryAction::Remove,
                        path: intended_path,
                        identity: Some(expected_identity),
                    } if intended_path == retained_path
                        && expected_identity == identity
                        && !intent.completed =>
                    {
                        intent.completed = true;
                    }
                    _ => {
                        return Err(invalid_events(
                            path,
                            format!("sequence {sequence} has a mismatched retained directory"),
                        ));
                    }
                }
            }
        }
    }

    Ok(())
}

fn validate_status_transition(
    path: &Path,
    previous: Option<JournalStatusRecord>,
    next: JournalStatusRecord,
) -> Result<(), JournalError> {
    let valid = match previous {
        None => next != JournalStatusRecord::Undone,
        Some(JournalStatusRecord::Applying) => matches!(
            next,
            JournalStatusRecord::Applying
                | JournalStatusRecord::Applied
                | JournalStatusRecord::FailedRolledBack
                | JournalStatusRecord::Incomplete
                | JournalStatusRecord::PartiallyRecovered
                | JournalStatusRecord::Undoing
        ),
        Some(
            JournalStatusRecord::Applied
            | JournalStatusRecord::Incomplete
            | JournalStatusRecord::PartiallyRecovered,
        ) => next == JournalStatusRecord::Undoing,
        Some(JournalStatusRecord::Undoing) => matches!(
            next,
            JournalStatusRecord::Undoing | JournalStatusRecord::Undone
        ),
        Some(JournalStatusRecord::FailedRolledBack | JournalStatusRecord::Undone) => false,
    };

    if !valid {
        return Err(invalid_events(
            path,
            format!("invalid status transition {previous:?} -> {next:?}"),
        ));
    }

    Ok(())
}

fn insert_intent<'a>(
    path: &Path,
    intents: &mut BTreeMap<u64, Intent<'a>>,
    last_sequence: &mut Option<u64>,
    sequence: u64,
    kind: IntentKind<'a>,
) -> Result<(), JournalError> {
    if intents.contains_key(&sequence) {
        return Err(invalid_events(
            path,
            format!("duplicate journal sequence {sequence}"),
        ));
    }
    if last_sequence.is_some_and(|previous| sequence < previous) {
        return Err(invalid_events(
            path,
            format!("journal sequence {sequence} is out of order"),
        ));
    }
    intents.insert(
        sequence,
        Intent {
            kind,
            completed: false,
        },
    );
    *last_sequence = Some(sequence);

    Ok(())
}

fn find_intent<'entry, 'records>(
    path: &Path,
    intents: &'entry mut BTreeMap<u64, Intent<'records>>,
    sequence: u64,
) -> Result<&'entry mut Intent<'records>, JournalError> {
    intents.get_mut(&sequence).ok_or_else(|| {
        invalid_events(
            path,
            format!("sequence {sequence} completion has no matching intent"),
        )
    })
}

fn complete_intent(
    path: &Path,
    intents: &mut BTreeMap<u64, Intent<'_>>,
    sequence: u64,
    completion: CompletionKind,
) -> Result<(), JournalError> {
    let intent = find_intent(path, intents, sequence)?;
    let matches_kind = matches!(
        (completion, &intent.kind),
        (CompletionKind::Move, IntentKind::Move { .. })
            | (CompletionKind::Directory, IntentKind::Directory { .. })
            | (CompletionKind::TextEdit, IntentKind::TextEdit)
    );
    if !matches_kind || intent.completed {
        return Err(invalid_events(
            path,
            format!("sequence {sequence} has a duplicate or mismatched completion"),
        ));
    }
    intent.completed = true;

    Ok(())
}

#[derive(Clone, Copy)]
enum CompletionKind {
    Move,
    Directory,
    TextEdit,
}

fn invalid_events(path: &Path, message: String) -> JournalError {
    JournalError::InvalidEvents {
        path: path.to_path_buf(),
        message,
    }
}

pub(super) fn validate_target(history: &JournalHistory) -> Result<(), JournalError> {
    validate_target_identity(&history.id, &history.target, &history.target_identity)
}

pub(super) fn validate_target_identity(
    run_id: &str,
    target: &Path,
    identity: &ObjectIdentity,
) -> Result<(), JournalError> {
    let canonical = target
        .canonicalize()
        .map_err(|source| JournalError::io(target, source))?;
    let metadata =
        fs::symlink_metadata(&canonical).map_err(|source| JournalError::io(&canonical, source))?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || !identity.matches(&metadata, EntryKind::Directory, &canonical)
    {
        return Err(JournalError::ChangedTarget(run_id.to_owned()));
    }
    Ok(())
}

pub(super) fn checked_target_path(target: &Path, relative: &Path) -> Result<PathBuf, JournalError> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(JournalError::TargetPath(relative.to_path_buf()));
    }
    let absolute = target.join(relative);
    let parent = absolute.parent().unwrap_or(target);
    let relative_parent = parent
        .strip_prefix(target)
        .map_err(|_| JournalError::TargetPath(relative.to_path_buf()))?;
    let mut current = target.to_path_buf();
    for component in relative_parent.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(JournalError::ChangedObject {
                    run_id: String::new(),
                    path: current,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(JournalError::ChangedObject {
                    run_id: String::new(),
                    path: current,
                });
            }
            Err(source) => return Err(JournalError::io(current, source)),
        }
    }
    Ok(absolute)
}

fn operation_reached_destination(
    target: &Path,
    source: &Path,
    destination: &Path,
    identity: &ObjectIdentity,
) -> Result<bool, JournalError> {
    let source = checked_target_path(target, source)?;
    let destination = checked_target_path(target, destination)?;
    Ok(matches_identity(&destination, identity)? && !matches_identity(&source, identity)?)
}

fn operation_still_at_source(
    target: &Path,
    source: &Path,
    destination: &Path,
    identity: &ObjectIdentity,
) -> Result<bool, JournalError> {
    let source = checked_target_path(target, source)?;
    let destination = checked_target_path(target, destination)?;
    Ok(matches_identity(&source, identity)? && !path_exists(&destination)?)
}

fn matches_identity(path: &Path, identity: &ObjectIdentity) -> Result<bool, JournalError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(identity.matches(&metadata, identity.kind, path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(JournalError::io(path, source)),
    }
}

fn path_exists(path: &Path) -> Result<bool, JournalError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(JournalError::io(path, source)),
    }
}

pub(super) fn preflight_undo(
    run_id: &str,
    target: &Path,
    mutations: &[UndoMutation],
) -> Result<(), JournalError> {
    let mut current_locations = std::collections::HashMap::new();
    for mutation in mutations {
        if let UndoMutation::Move {
            object,
            destination,
            identity,
            ..
        } = mutation
        {
            current_locations
                .entry(*object)
                .or_insert_with(|| (destination.clone(), identity.clone()));
        }
    }
    let mut released = Vec::new();
    let mut verified_objects = std::collections::HashSet::new();
    for mutation in mutations {
        match mutation {
            UndoMutation::Move {
                object,
                source,
                destination,
                identity,
            } => {
                let expected = current_locations.get(object).ok_or_else(|| {
                    JournalError::AmbiguousOperation {
                        run_id: run_id.into(),
                        path: destination.clone(),
                    }
                })?;
                if expected.0 != *destination {
                    return Err(JournalError::AmbiguousOperation {
                        run_id: run_id.into(),
                        path: destination.clone(),
                    });
                }
                let source_path = checked_target_path(target, destination)?;
                if verified_objects.insert(*object) && !matches_identity(&source_path, identity)? {
                    return Err(JournalError::ChangedObject {
                        run_id: run_id.into(),
                        path: source_path,
                    });
                }
                let destination_path = checked_target_path(target, source)?;
                if path_exists(&destination_path)?
                    && !was_released(&released, &destination_path)?
                    && !same_existing_object_alias(&source_path, &destination_path, identity)?
                {
                    return Err(JournalError::ExistingDestination(destination_path));
                }
                current_locations.insert(*object, (source.clone(), identity.clone()));
                released.push((source_path, identity.clone()));
            }
            UndoMutation::Directory {
                path,
                action,
                identity,
            } => {
                let absolute = checked_target_path(target, path)?;
                if *action == DirectoryAction::Create {
                    let metadata = fs::symlink_metadata(&absolute)
                        .map_err(|source| JournalError::io(&absolute, source))?;
                    if !identity.matches(&metadata, EntryKind::Directory, &absolute) {
                        return Err(JournalError::ChangedObject {
                            run_id: run_id.into(),
                            path: absolute,
                        });
                    }
                } else if path_exists(&absolute)? && !was_released(&released, &absolute)? {
                    return Err(JournalError::ExistingDestination(absolute));
                }
            }
            UndoMutation::TextEdit {
                path,
                before,
                after,
                ..
            } => {
                let absolute = checked_target_path(target, path)?;
                let metadata = fs::symlink_metadata(&absolute)
                    .map_err(|source| JournalError::io(&absolute, source))?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(JournalError::ChangedObject {
                        run_id: run_id.into(),
                        path: absolute,
                    });
                }
                let contents = fs::read_to_string(&absolute)
                    .map_err(|source| JournalError::io(&absolute, source))?;
                if contents != *before && contents != *after {
                    return Err(JournalError::ChangedObject {
                        run_id: run_id.into(),
                        path: absolute,
                    });
                }
            }
        }
    }
    Ok(())
}

fn was_released(released: &[(PathBuf, ObjectIdentity)], path: &Path) -> Result<bool, JournalError> {
    for (released_path, identity) in released {
        if released_path == path || same_existing_object_alias(released_path, path, identity)? {
            return Ok(true);
        }
    }

    Ok(false)
}

pub(super) fn same_existing_path(left: &Path, right: &Path) -> bool {
    left == right
        || matches!(
                (fs::canonicalize(left), fs::canonicalize(right)),
                (Ok(left), Ok(right)) if left == right
        )
}

pub(super) fn same_existing_object_alias(
    source: &Path,
    destination: &Path,
    identity: &ObjectIdentity,
) -> Result<bool, JournalError> {
    if same_existing_path(source, destination) {
        return Ok(true);
    }

    let source_components = source.components().collect::<Vec<_>>();
    let destination_components = destination.components().collect::<Vec<_>>();
    if source_components.len() != destination_components.len()
        || !source_components
            .iter()
            .zip(&destination_components)
            .all(|(source, destination)| {
                source
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&destination.as_os_str().to_string_lossy())
            })
    {
        return Ok(false);
    }

    match fs::symlink_metadata(destination) {
        Ok(metadata) => Ok(identity.matches(&metadata, identity.kind, destination)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(JournalError::io(destination, source)),
    }
}

fn cancel_inverse_mutations(mutations: Vec<UndoMutation>) -> Vec<UndoMutation> {
    let mut pending = Vec::<UndoMutation>::new();
    for mutation in mutations {
        let cancels = pending
            .last()
            .is_some_and(|previous| match (previous, &mutation) {
                (
                    UndoMutation::Move {
                        object: a,
                        source: sa,
                        destination: da,
                        ..
                    },
                    UndoMutation::Move {
                        object: b,
                        source: sb,
                        destination: db,
                        ..
                    },
                ) => a == b && sa == db && da == sb,
                (
                    UndoMutation::Directory {
                        path: a,
                        action: aa,
                        ..
                    },
                    UndoMutation::Directory {
                        path: b,
                        action: ab,
                        ..
                    },
                ) => a == b && aa != ab,
                _ => false,
            });
        if cancels {
            pending.pop();
        } else {
            pending.push(mutation);
        }
    }
    pending
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::path_exists;
    use crate::{
        ConflictPolicy, FileTextEdit, JournalError, JournalStatus, JournalStore,
        apply_journaled_with_text_edits, scan,
    };
    use dirule_core::Plan;
    use std::path::Path;

    #[cfg(unix)]
    #[test]
    fn path_existence_propagates_metadata_errors() {
        use std::os::unix::ffi::OsStrExt;

        let invalid_path = std::ffi::OsStr::from_bytes(b"invalid\0path");
        let error = path_exists(Path::new(invalid_path)).expect_err("metadata path is invalid");

        assert!(matches!(error, JournalError::Io { .. }));
    }

    #[test]
    fn undo_refuses_to_overwrite_a_text_document_changed_after_apply() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let target = temp.path().join("project");
        std::fs::create_dir(&target).expect("create project target");
        let document = target.join("references.rs");
        std::fs::write(&document, "mod HTTPClient;\n").expect("write reference document");
        let snapshot = scan(&target).expect("scan project");
        let journal = JournalStore::new(temp.path().join("state/runs"));
        let edit = FileTextEdit {
            path: Path::new("references.rs").to_path_buf(),
            before: "mod HTTPClient;\n".into(),
            after: "mod http_client;\n".into(),
        };

        let applied = apply_journaled_with_text_edits(
            &target,
            &Plan::default(),
            &snapshot,
            ConflictPolicy::Abort,
            &[edit],
            &journal,
        )
        .expect("apply reference edit");
        let run_id = applied.run_id.expect("text edit creates a journal run");
        assert_eq!(
            std::fs::read_to_string(&document).unwrap(),
            "mod http_client;\n"
        );

        std::fs::write(&document, "mod user_changed_this;\n").expect("edit document after apply");
        let error = journal
            .undo(&run_id)
            .expect_err("undo must not overwrite the user's later edit");

        assert!(matches!(error, JournalError::ChangedObject { .. }));
        assert_eq!(
            std::fs::read_to_string(&document).unwrap(),
            "mod user_changed_this;\n"
        );
        assert_eq!(journal.list().unwrap()[0].status, JournalStatus::Applied);
    }
}
