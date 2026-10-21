use crate::apply::ApplyError;
use dirule_core::{EntryKind, Plan};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

mod history;
mod identity;
mod migrations;
mod writer;

use history::{
    JournalHistory, UndoMutation, checked_target_path, preflight_undo, validate_target,
    validate_target_identity,
};
use identity::{EncodedPath, ObjectIdentity};
pub(crate) use writer::JournalRecorder;
use writer::{JournalWriter, LockMode, RunLock};

pub const CURRENT_JOURNAL_VERSION: &str = "1.1";

static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(0);
static NEXT_UNDO_RENAME: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("cannot access journal state at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("journal {path} is invalid at line {line}: {source}")]
    InvalidRecord {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("journal {path} uses unsupported version {version}")]
    UnsupportedVersion { path: PathBuf, version: String },
    #[error("journal {path} cannot be migrated: {message}")]
    Migration { path: PathBuf, message: String },
    #[error("journal {path} has invalid events: {message}")]
    InvalidEvents { path: PathBuf, message: String },
    #[error("run identifier is invalid: {0}")]
    InvalidRunId(String),
    #[error("run {0} cannot be undone: {1}")]
    NotUndoable(String, String),
    #[error("run {0} is currently being modified; retry after it finishes")]
    RunBusy(String),
    #[error("run {0} no longer targets the same directory")]
    ChangedTarget(String),
    #[error("cannot verify run {run_id} operation at {path}")]
    ChangedObject { run_id: String, path: PathBuf },
    #[error("undo would overwrite an existing path: {0}")]
    ExistingDestination(PathBuf),
    #[error("run {run_id} has an unresolved filesystem operation at {path}")]
    AmbiguousOperation { run_id: String, path: PathBuf },
    #[error("cannot encode target path for journal state: {0}")]
    TargetPath(PathBuf),
}

impl JournalError {
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

impl From<JournalError> for ApplyError {
    fn from(error: JournalError) -> Self {
        Self::Journal(Box::new(error))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalRun {
    pub id: String,
    pub target: PathBuf,
    pub started_at: u64,
    pub status: JournalStatus,
    pub operations: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalDetails {
    pub run: JournalRun,
    pub operations: Vec<JournalOperation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalUndoPreview {
    pub run: JournalRun,
    pub actions: Vec<UndoAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalOperation {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub kind: EntryKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UndoAction {
    Move {
        source: PathBuf,
        destination: PathBuf,
        kind: EntryKind,
    },
    CreateDirectory {
        path: PathBuf,
    },
    RemoveDirectoryIfEmpty {
        path: PathBuf,
    },
    RestoreFileContents {
        path: PathBuf,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UndoReport {
    pub retained_directories: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalStatus {
    Applying,
    Applied,
    FailedRolledBack,
    Interrupted,
    Undoing,
    PartiallyRecovered,
    Undone,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
enum JournalRecord {
    Header {
        version: String,
        id: String,
        target: EncodedPath,
        target_display: String,
        target_identity: ObjectIdentity,
        started_at: u64,
        plan_fingerprint: String,
    },
    MoveIntent {
        sequence: u64,
        object: u64,
        source: PathBuf,
        destination: PathBuf,
        identity: ObjectIdentity,
    },
    MoveCompleted {
        sequence: u64,
        identity: ObjectIdentity,
    },
    DirectoryIntent {
        sequence: u64,
        path: PathBuf,
        action: DirectoryAction,
        identity: Option<ObjectIdentity>,
    },
    DirectoryChanged {
        sequence: u64,
        action: DirectoryAction,
        identity: ObjectIdentity,
    },
    DirectoryRetained {
        sequence: u64,
        path: PathBuf,
        identity: ObjectIdentity,
    },
    TextEditIntent {
        sequence: u64,
        path: PathBuf,
        before: String,
        after: String,
    },
    TextEditCompleted {
        sequence: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity: Option<ObjectIdentity>,
    },
    TextEditUndone {
        sequence: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity: Option<ObjectIdentity>,
    },
    Status {
        status: JournalStatusRecord,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DirectoryAction {
    Create,
    Remove,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JournalStatusRecord {
    Applying,
    Applied,
    FailedRolledBack,
    Incomplete,
    Undoing,
    PartiallyRecovered,
    Undone,
}

impl From<JournalStatusRecord> for JournalStatus {
    fn from(status: JournalStatusRecord) -> Self {
        match status {
            JournalStatusRecord::Applying => Self::Applying,
            JournalStatusRecord::Applied => Self::Applied,
            JournalStatusRecord::FailedRolledBack => Self::FailedRolledBack,
            JournalStatusRecord::Incomplete => Self::Interrupted,
            JournalStatusRecord::Undoing => Self::Undoing,
            JournalStatusRecord::PartiallyRecovered => Self::PartiallyRecovered,
            JournalStatusRecord::Undone => Self::Undone,
        }
    }
}

impl From<JournalStatus> for JournalStatusRecord {
    fn from(status: JournalStatus) -> Self {
        match status {
            JournalStatus::Applying => Self::Applying,
            JournalStatus::Applied => Self::Applied,
            JournalStatus::FailedRolledBack => Self::FailedRolledBack,
            JournalStatus::Interrupted => Self::Incomplete,
            JournalStatus::Undoing => Self::Undoing,
            JournalStatus::PartiallyRecovered => Self::PartiallyRecovered,
            JournalStatus::Undone => Self::Undone,
        }
    }
}

pub struct JournalStore {
    directory: PathBuf,
}

impl JournalStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn list(&self) -> Result<Vec<JournalRun>, JournalError> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(JournalError::io(&self.directory, source)),
        };
        let mut runs = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| JournalError::io(&self.directory, source))?;
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            {
                let run_id = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| JournalError::InvalidRunId(path.display().to_string()))?;
                let _lock = RunLock::acquire_existing(&path, run_id, LockMode::Shared)?;
                runs.push(read_run(&path)?);
            }
        }
        runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));
        Ok(runs)
    }

    pub fn get(&self, run_id: &str) -> Result<JournalDetails, JournalError> {
        let path = self.run_path(run_id)?;
        let _lock = RunLock::acquire_existing(&path, run_id, LockMode::Shared)?;
        let journal = read_records(&path)?;
        let history = JournalHistory::from_records(&path, journal.records)?;
        if history.id != run_id {
            return Err(JournalError::InvalidRunId(run_id.to_owned()));
        }

        let run = run_from_history(&history);
        let operations = history
            .records
            .into_iter()
            .take_while(|record| {
                !matches!(
                    record,
                    JournalRecord::Status {
                        status: JournalStatusRecord::Undoing
                    }
                )
            })
            .filter_map(|record| match record {
                JournalRecord::MoveIntent {
                    source,
                    destination,
                    identity,
                    ..
                } => Some(JournalOperation {
                    source,
                    destination,
                    kind: identity.kind,
                }),
                _ => None,
            })
            .collect();

        Ok(JournalDetails { run, operations })
    }

    pub fn preview_undo(&self, run_id: &str) -> Result<JournalUndoPreview, JournalError> {
        let path = self.run_path(run_id)?;
        let _lock = RunLock::acquire_existing(&path, run_id, LockMode::Shared)?;
        let journal = read_records(&path)?;
        let history = JournalHistory::from_records(&path, journal.records)?;
        history.validate_undoable(run_id)?;

        let target = history.target.clone();
        validate_target(&history)?;
        let mut mutations = history.mutations()?;
        mutations.reverse();
        preflight_undo(run_id, &target, &mutations)?;

        let actions = mutations
            .into_iter()
            .map(|mutation| match mutation {
                UndoMutation::Move {
                    source,
                    destination,
                    identity,
                    ..
                } => UndoAction::Move {
                    source: destination,
                    destination: source,
                    kind: identity.kind,
                },
                UndoMutation::Directory {
                    path,
                    action: DirectoryAction::Create,
                    ..
                } => UndoAction::RemoveDirectoryIfEmpty { path },
                UndoMutation::Directory {
                    path,
                    action: DirectoryAction::Remove,
                    ..
                } => UndoAction::CreateDirectory { path },
                UndoMutation::TextEdit { path, .. } => UndoAction::RestoreFileContents { path },
            })
            .collect();

        Ok(JournalUndoPreview {
            run: run_from_history(&history),
            actions,
        })
    }

    pub fn undo(&self, run_id: &str) -> Result<UndoReport, JournalError> {
        let path = self.run_path(run_id)?;
        let run_lock = RunLock::acquire_existing(&path, run_id, LockMode::Exclusive)?;
        let journal = read_records(&path)?;
        let history = JournalHistory::from_records(&path, journal.records)?;
        history.validate_undoable(run_id)?;

        let target = history.target.clone();
        validate_target(&history)?;
        let mut mutations = history.mutations()?;
        mutations.reverse();
        preflight_undo(run_id, &target, &mutations)?;

        let mut writer = JournalWriter::open(
            &path,
            run_id,
            &target,
            &history.records,
            journal.valid_length,
            journal.observed_length,
            run_lock,
        )?;
        writer.status(JournalStatus::Undoing)?;
        let mut retained_directories = history
            .records
            .iter()
            .filter_map(|record| match record {
                JournalRecord::DirectoryRetained { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut text_edit_undo_identities = HashMap::new();
        for mutation in mutations {
            validate_target_identity(run_id, &target, &history.target_identity)?;
            match mutation {
                UndoMutation::Move {
                    object,
                    source,
                    destination,
                    identity,
                } => {
                    let identity = text_edit_undo_identities
                        .get(&destination)
                        .cloned()
                        .unwrap_or(identity);
                    let source_path = checked_target_path(&target, &destination)?;
                    let destination_path = checked_target_path(&target, &source)?;
                    let metadata = fs::symlink_metadata(&source_path)
                        .map_err(|source| JournalError::io(&source_path, source))?;
                    if !identity.matches(&metadata, identity.kind, &source_path) {
                        return Err(JournalError::ChangedObject {
                            run_id: run_id.to_owned(),
                            path: source_path,
                        });
                    }
                    if history::same_existing_object_alias(
                        &source_path,
                        &destination_path,
                        &identity,
                    )? {
                        let temporary = temporary_sibling(&source_path)?;
                        record_undo_move(
                            &mut writer,
                            object,
                            identity.kind,
                            &source_path,
                            &temporary,
                        )?;
                        record_undo_move(
                            &mut writer,
                            object,
                            identity.kind,
                            &temporary,
                            &destination_path,
                        )?;
                    } else {
                        record_undo_move(
                            &mut writer,
                            object,
                            identity.kind,
                            &source_path,
                            &destination_path,
                        )?;
                    }
                }
                UndoMutation::Directory {
                    path,
                    action,
                    identity,
                } => match action {
                    DirectoryAction::Create => {
                        let directory = checked_target_path(&target, &path)?;
                        let metadata = fs::symlink_metadata(&directory)
                            .map_err(|source| JournalError::io(&directory, source))?;
                        if !identity.matches(&metadata, EntryKind::Directory, &directory) {
                            return Err(JournalError::ChangedObject {
                                run_id: run_id.to_owned(),
                                path: directory,
                            });
                        }
                        let sequence = writer.directory_remove_intent(&directory)?;
                        match fs::remove_dir(&directory) {
                            Ok(()) => writer.directory_removed(sequence)?,
                            Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                                let metadata = fs::symlink_metadata(&directory)
                                    .map_err(|source| JournalError::io(&directory, source))?;
                                if !identity.matches(&metadata, EntryKind::Directory, &directory) {
                                    return Err(JournalError::ChangedObject {
                                        run_id: run_id.to_owned(),
                                        path: directory,
                                    });
                                }
                                writer.directory_retained(sequence, &directory)?;
                                if !retained_directories.contains(&path) {
                                    retained_directories.push(path);
                                }
                            }
                            Err(source) => {
                                return Err(JournalError::io(&directory, source));
                            }
                        }
                    }
                    DirectoryAction::Remove => {
                        let directory = checked_target_path(&target, &path)?;
                        let sequence = writer.directory_intent(&directory)?;
                        fs::create_dir(&directory)
                            .map_err(|error| JournalError::io(&directory, error))?;
                        writer.directory_created(sequence, &directory)?;
                    }
                },
                UndoMutation::TextEdit {
                    sequence,
                    path: relative,
                    before,
                    after,
                } => {
                    let file = checked_target_path(&target, &relative)?;
                    let metadata = fs::symlink_metadata(&file)
                        .map_err(|source| JournalError::io(&file, source))?;
                    if !metadata.is_file() || metadata.file_type().is_symlink() {
                        return Err(JournalError::ChangedObject {
                            run_id: run_id.to_owned(),
                            path: file,
                        });
                    }
                    let current = fs::read_to_string(&file)
                        .map_err(|source| JournalError::io(&file, source))?;
                    if current == after {
                        crate::apply::replace_file_contents(&file, &before)
                            .map_err(|source| JournalError::io(&file, source))?;
                    } else if current != before {
                        return Err(JournalError::ChangedObject {
                            run_id: run_id.to_owned(),
                            path: file,
                        });
                    }
                    let metadata = fs::symlink_metadata(&file)
                        .map_err(|source| JournalError::io(&file, source))?;
                    text_edit_undo_identities.insert(
                        relative.clone(),
                        ObjectIdentity::from_metadata(&metadata, EntryKind::File, &file),
                    );
                    writer.text_edit_undone(sequence, &file)?;
                }
            }
        }
        writer.status(JournalStatus::Undone)?;

        Ok(UndoReport {
            retained_directories,
        })
    }

    pub(crate) fn begin(&self, target: &Path, plan: &Plan) -> Result<JournalWriter, JournalError> {
        let target = target
            .canonicalize()
            .map_err(|source| JournalError::io(target, source))?;
        let metadata =
            fs::symlink_metadata(&target).map_err(|source| JournalError::io(&target, source))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(JournalError::TargetPath(target));
        }
        prepare_state_directory(&self.directory)?;

        loop {
            let id = next_run_id();
            let path = self.directory.join(format!("{id}.jsonl"));
            let temporary_path = self.directory.join(format!("{id}.tmp"));
            let lock = match RunLock::acquire(&path, &id, LockMode::Exclusive) {
                Ok(lock) => lock,
                Err(JournalError::RunBusy(_)) => continue,
                Err(error) => return Err(error),
            };
            let started_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let header = JournalRecord::Header {
                version: CURRENT_JOURNAL_VERSION.to_owned(),
                id: id.clone(),
                target: EncodedPath::from_path(&target)?,
                target_display: target.to_string_lossy().into_owned(),
                target_identity: ObjectIdentity::from_metadata(
                    &metadata,
                    EntryKind::Directory,
                    &target,
                ),
                started_at,
                plan_fingerprint: fingerprint(plan),
            };
            match create_private_file(&temporary_path) {
                Ok(mut file) => {
                    if let Err(error) = write_record(&mut file, &header, &temporary_path) {
                        drop(file);
                        let _ = fs::remove_file(&temporary_path);
                        return Err(error);
                    }
                    drop(file);
                    if let Err(source) = renamore::rename_exclusive(&temporary_path, &path) {
                        let _ = fs::remove_file(&temporary_path);
                        if source.kind() == std::io::ErrorKind::AlreadyExists {
                            continue;
                        }
                        return Err(JournalError::io(&path, source));
                    }
                    sync_directory(&self.directory)?;
                    let file = OpenOptions::new()
                        .append(true)
                        .open(&path)
                        .map_err(|source| JournalError::io(&path, source))?;
                    return Ok(JournalWriter::create(id, target, path, file, lock));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => return Err(JournalError::io(temporary_path, source)),
            }
        }
    }

    pub(crate) fn run_path(&self, run_id: &str) -> Result<PathBuf, JournalError> {
        if run_id.is_empty()
            || !run_id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            return Err(JournalError::InvalidRunId(run_id.to_owned()));
        }
        Ok(self.directory.join(format!("{run_id}.jsonl")))
    }
}

fn record_undo_move(
    writer: &mut JournalWriter,
    object: u64,
    kind: EntryKind,
    source: &Path,
    destination: &Path,
) -> Result<(), JournalError> {
    let sequence = writer.move_intent(object, source, destination, kind)?;
    renamore::rename_exclusive(source, destination)
        .map_err(|error| JournalError::io(source, error))?;
    writer.move_completed(sequence, destination, kind)
}

fn temporary_sibling(source: &Path) -> Result<PathBuf, JournalError> {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    loop {
        let id = NEXT_UNDO_RENAME.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".dirule-undo-{}-{id}", std::process::id()));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(path),
            Ok(_) => continue,
            Err(source) => return Err(JournalError::io(path, source)),
        }
    }
}

fn read_run(path: &Path) -> Result<JournalRun, JournalError> {
    let journal = read_records(path)?;
    let history = JournalHistory::from_records(path, journal.records)?;
    Ok(run_from_history(&history))
}

fn run_from_history(history: &JournalHistory) -> JournalRun {
    let mut operations = 0;
    for record in &history.records {
        match record {
            JournalRecord::Status {
                status: JournalStatusRecord::Undoing,
            } => break,
            JournalRecord::MoveIntent { .. } => operations += 1,
            _ => {}
        }
    }
    JournalRun {
        id: history.id.clone(),
        target: history.target.clone(),
        started_at: history.started_at,
        status: history.status,
        operations,
    }
}

struct JournalRecords {
    records: Vec<JournalRecord>,
    valid_length: u64,
    observed_length: u64,
}

fn read_records(path: &Path) -> Result<JournalRecords, JournalError> {
    let file = File::open(path).map_err(|source| JournalError::io(path, source))?;
    let mut reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut line = Vec::new();
    let mut line_number = 0;
    let mut valid_length = 0;
    let mut observed_length = 0;
    loop {
        line.clear();
        if reader
            .read_until(b'\n', &mut line)
            .map_err(|source| JournalError::io(path, source))?
            == 0
        {
            break;
        }
        line_number += 1;
        observed_length += line.len() as u64;
        if !line.ends_with(b"\n") {
            if records.is_empty() {
                return Err(JournalError::InvalidRunId(path.display().to_string()));
            }
            break;
        }
        valid_length += line.len() as u64;
        records.push(parse_value(path, line_number, &line)?);
    }
    if records.is_empty() {
        return Err(JournalError::InvalidRunId(path.display().to_string()));
    }
    let version =
        migrations::read_version(&records).map_err(|message| JournalError::Migration {
            path: path.to_path_buf(),
            message,
        })?;
    let records = migrations::upgrade(path, records, &version).map_err(|message| {
        JournalError::Migration {
            path: path.to_path_buf(),
            message,
        }
    })?;
    let records = records
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            serde_json::from_value(record).map_err(|source| JournalError::InvalidRecord {
                path: path.to_path_buf(),
                line: index + 1,
                source,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(JournalRecords {
        records,
        valid_length,
        observed_length,
    })
}

fn parse_value(path: &Path, line: usize, bytes: &[u8]) -> Result<serde_json::Value, JournalError> {
    serde_json::from_slice(bytes).map_err(|source| JournalError::InvalidRecord {
        path: path.to_path_buf(),
        line,
        source,
    })
}

fn write_record(file: &mut File, record: &JournalRecord, path: &Path) -> Result<(), JournalError> {
    let mut bytes = serde_json::to_vec(record)
        .map_err(|source| JournalError::io(path, std::io::Error::other(source)))?;
    bytes.push(b'\n');

    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| JournalError::io(path, source))
}

fn create_private_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).append(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn prepare_state_directory(path: &Path) -> Result<(), JournalError> {
    if !path.is_absolute() {
        return Err(JournalError::io(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "journal state directory must be absolute",
            ),
        ));
    }

    #[cfg(unix)]
    {
        if path == Path::new("/") {
            return Err(JournalError::io(
                path,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "journal state directory must not be the filesystem root",
                ),
            ));
        }

        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            ensure_parent_directory(parent, true)?;
        }

        create_private_directory(path)?;
    }

    #[cfg(not(unix))]
    fs::create_dir_all(path).map_err(|source| JournalError::io(path, source))?;

    Ok(())
}

#[cfg(unix)]
fn ensure_parent_directory(path: &Path, reject_symlink: bool) -> Result<(), JournalError> {
    let metadata = match if reject_symlink {
        fs::symlink_metadata(path)
    } else {
        fs::metadata(path)
    } {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty() && *parent != path)
            {
                ensure_parent_directory(parent, false)?;
            }

            return create_private_directory_if_missing(path);
        }
        Err(source) => return Err(JournalError::io(path, source)),
    };

    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(JournalError::io(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "journal parent must be a directory and not a symlink",
            ),
        ));
    }

    Ok(())
}

#[cfg(unix)]
fn create_private_directory_if_missing(path: &Path) -> Result<(), JournalError> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);

    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata =
                fs::symlink_metadata(path).map_err(|source| JournalError::io(path, source))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(JournalError::io(
                    path,
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "journal parent must be a directory and not a symlink",
                    ),
                ));
            }

            Ok(())
        }
        Err(source) => Err(JournalError::io(path, source)),
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<(), JournalError> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);

    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata =
                fs::symlink_metadata(path).map_err(|source| JournalError::io(path, source))?;
            if !metadata.file_type().is_dir() {
                return Err(JournalError::io(
                    path,
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "journal state path must be a directory and not a symlink",
                    ),
                ));
            }

            set_private_directory(path)
        }
        Err(source) => Err(JournalError::io(path, source)),
    }
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<(), JournalError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|source| JournalError::io(path, source))
}

fn sync_directory(_path: &Path) -> Result<(), JournalError> {
    #[cfg(unix)]
    File::open(_path)
        .and_then(|file| file.sync_all())
        .map_err(|source| JournalError::io(_path, source))?;
    Ok(())
}

fn next_run_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed);
    format!("{timestamp:x}-{:x}-{counter:x}", std::process::id())
}

fn fingerprint(plan: &Plan) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    let mut write = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(PRIME);
    };
    for entry in &plan.entries {
        write(entry.source.to_string_lossy().as_bytes());
        write(entry.destination.to_string_lossy().as_bytes());
        write(match entry.kind {
            EntryKind::File => b"file",
            EntryKind::Directory => b"directory",
            EntryKind::Symlink => b"symlink",
        });
        write(entry.rule.as_deref().unwrap_or_default().as_bytes());
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(unix)]
fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[(byte >> 4) as usize]));
        encoded.push(char::from(DIGITS[(byte & 0x0f) as usize]));
    }
    encoded
}

#[cfg(unix)]
fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() & 1 != 0 {
        return None;
    }
    let (pairs, _) = value.as_bytes().as_chunks::<2>();

    pairs
        .iter()
        .map(|pair| {
            let digit = |value: u8| match value {
                b'0'..=b'9' => Some(value - b'0'),
                b'a'..=b'f' => Some(value - b'a' + 10),
                _ => None,
            };
            Some((digit(pair[0])? << 4) | digit(pair[1])?)
        })
        .collect()
}
