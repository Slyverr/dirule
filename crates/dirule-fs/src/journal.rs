use crate::apply::{ApplyError, MutationRecorder};
use dirule_core::{EntryKind, Plan};
use fs4::{FileExt, TryLockError};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

mod history;
mod identity;
mod migrations;

use history::{
    JournalHistory, UndoMutation, checked_target_path, preflight_undo, validate_target,
    validate_target_identity,
};
use identity::{EncodedPath, ObjectIdentity};

pub const CURRENT_JOURNAL_VERSION: &str = "1.0";

static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(0);

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
pub struct JournalOperation {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub kind: EntryKind,
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
        let records = read_records(&path)?;
        let history = JournalHistory::from_records(&path, records)?;
        if history.id != run_id {
            return Err(JournalError::InvalidRunId(run_id.to_owned()));
        }

        let run = run_from_history(&history);
        let operations = history
            .records
            .iter()
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
                    source: source.clone(),
                    destination: destination.clone(),
                    kind: identity.kind,
                }),
                _ => None,
            })
            .collect();

        Ok(JournalDetails { run, operations })
    }

    pub fn undo(&self, run_id: &str) -> Result<UndoReport, JournalError> {
        let path = self.run_path(run_id)?;
        let run_lock = RunLock::acquire_existing(&path, run_id, LockMode::Exclusive)?;
        let records = read_records(&path)?;
        let history = JournalHistory::from_records(&path, records)?;
        history.validate_undoable(run_id)?;

        let target = history.target.clone();
        validate_target(&history)?;
        let mut mutations = history.mutations()?;
        mutations.reverse();
        preflight_undo(run_id, &target, &mutations)?;

        let mut writer = JournalWriter::open(&path, run_id, &target, &history.records, run_lock)?;
        writer.status(JournalStatus::Undoing)?;
        let mut retained_directories = history
            .records
            .iter()
            .filter_map(|record| match record {
                JournalRecord::DirectoryRetained { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        for mutation in mutations {
            validate_target_identity(run_id, &target, &history.target_identity)?;
            match mutation {
                UndoMutation::Move {
                    object,
                    source,
                    destination,
                    identity,
                } => {
                    let source_path = checked_target_path(&target, &destination)?;
                    let destination_path = checked_target_path(&target, &source)?;
                    let metadata = fs::symlink_metadata(&source_path)
                        .map_err(|source| JournalError::io(&source_path, source))?;
                    if !identity.matches(&metadata, identity.kind) {
                        return Err(JournalError::ChangedObject {
                            run_id: run_id.to_owned(),
                            path: source_path,
                        });
                    }
                    let sequence = writer.move_intent(
                        object,
                        &source_path,
                        &destination_path,
                        identity.kind,
                    )?;
                    renamore::rename_exclusive(&source_path, &destination_path)
                        .map_err(|error| JournalError::io(&source_path, error))?;
                    writer.move_completed(sequence, &destination_path, identity.kind)?;
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
                        if !identity.matches(&metadata, EntryKind::Directory) {
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
                                if !identity.matches(&metadata, EntryKind::Directory) {
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
        fs::create_dir_all(&self.directory)
            .map_err(|source| JournalError::io(&self.directory, source))?;
        set_private_directory(&self.directory)?;
        if let Some(parent) = self.directory.parent() {
            set_private_directory(parent)?;
        }

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
                target_identity: ObjectIdentity::from_metadata(&metadata, EntryKind::Directory),
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
                    return Ok(JournalWriter {
                        id,
                        target,
                        path,
                        file,
                        next_sequence: 0,
                        _run_lock: lock,
                    });
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

pub(crate) struct JournalWriter {
    pub(crate) id: String,
    pub(crate) target: PathBuf,
    path: PathBuf,
    file: File,
    next_sequence: u64,
    _run_lock: RunLock,
}

struct RunLock {
    _file: File,
}

#[derive(Clone, Copy)]
enum LockMode {
    Shared,
    Exclusive,
}

impl RunLock {
    fn acquire_existing(
        journal: &Path,
        run_id: &str,
        mode: LockMode,
    ) -> Result<Self, JournalError> {
        File::open(journal).map_err(|source| JournalError::io(journal, source))?;

        Self::acquire(journal, run_id, mode)
    }

    fn acquire(journal: &Path, run_id: &str, mode: LockMode) -> Result<Self, JournalError> {
        let path = journal.with_extension("lock");
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|source| JournalError::io(&path, source))?;
        let result = match mode {
            LockMode::Shared => FileExt::try_lock_shared(&file),
            LockMode::Exclusive => FileExt::try_lock(&file),
        };
        match result {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(JournalError::RunBusy(run_id.to_owned())),
            Err(TryLockError::Error(source)) => Err(JournalError::io(path, source)),
        }
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self._file);
    }
}

pub(crate) struct JournalRecorder<'a> {
    store: &'a JournalStore,
    target: PathBuf,
    plan: &'a Plan,
    writer: Option<JournalWriter>,
}

impl<'a> JournalRecorder<'a> {
    pub(crate) fn new(store: &'a JournalStore, target: &Path, plan: &'a Plan) -> Self {
        Self {
            store,
            target: target.to_path_buf(),
            plan,
            writer: None,
        }
    }

    pub(crate) fn id(&self) -> Option<&str> {
        self.writer.as_ref().map(|writer| writer.id.as_str())
    }

    pub(crate) fn finish(&mut self, status: JournalStatus) -> Result<(), JournalError> {
        if let Some(writer) = &mut self.writer {
            writer.status(status)?;
        }
        Ok(())
    }

    fn writer(&mut self) -> Result<&mut JournalWriter, JournalError> {
        if self.writer.is_none() {
            self.writer = Some(self.store.begin(&self.target, self.plan)?);
        }
        Ok(self.writer.as_mut().expect("journal writer initialized"))
    }
}

impl MutationRecorder for JournalRecorder<'_> {
    fn move_intent(
        &mut self,
        object: u64,
        source: &Path,
        destination: &Path,
        kind: EntryKind,
    ) -> Result<Option<u64>, ApplyError> {
        Ok(Some(self.writer()?.move_intent(
            object,
            source,
            destination,
            kind,
        )?))
    }

    fn move_completed(
        &mut self,
        sequence: Option<u64>,
        destination: &Path,
        kind: EntryKind,
    ) -> Result<(), ApplyError> {
        if let Some(writer) = &mut self.writer {
            writer.move_completed(
                sequence.expect("journal move intent returns a sequence"),
                destination,
                kind,
            )?;
        }
        Ok(())
    }

    fn directory_intent(&mut self, directory: &Path) -> Result<Option<u64>, ApplyError> {
        Ok(Some(self.writer()?.directory_intent(directory)?))
    }

    fn directory_created(
        &mut self,
        sequence: Option<u64>,
        directory: &Path,
    ) -> Result<(), ApplyError> {
        if let Some(writer) = &mut self.writer {
            writer.directory_created(
                sequence.expect("journal directory intent returns a sequence"),
                directory,
            )?;
        }
        Ok(())
    }

    fn directory_remove_intent(&mut self, directory: &Path) -> Result<Option<u64>, ApplyError> {
        Ok(Some(self.writer()?.directory_remove_intent(directory)?))
    }

    fn directory_removed(&mut self, sequence: Option<u64>) -> Result<(), ApplyError> {
        if let Some(writer) = &mut self.writer {
            writer.directory_removed(
                sequence.expect("journal directory intent returns a sequence"),
            )?;
        }
        Ok(())
    }
}

impl JournalWriter {
    fn open(
        path: &Path,
        id: &str,
        target: &Path,
        records: &[JournalRecord],
        run_lock: RunLock,
    ) -> Result<Self, JournalError> {
        let next_sequence = records
            .iter()
            .filter_map(|record| match record {
                JournalRecord::MoveIntent { sequence, .. }
                | JournalRecord::DirectoryIntent { sequence, .. } => Some(*sequence),
                _ => None,
            })
            .max()
            .map_or(0, |sequence| sequence + 1);
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(path)
            .map_err(|source| JournalError::io(path, source))?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .map_err(|source| JournalError::io(path, source))?;
        let valid_length = contents
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1) as u64;
        if valid_length != contents.len() as u64 {
            file.set_len(valid_length)
                .and_then(|()| file.sync_all())
                .map_err(|source| JournalError::io(path, source))?;
        }
        Ok(Self {
            id: id.to_owned(),
            target: target.to_path_buf(),
            path: path.to_path_buf(),
            file,
            next_sequence,
            _run_lock: run_lock,
        })
    }

    pub(crate) fn move_intent(
        &mut self,
        object: u64,
        source: &Path,
        destination: &Path,
        kind: EntryKind,
    ) -> Result<u64, JournalError> {
        let source_relative = source
            .strip_prefix(&self.target)
            .map_err(|_| JournalError::TargetPath(source.to_path_buf()))?;
        let destination_relative = destination
            .strip_prefix(&self.target)
            .map_err(|_| JournalError::TargetPath(destination.to_path_buf()))?;
        let metadata =
            fs::symlink_metadata(source).map_err(|error| JournalError::io(source, error))?;
        let sequence = self.sequence();
        write_record(
            &mut self.file,
            &JournalRecord::MoveIntent {
                sequence,
                object,
                source: source_relative.to_path_buf(),
                destination: destination_relative.to_path_buf(),
                identity: ObjectIdentity::from_metadata(&metadata, kind),
            },
            &self.path,
        )?;
        Ok(sequence)
    }

    pub(crate) fn move_completed(
        &mut self,
        sequence: u64,
        destination: &Path,
        kind: EntryKind,
    ) -> Result<(), JournalError> {
        let metadata = fs::symlink_metadata(destination)
            .map_err(|error| JournalError::io(destination, error))?;
        write_record(
            &mut self.file,
            &JournalRecord::MoveCompleted {
                sequence,
                identity: ObjectIdentity::from_metadata(&metadata, kind),
            },
            &self.path,
        )
    }

    pub(crate) fn directory_intent(&mut self, directory: &Path) -> Result<u64, JournalError> {
        self.directory_action_intent(directory, DirectoryAction::Create)
    }

    pub(crate) fn directory_remove_intent(
        &mut self,
        directory: &Path,
    ) -> Result<u64, JournalError> {
        self.directory_action_intent(directory, DirectoryAction::Remove)
    }

    fn directory_action_intent(
        &mut self,
        directory: &Path,
        action: DirectoryAction,
    ) -> Result<u64, JournalError> {
        let relative = directory
            .strip_prefix(&self.target)
            .map_err(|_| JournalError::TargetPath(directory.to_path_buf()))?;
        let identity = if action == DirectoryAction::Remove {
            let metadata = fs::symlink_metadata(directory)
                .map_err(|error| JournalError::io(directory, error))?;
            Some(ObjectIdentity::from_metadata(
                &metadata,
                EntryKind::Directory,
            ))
        } else {
            None
        };
        let sequence = self.sequence();
        write_record(
            &mut self.file,
            &JournalRecord::DirectoryIntent {
                sequence,
                path: relative.to_path_buf(),
                action,
                identity,
            },
            &self.path,
        )?;
        Ok(sequence)
    }

    pub(crate) fn directory_created(
        &mut self,
        sequence: u64,
        directory: &Path,
    ) -> Result<(), JournalError> {
        let metadata =
            fs::symlink_metadata(directory).map_err(|error| JournalError::io(directory, error))?;
        write_record(
            &mut self.file,
            &JournalRecord::DirectoryChanged {
                sequence,
                action: DirectoryAction::Create,
                identity: ObjectIdentity::from_metadata(&metadata, EntryKind::Directory),
            },
            &self.path,
        )
    }

    pub(crate) fn directory_removed(&mut self, sequence: u64) -> Result<(), JournalError> {
        let Some(JournalRecord::DirectoryIntent {
            identity: Some(identity),
            ..
        }) = self.record_for_sequence(sequence)?
        else {
            return Err(JournalError::AmbiguousOperation {
                run_id: self.id.clone(),
                path: self.target.clone(),
            });
        };
        write_record(
            &mut self.file,
            &JournalRecord::DirectoryChanged {
                sequence,
                action: DirectoryAction::Remove,
                identity,
            },
            &self.path,
        )
    }

    fn directory_retained(&mut self, sequence: u64, directory: &Path) -> Result<(), JournalError> {
        let relative = directory
            .strip_prefix(&self.target)
            .map_err(|_| JournalError::TargetPath(directory.to_path_buf()))?;
        let metadata = fs::symlink_metadata(directory)
            .map_err(|source| JournalError::io(directory, source))?;
        write_record(
            &mut self.file,
            &JournalRecord::DirectoryRetained {
                sequence,
                path: relative.to_path_buf(),
                identity: ObjectIdentity::from_metadata(&metadata, EntryKind::Directory),
            },
            &self.path,
        )
    }

    fn record_for_sequence(&self, sequence: u64) -> Result<Option<JournalRecord>, JournalError> {
        let file = File::open(&self.path).map_err(|source| JournalError::io(&self.path, source))?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        let mut line_number = 0;
        loop {
            line.clear();
            if reader
                .read_until(b'\n', &mut line)
                .map_err(|source| JournalError::io(&self.path, source))?
                == 0
            {
                return Ok(None);
            }
            line_number += 1;
            let record = parse_record(&self.path, line_number, &line)?;
            if matches!(record, JournalRecord::DirectoryIntent { sequence: found, .. } if found == sequence)
            {
                return Ok(Some(record));
            }
        }
    }

    pub(crate) fn status(&mut self, status: JournalStatus) -> Result<(), JournalError> {
        write_record(
            &mut self.file,
            &JournalRecord::Status {
                status: status.into(),
            },
            &self.path,
        )
    }

    fn sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }
}

fn read_run(path: &Path) -> Result<JournalRun, JournalError> {
    let records = read_records(path)?;
    let history = JournalHistory::from_records(path, records)?;
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

fn read_records(path: &Path) -> Result<Vec<JournalRecord>, JournalError> {
    let file = File::open(path).map_err(|source| JournalError::io(path, source))?;
    let mut reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut line = Vec::new();
    let mut line_number = 0;
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
        if !line.ends_with(b"\n") {
            if records.is_empty() {
                return Err(JournalError::InvalidRunId(path.display().to_string()));
            }
            break;
        }
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
    records
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            serde_json::from_value(record).map_err(|source| JournalError::InvalidRecord {
                path: path.to_path_buf(),
                line: index + 1,
                source,
            })
        })
        .collect()
}

fn parse_record(path: &Path, line: usize, bytes: &[u8]) -> Result<JournalRecord, JournalError> {
    serde_json::from_value(parse_value(path, line, bytes)?).map_err(|source| {
        JournalError::InvalidRecord {
            path: path.to_path_buf(),
            line,
            source,
        }
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
    serde_json::to_writer(&mut *file, record)
        .map_err(|source| JournalError::io(path, std::io::Error::other(source)))?;
    file.write_all(b"\n")
        .and_then(|()| file.flush())
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

fn set_private_directory(path: &Path) -> Result<(), JournalError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|source| JournalError::io(path, source))?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), JournalError> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| JournalError::io(path, source))?;
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

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[(byte >> 4) as usize]));
        encoded.push(char::from(DIGITS[(byte & 0x0f) as usize]));
    }
    encoded
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() & 1 != 0 {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
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
