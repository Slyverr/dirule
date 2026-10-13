use super::JournalError;
#[cfg(unix)]
use super::{decode_hex, encode_hex};
use dirule_core::EntryKind;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "encoding", content = "value", rename_all = "snake_case")]
pub(super) enum EncodedPath {
    Utf8(String),
    #[cfg(unix)]
    UnixBytes(String),
    #[cfg(windows)]
    WindowsUtf16(Vec<u16>),
}

impl EncodedPath {
    pub(super) fn from_path(path: &Path) -> Result<Self, JournalError> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Ok(Self::UnixBytes(encode_hex(path.as_os_str().as_bytes())))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Ok(Self::WindowsUtf16(path.as_os_str().encode_wide().collect()))
        }
        #[cfg(not(any(unix, windows)))]
        {
            path.to_str()
                .map(|value| Self::Utf8(value.to_owned()))
                .ok_or_else(|| JournalError::TargetPath(path.to_path_buf()))
        }
    }

    pub(super) fn to_path(&self) -> Result<PathBuf, JournalError> {
        match self {
            Self::Utf8(value) => Ok(PathBuf::from(value)),
            #[cfg(unix)]
            Self::UnixBytes(value) => {
                use std::os::unix::ffi::OsStringExt;
                let bytes = decode_hex(value)
                    .ok_or_else(|| JournalError::TargetPath(PathBuf::from(value)))?;
                Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
            }
            #[cfg(windows)]
            Self::WindowsUtf16(value) => {
                use std::os::windows::ffi::OsStringExt;
                Ok(PathBuf::from(std::ffi::OsString::from_wide(value)))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct ObjectIdentity {
    pub(super) kind: EntryKind,
    length: u64,
    modified_seconds: Option<u64>,
    modified_nanos: Option<u32>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    volume_serial_number: Option<u32>,
    #[cfg(windows)]
    file_index: Option<u64>,
}

impl ObjectIdentity {
    pub(super) fn from_kind(kind: EntryKind) -> Self {
        Self {
            kind,
            length: 0,
            modified_seconds: None,
            modified_nanos: None,
            #[cfg(unix)]
            device: 0,
            #[cfg(unix)]
            inode: 0,
            #[cfg(windows)]
            volume_serial_number: None,
            #[cfg(windows)]
            file_index: None,
        }
    }

    pub(super) fn from_metadata(metadata: &fs::Metadata, kind: EntryKind, path: &Path) -> Self {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let _ = path;
            Self {
                kind,
                length: metadata.len(),
                modified_seconds: modified.map(|time| time.as_secs()),
                modified_nanos: modified.map(|time| time.subsec_nanos()),
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }
        #[cfg(windows)]
        {
            let (volume_serial_number, file_index) = crate::windows_identity::get(path)
                .map_or((None, None), |(volume, index)| (Some(volume), Some(index)));
            Self {
                kind,
                length: metadata.len(),
                modified_seconds: modified.map(|time| time.as_secs()),
                modified_nanos: modified.map(|time| time.subsec_nanos()),
                volume_serial_number,
                file_index,
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            Self {
                kind,
                length: metadata.len(),
                modified_seconds: modified.map(|time| time.as_secs()),
                modified_nanos: modified.map(|time| time.subsec_nanos()),
            }
        }
    }

    pub(super) fn matches(&self, metadata: &fs::Metadata, kind: EntryKind, path: &Path) -> bool {
        if self.kind != kind {
            return false;
        }
        if kind == EntryKind::Directory {
            return self.matches_object_id(metadata, path);
        }

        let modified = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok());
        self.length == metadata.len()
            && self.modified_seconds == modified.map(|time| time.as_secs())
            && self.modified_nanos == modified.map(|time| time.subsec_nanos())
            && self.matches_object_id(metadata, path)
    }

    fn matches_object_id(&self, metadata: &fs::Metadata, path: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let _ = path;
            self.inode != 0 && self.device == metadata.dev() && self.inode == metadata.ino()
        }
        #[cfg(windows)]
        {
            let _ = metadata;
            crate::windows_identity::get(path).is_some_and(|(volume, index)| {
                self.volume_serial_number == Some(volume) && self.file_index == Some(index)
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (metadata, path);
            false
        }
    }
}
