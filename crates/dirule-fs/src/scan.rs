use dirule_core::{Entry, EntryKind, Finding, FindingKind};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use thiserror::Error;

#[path = "scan/git.rs"]
mod git;

#[derive(Clone, Debug, Default)]
pub struct ScanResult {
    pub entries: Vec<Entry>,
    pub findings: Vec<Finding>,
    pub(crate) target_identity: Option<SourceIdentity>,
    pub(crate) identities: HashMap<PathBuf, SourceIdentity>,
}

struct ScannedEntry {
    kind: EntryKind,
    identity: Option<SourceIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceIdentity {
    length: u64,
    modified: Option<SystemTime>,
    kind: EntryKind,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    volume_serial_number: Option<u32>,
    #[cfg(windows)]
    file_index: Option<u64>,
    #[cfg(windows)]
    extended_file_id: Option<crate::windows_identity::ExtendedFileId>,
}

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("cannot scan target {path}: {source}")]
    Walk {
        path: PathBuf,
        #[source]
        source: ignore::Error,
    },
    #[error("cannot inspect {path}: {source}")]
    Metadata {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot read ignore file {path}: {source}")]
    IgnoreFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot read tracked paths from Git: {0}")]
    Git(String),
    #[error("target is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("refusing to scan Git metadata: {0}")]
    GitMetadataTarget(PathBuf),
}

pub fn scan(target: &Path) -> Result<ScanResult, ScanError> {
    let target = target
        .canonicalize()
        .map_err(|source| ScanError::Metadata {
            path: target.to_path_buf(),
            source,
        })?;
    if target
        .components()
        .any(|component| component.as_os_str() == OsStr::new(".git"))
    {
        return Err(ScanError::GitMetadataTarget(target));
    }

    if !target.is_dir() {
        return Err(ScanError::NotDirectory(target));
    }
    let target_metadata =
        std::fs::symlink_metadata(&target).map_err(|source| ScanError::Metadata {
            path: target.clone(),
            source,
        })?;
    let target_identity =
        SourceIdentity::from_metadata(&target_metadata, EntryKind::Directory, &target);
    if let (_, Some(source)) = ignore::gitignore::Gitignore::global() {
        return Err(ScanError::Walk {
            path: target.clone(),
            source,
        });
    }

    for directory in target.ancestors() {
        validate_ignore_files(directory)?;
    }

    let mut builder = ignore::WalkBuilder::new(&target);
    builder
        .hidden(false)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| entry.depth() == 0 || entry.file_name() != OsStr::new(".git"));

    let mut paths = HashMap::<PathBuf, ScannedEntry>::new();
    let mut findings = Vec::new();
    for item in builder.build() {
        let entry = item.map_err(|source| ScanError::Walk {
            path: target.clone(),
            source,
        })?;
        if let Some(source) = entry.error() {
            return Err(ScanError::Walk {
                path: entry.path().to_path_buf(),
                source: source.clone(),
            });
        }

        if entry.path() == target {
            continue;
        }

        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            validate_ignore_files(entry.path())?;
        }

        if let Some(relative) = relative_unicode(&target, entry.path()) {
            let metadata = entry.metadata().map_err(|source| ScanError::Walk {
                path: entry.path().to_path_buf(),
                source,
            })?;
            if let Some(kind) = entry_kind(Some(metadata.file_type())) {
                paths.insert(
                    relative,
                    ScannedEntry {
                        kind,
                        identity: Some(SourceIdentity::from_metadata(
                            &metadata,
                            kind,
                            entry.path(),
                        )),
                    },
                );
            } else {
                findings.push(Finding::new(
                    FindingKind::SkippedNonRegularEntry,
                    Some(&relative),
                    format!(
                        "skipped non-regular filesystem entry: {}",
                        relative.display()
                    ),
                ));
            }
        } else if let Ok(relative) = entry.path().strip_prefix(&target) {
            findings.push(Finding::new(
                FindingKind::SkippedNonUnicodePath,
                None,
                format!(
                    "skipped path that cannot be represented as Unicode: {}",
                    relative.display()
                ),
            ));
        }
    }

    let (tracked, tracked_findings) = git::tracked_paths(&target)?;
    findings.extend(tracked_findings);
    for tracked in tracked {
        if !paths.contains_key(&tracked) {
            let absolute = target.join(&tracked);
            let metadata = match std::fs::symlink_metadata(&absolute) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(ScanError::Metadata {
                        path: absolute,
                        source,
                    });
                }
            };
            let Some(kind) = entry_kind(Some(metadata.file_type())) else {
                findings.push(Finding::new(
                    FindingKind::SkippedNonRegularEntry,
                    Some(&tracked),
                    format!(
                        "skipped non-regular filesystem entry: {}",
                        tracked.display()
                    ),
                ));
                continue;
            };
            paths.insert(
                tracked.clone(),
                ScannedEntry {
                    kind,
                    identity: Some(SourceIdentity::from_metadata(&metadata, kind, &absolute)),
                },
            );
        }

        let mut parent = tracked.parent();
        while let Some(path) = parent {
            if !path.as_os_str().is_empty() {
                paths.entry(path.to_path_buf()).or_insert(ScannedEntry {
                    kind: EntryKind::Directory,
                    identity: None,
                });
            }
            parent = path.parent();
        }
    }

    for (path, entry) in &mut paths {
        if entry.identity.is_some() {
            continue;
        }

        let absolute = target.join(path);
        let metadata =
            std::fs::symlink_metadata(&absolute).map_err(|source| ScanError::Metadata {
                path: absolute.clone(),
                source,
            })?;
        entry.identity = Some(SourceIdentity::from_metadata(
            &metadata, entry.kind, &absolute,
        ));
    }

    let mut identities = HashMap::with_capacity(paths.len());
    let mut entries = Vec::with_capacity(paths.len());
    for (path, entry) in paths {
        identities.insert(
            path.clone(),
            entry
                .identity
                .expect("scanned entries have identities before returning"),
        );
        entries.push(Entry {
            path,
            kind: entry.kind,
        });
    }
    entries.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    findings.sort_unstable_by(|left, right| left.message.cmp(&right.message));

    Ok(ScanResult {
        entries,
        findings,
        target_identity: Some(target_identity),
        identities,
    })
}

fn validate_ignore_files(directory: &Path) -> Result<(), ScanError> {
    for name in [".ignore", ".gitignore"] {
        validate_ignore_file(&directory.join(name))?;
    }

    Ok(())
}

fn validate_ignore_file(path: &Path) -> Result<(), ScanError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(ScanError::IgnoreFile {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if !metadata.is_file() {
        return Err(ScanError::IgnoreFile {
            path: path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "ignore rules must be a regular file",
            ),
        });
    }

    let mut file = std::fs::File::open(path).map_err(|source| ScanError::IgnoreFile {
        path: path.to_path_buf(),
        source,
    })?;
    std::io::copy(&mut file, &mut std::io::sink()).map_err(|source| ScanError::IgnoreFile {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(())
}

impl SourceIdentity {
    fn from_metadata(metadata: &std::fs::Metadata, kind: EntryKind, path: &Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let _ = path;
            Self {
                length: metadata.len(),
                modified: metadata.modified().ok(),
                kind,
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }
        #[cfg(windows)]
        {
            let identity = crate::windows_identity::get(path);
            Self {
                length: metadata.len(),
                modified: metadata.modified().ok(),
                kind,
                volume_serial_number: identity.and_then(|value| value.volume_serial_number),
                file_index: identity.and_then(|value| value.file_index),
                extended_file_id: identity.and_then(|value| value.extended_file_id),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            Self {
                length: metadata.len(),
                modified: metadata.modified().ok(),
                kind,
            }
        }
    }

    pub(crate) fn matches(
        &self,
        metadata: &std::fs::Metadata,
        kind: EntryKind,
        path: &Path,
    ) -> bool {
        #[cfg(windows)]
        if kind == EntryKind::Directory {
            return self.matches_object(metadata, kind, path);
        }

        let common = self.length == metadata.len()
            && self.modified == metadata.modified().ok()
            && self.kind == kind;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let _ = path;
            common && self.device == metadata.dev() && self.inode == metadata.ino()
        }
        #[cfg(windows)]
        {
            common
                && crate::windows_identity::get(path).is_some_and(|identity| {
                    identity.matches(
                        self.volume_serial_number,
                        self.file_index,
                        self.extended_file_id,
                    )
                })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            common
        }
    }

    pub(crate) fn matches_object(
        &self,
        metadata: &std::fs::Metadata,
        kind: EntryKind,
        path: &Path,
    ) -> bool {
        if self.kind != kind {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let _ = path;
            self.device == metadata.dev() && self.inode == metadata.ino()
        }
        #[cfg(windows)]
        {
            let _ = metadata;
            crate::windows_identity::get(path).is_some_and(|identity| {
                identity.matches(
                    self.volume_serial_number,
                    self.file_index,
                    self.extended_file_id,
                )
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            self.matches(metadata, kind, path)
        }
    }
}

fn relative_unicode(root: &Path, path: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(root).ok()?;
    if relative
        .components()
        .any(|component| component.as_os_str().to_str().is_none())
    {
        return None;
    }
    Some(relative.to_path_buf())
}

fn entry_kind(file_type: Option<std::fs::FileType>) -> Option<EntryKind> {
    match file_type {
        Some(file_type) if file_type.is_symlink() => Some(EntryKind::Symlink),
        Some(file_type) if file_type.is_dir() => Some(EntryKind::Directory),
        Some(file_type) if file_type.is_file() => Some(EntryKind::File),
        _ => None,
    }
}
