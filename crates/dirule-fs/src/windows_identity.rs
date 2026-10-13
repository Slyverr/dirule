use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ExtendedFileId {
    volume_serial_number: u64,
    file_id: [u8; 16],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileIdentity {
    pub(crate) volume_serial_number: Option<u32>,
    pub(crate) file_index: Option<u64>,
    pub(crate) extended_file_id: Option<ExtendedFileId>,
}

impl FileIdentity {
    pub(crate) fn matches(
        self,
        volume_serial_number: Option<u32>,
        file_index: Option<u64>,
        extended_file_id: Option<ExtendedFileId>,
    ) -> bool {
        if let Some(expected) = extended_file_id {
            return self.extended_file_id == Some(expected);
        }

        self.volume_serial_number.is_some()
            && self.file_index.is_some()
            && self.volume_serial_number == volume_serial_number
            && self.file_index == file_index
    }
}

pub(crate) fn get(path: &Path) -> Option<FileIdentity> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;

    let mut extended = FILE_ID_INFO::default();
    let has_extended = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut extended as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        ) != 0
    };

    if has_extended {
        return Some(FileIdentity {
            volume_serial_number: None,
            file_index: None,
            extended_file_id: Some(ExtendedFileId {
                volume_serial_number: extended.VolumeSerialNumber,
                file_id: extended.FileId.Identifier,
            }),
        });
    }

    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let has_legacy = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) != 0 };

    let file_index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);

    if !has_legacy {
        return None;
    }

    Some(FileIdentity {
        volume_serial_number: has_legacy.then_some(info.dwVolumeSerialNumber),
        file_index: has_legacy.then_some(file_index),
        extended_file_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{ExtendedFileId, FileIdentity};

    #[test]
    fn extended_identity_does_not_fall_back_to_a_legacy_match() {
        let expected = ExtendedFileId {
            volume_serial_number: 7,
            file_id: [1; 16],
        };
        let actual = FileIdentity {
            volume_serial_number: Some(7),
            file_index: Some(42),
            extended_file_id: None,
        };

        assert!(!actual.matches(Some(7), Some(42), Some(expected)));
    }

    #[test]
    fn extended_identity_matches_the_complete_volume_and_file_id() {
        let expected = ExtendedFileId {
            volume_serial_number: 7,
            file_id: [1; 16],
        };
        let actual = FileIdentity {
            volume_serial_number: None,
            file_index: None,
            extended_file_id: Some(expected),
        };

        assert!(actual.matches(None, None, Some(expected)));
        assert!(!actual.matches(
            None,
            None,
            Some(ExtendedFileId {
                volume_serial_number: 8,
                file_id: [1; 16],
            })
        ));
        assert!(!actual.matches(
            None,
            None,
            Some(ExtendedFileId {
                volume_serial_number: 7,
                file_id: [2; 16],
            })
        ));
    }

    #[test]
    fn legacy_identity_matches_when_no_extended_identity_was_recorded() {
        let actual = FileIdentity {
            volume_serial_number: Some(7),
            file_index: Some(42),
            extended_file_id: None,
        };

        assert!(actual.matches(Some(7), Some(42), None));
        assert!(!actual.matches(Some(7), Some(43), None));
    }
}
