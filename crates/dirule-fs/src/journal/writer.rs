use crate::apply::{ApplyError, MutationRecorder};
use dirule_core::{EntryKind, Plan};
use fs4::{FileExt, TryLockError};
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::{
    DirectoryAction, JournalError, JournalRecord, JournalStatus, JournalStore, ObjectIdentity,
    write_record,
};

pub(crate) struct JournalWriter {
    pub(crate) id: String,
    pub(crate) target: PathBuf,
    path: PathBuf,
    file: File,
    next_sequence: u64,
    pending_directory_removal: Option<(u64, ObjectIdentity)>,
    _run_lock: RunLock,
}

pub(super) struct RunLock {
    _file: File,
}

#[derive(Clone, Copy)]
pub(super) enum LockMode {
    Shared,
    Exclusive,
}

impl RunLock {
    pub(super) fn acquire_existing(
        journal: &Path,
        run_id: &str,
        mode: LockMode,
    ) -> Result<Self, JournalError> {
        File::open(journal).map_err(|source| JournalError::io(journal, source))?;

        Self::acquire(journal, run_id, mode)
    }

    pub(super) fn acquire(
        journal: &Path,
        run_id: &str,
        mode: LockMode,
    ) -> Result<Self, JournalError> {
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

    fn text_edit_intent(
        &mut self,
        path: &Path,
        before: &str,
        after: &str,
    ) -> Result<Option<u64>, ApplyError> {
        Ok(Some(self.writer()?.text_edit_intent(path, before, after)?))
    }

    fn text_edit_completed(
        &mut self,
        sequence: Option<u64>,
        path: &Path,
    ) -> Result<(), ApplyError> {
        if let Some(writer) = &mut self.writer {
            writer.text_edit_completed(
                sequence.expect("text-edit intent returns a sequence"),
                path,
            )?;
        }
        Ok(())
    }
}

impl JournalWriter {
    pub(super) fn create(
        id: String,
        target: PathBuf,
        path: PathBuf,
        file: File,
        run_lock: RunLock,
    ) -> Self {
        Self {
            id,
            target,
            path,
            file,
            next_sequence: 0,
            pending_directory_removal: None,
            _run_lock: run_lock,
        }
    }

    pub(super) fn open(
        path: &Path,
        id: &str,
        target: &Path,
        records: &[JournalRecord],
        valid_length: u64,
        observed_length: u64,
        run_lock: RunLock,
    ) -> Result<Self, JournalError> {
        let next_sequence = records
            .iter()
            .filter_map(|record| match record {
                JournalRecord::MoveIntent { sequence, .. }
                | JournalRecord::DirectoryIntent { sequence, .. }
                | JournalRecord::TextEditIntent { sequence, .. } => Some(*sequence),
                _ => None,
            })
            .max()
            .map_or(0, |sequence| sequence + 1);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| JournalError::io(path, source))?;
        let file_length = file
            .metadata()
            .map_err(|source| JournalError::io(path, source))?
            .len();
        if file_length != observed_length {
            return Err(JournalError::InvalidEvents {
                path: path.to_path_buf(),
                message: "journal changed while preparing undo".into(),
            });
        }
        if file_length > valid_length {
            file.set_len(valid_length)
                .and_then(|()| file.sync_all())
                .map_err(|source| JournalError::io(path, source))?;
        }
        file.seek(SeekFrom::End(0))
            .map_err(|source| JournalError::io(path, source))?;
        Ok(Self {
            id: id.to_owned(),
            target: target.to_path_buf(),
            path: path.to_path_buf(),
            file,
            next_sequence,
            pending_directory_removal: None,
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
                identity: ObjectIdentity::from_metadata(&metadata, kind, source),
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
                identity: ObjectIdentity::from_metadata(&metadata, kind, destination),
            },
            &self.path,
        )
    }

    pub(crate) fn text_edit_intent(
        &mut self,
        path: &Path,
        before: &str,
        after: &str,
    ) -> Result<u64, JournalError> {
        let relative = path
            .strip_prefix(&self.target)
            .map_err(|_| JournalError::TargetPath(path.to_path_buf()))?;
        let sequence = self.sequence();
        write_record(
            &mut self.file,
            &JournalRecord::TextEditIntent {
                sequence,
                path: relative.to_path_buf(),
                before: before.to_owned(),
                after: after.to_owned(),
            },
            &self.path,
        )?;
        Ok(sequence)
    }

    pub(crate) fn text_edit_completed(
        &mut self,
        sequence: u64,
        path: &Path,
    ) -> Result<(), JournalError> {
        let metadata = fs::symlink_metadata(path).map_err(|error| JournalError::io(path, error))?;
        let identity = ObjectIdentity::from_metadata(&metadata, EntryKind::File, path);
        write_record(
            &mut self.file,
            &JournalRecord::TextEditCompleted {
                sequence,
                identity: Some(identity),
            },
            &self.path,
        )
    }

    pub(crate) fn text_edit_undone(
        &mut self,
        sequence: u64,
        path: &Path,
    ) -> Result<(), JournalError> {
        let metadata = fs::symlink_metadata(path).map_err(|error| JournalError::io(path, error))?;
        let identity = ObjectIdentity::from_metadata(&metadata, EntryKind::File, path);
        write_record(
            &mut self.file,
            &JournalRecord::TextEditUndone {
                sequence,
                identity: Some(identity),
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
                directory,
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
                identity: identity.clone(),
            },
            &self.path,
        )?;
        if let Some(identity) = identity {
            self.pending_directory_removal = Some((sequence, identity));
        }
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
                identity: ObjectIdentity::from_metadata(&metadata, EntryKind::Directory, directory),
            },
            &self.path,
        )
    }

    pub(crate) fn directory_removed(&mut self, sequence: u64) -> Result<(), JournalError> {
        let Some((pending_sequence, identity)) = &self.pending_directory_removal else {
            return Err(JournalError::AmbiguousOperation {
                run_id: self.id.clone(),
                path: self.target.clone(),
            });
        };
        if *pending_sequence != sequence {
            return Err(JournalError::AmbiguousOperation {
                run_id: self.id.clone(),
                path: self.target.clone(),
            });
        }
        write_record(
            &mut self.file,
            &JournalRecord::DirectoryChanged {
                sequence,
                action: DirectoryAction::Remove,
                identity: identity.clone(),
            },
            &self.path,
        )?;
        self.pending_directory_removal = None;
        Ok(())
    }

    pub(super) fn directory_retained(
        &mut self,
        sequence: u64,
        directory: &Path,
    ) -> Result<(), JournalError> {
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
                identity: ObjectIdentity::from_metadata(&metadata, EntryKind::Directory, directory),
            },
            &self.path,
        )?;
        if self
            .pending_directory_removal
            .as_ref()
            .is_some_and(|(pending_sequence, _)| *pending_sequence == sequence)
        {
            self.pending_directory_removal = None;
        }
        Ok(())
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
