use dirule_core::{Entry, EntryKind};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use thiserror::Error;

#[derive(Clone, Debug, Default)]
pub struct ScanResult {
    pub entries: Vec<Entry>,
    pub findings: Vec<String>,
    pub(crate) identities: BTreeMap<PathBuf, SourceIdentity>,
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
    #[error("cannot read tracked paths from Git: {0}")]
    Git(String),
    #[error("target is not a directory: {0}")]
    NotDirectory(PathBuf),
}

pub fn scan(target: &Path) -> Result<ScanResult, ScanError> {
    let target = target
        .canonicalize()
        .map_err(|source| ScanError::Metadata {
            path: target.to_path_buf(),
            source,
        })?;
    if !target.is_dir() {
        return Err(ScanError::NotDirectory(target));
    }
    if let (_, Some(source)) = ignore::gitignore::Gitignore::global() {
        return Err(ScanError::Walk {
            path: target.clone(),
            source,
        });
    }

    let mut paths = BTreeMap::<PathBuf, EntryKind>::new();
    let mut findings = Vec::new();
    for item in ignore::WalkBuilder::new(&target)
        .hidden(false)
        .ignore(true)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .build()
    {
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
        if let Some(relative) = relative_unicode(&target, entry.path()) {
            if let Some(kind) = entry_kind(entry.file_type()) {
                paths.insert(relative, kind);
            } else {
                findings.push(format!(
                    "skipped non-regular filesystem entry: {}",
                    relative.display()
                ));
            }
        } else if let Ok(relative) = entry.path().strip_prefix(&target) {
            findings.push(format!(
                "skipped path that cannot be represented as Unicode: {}",
                relative.display()
            ));
        }
    }

    let (tracked, tracked_findings) = tracked_paths(&target)?;
    findings.extend(tracked_findings);
    for tracked in tracked {
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
            findings.push(format!(
                "skipped non-regular filesystem entry: {}",
                tracked.display()
            ));
            continue;
        };
        paths.insert(tracked.clone(), kind);
        let mut parent = tracked.parent();
        while let Some(path) = parent {
            if !path.as_os_str().is_empty() {
                paths
                    .entry(path.to_path_buf())
                    .or_insert(EntryKind::Directory);
            }
            parent = path.parent();
        }
    }

    let entries = paths
        .into_iter()
        .map(|(path, kind)| Entry { path, kind })
        .collect::<Vec<_>>();
    let mut identities = BTreeMap::new();
    for entry in &entries {
        let absolute = target.join(&entry.path);
        let metadata =
            std::fs::symlink_metadata(&absolute).map_err(|source| ScanError::Metadata {
                path: absolute,
                source,
            })?;
        identities.insert(
            entry.path.clone(),
            SourceIdentity::from_metadata(&metadata, entry.kind),
        );
    }

    Ok(ScanResult {
        entries,
        findings,
        identities,
    })
}

impl SourceIdentity {
    fn from_metadata(metadata: &std::fs::Metadata, kind: EntryKind) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                length: metadata.len(),
                modified: metadata.modified().ok(),
                kind,
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                length: metadata.len(),
                modified: metadata.modified().ok(),
                kind,
            }
        }
    }

    pub(crate) fn matches(&self, metadata: &std::fs::Metadata, kind: EntryKind) -> bool {
        let common = self.length == metadata.len()
            && self.modified == metadata.modified().ok()
            && self.kind == kind;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            common && self.device == metadata.dev() && self.inode == metadata.ino()
        }
        #[cfg(not(unix))]
        {
            common
        }
    }
}

fn tracked_paths(target: &Path) -> Result<(BTreeSet<PathBuf>, Vec<String>), ScanError> {
    let root = match Command::new("git")
        .arg("-C")
        .arg(target)
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((BTreeSet::new(), Vec::new()));
        }
        Err(error) => {
            return Err(ScanError::Git(format!(
                "failed to find the Git worktree: {error}"
            )));
        }
    };
    if !root.status.success() {
        return Ok((BTreeSet::new(), Vec::new()));
    }

    let root = std::str::from_utf8(&root.stdout)
        .map_err(|error| {
            ScanError::Git(format!("Git returned a non-Unicode worktree path: {error}"))
        })?
        .trim();
    let root = PathBuf::from(root)
        .canonicalize()
        .map_err(|source| ScanError::Metadata {
            path: PathBuf::from(root),
            source,
        })?;
    let target_from_root = target.strip_prefix(&root).map_err(|error| {
        ScanError::Git(format!(
            "target is outside its reported Git worktree: {error}"
        ))
    })?;
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|error| ScanError::Git(format!("failed to run git ls-files: {error}")))?;
    if !output.status.success() {
        return Err(ScanError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let mut paths = BTreeSet::new();
    let mut findings = Vec::new();
    for bytes in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
    {
        let Ok(path) = std::str::from_utf8(bytes) else {
            findings.push(format!(
                "skipped Git-tracked path with non-Unicode bytes: {bytes:?}"
            ));
            continue;
        };
        let path = PathBuf::from(path);
        if let Ok(relative) = path.strip_prefix(target_from_root) {
            paths.insert(relative.to_path_buf());
        }
    }
    Ok((paths, findings))
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
