use dirule_core::{Entry, EntryKind};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use thiserror::Error;

#[derive(Clone, Debug, Default)]
pub struct ScanResult {
    pub entries: Vec<Entry>,
    pub findings: Vec<String>,
    pub(crate) target_identity: Option<SourceIdentity>,
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
    #[cfg(windows)]
    volume_serial_number: Option<u32>,
    #[cfg(windows)]
    file_index: Option<u64>,
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
        .follow_links(false);

    let mut paths = BTreeMap::<PathBuf, EntryKind>::new();
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
    let mut identities = BTreeMap::new();
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
                findings.push(format!(
                    "skipped non-regular filesystem entry: {}",
                    tracked.display()
                ));
                continue;
            };
            identities.insert(
                tracked.clone(),
                SourceIdentity::from_metadata(&metadata, kind, &absolute),
            );
            paths.insert(tracked.clone(), kind);
        }

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
    for entry in &entries {
        if identities.contains_key(&entry.path) {
            continue;
        }

        let absolute = target.join(&entry.path);
        let metadata =
            std::fs::symlink_metadata(&absolute).map_err(|source| ScanError::Metadata {
                path: absolute.clone(),
                source,
            })?;
        identities.insert(
            entry.path.clone(),
            SourceIdentity::from_metadata(&metadata, entry.kind, &absolute),
        );
    }

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
            let (volume_serial_number, file_index) = crate::windows_identity::get(path)
                .map_or((None, None), |(volume, index)| (Some(volume), Some(index)));
            Self {
                length: metadata.len(),
                modified: metadata.modified().ok(),
                kind,
                volume_serial_number,
                file_index,
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
                && crate::windows_identity::get(path).is_some_and(|(volume, index)| {
                    self.volume_serial_number == Some(volume) && self.file_index == Some(index)
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
            match crate::windows_identity::get(path) {
                Some((volume, index)) => {
                    self.volume_serial_number == Some(volume) && self.file_index == Some(index)
                }
                None => self.matches(metadata, kind, path),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            self.matches(metadata, kind, path)
        }
    }
}

fn tracked_paths(target: &Path) -> Result<(BTreeSet<PathBuf>, Vec<String>), ScanError> {
    let root = match Command::new("git")
        .arg("-C")
        .arg(target)
        .args(["rev-parse", "--show-toplevel", "--git-path", "info/exclude"])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if has_git_marker(target) {
                return Err(ScanError::Git(
                    "Git is required to include tracked paths from this repository".into(),
                ));
            }
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

    let paths = std::str::from_utf8(&root.stdout)
        .map_err(|error| ScanError::Git(format!("Git returned a non-Unicode path: {error}")))?;
    let mut paths = paths.lines();
    let root_path = paths
        .next()
        .ok_or_else(|| ScanError::Git("Git did not return a worktree path".into()))?;
    let exclude = paths
        .next()
        .ok_or_else(|| ScanError::Git("Git did not return an exclude path".into()))?;
    let exclude = PathBuf::from(exclude);
    let exclude = if exclude.is_absolute() {
        exclude
    } else {
        target.join(exclude)
    };
    validate_ignore_file(&exclude)?;

    let root = PathBuf::from(root_path)
        .canonicalize()
        .map_err(|source| ScanError::Metadata {
            path: PathBuf::from(root_path),
            source,
        })?;
    let target_from_root = target.strip_prefix(&root).map_err(|error| {
        ScanError::Git(format!(
            "target is outside its reported Git worktree: {error}"
        ))
    })?;
    let mut command = Command::new("git");
    command
        .args(["--literal-pathspecs", "-C"])
        .arg(&root)
        .args(["ls-files", "-z"]);
    if let Some(pathspec) = literal_pathspec(target_from_root) {
        command.arg("--").arg(pathspec);
    }
    let output = command
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

fn has_git_marker(target: &Path) -> bool {
    target
        .ancestors()
        .any(|ancestor| std::fs::symlink_metadata(ancestor.join(".git")).is_ok())
}

fn literal_pathspec(path: &Path) -> Option<OsString> {
    let mut pathspec = OsString::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return None;
        };
        if !pathspec.is_empty() {
            pathspec.push("/");
        }
        pathspec.push(component);
    }
    (!pathspec.is_empty()).then_some(pathspec)
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
