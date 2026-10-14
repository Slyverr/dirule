use super::{ScanError, validate_ignore_file};
use dirule_core::{Finding, FindingKind};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub(super) fn tracked_paths(target: &Path) -> Result<(BTreeSet<PathBuf>, Vec<Finding>), ScanError> {
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

    let (root, exclude) = git_paths_from_output(&root.stdout).map_or_else(
        || {
            Ok((
                rev_parse_path(target, &["--show-toplevel"], "worktree")?,
                rev_parse_path(target, &["--git-path", "info/exclude"], "exclude")?,
            ))
        },
        Ok,
    )?;
    let exclude = if exclude.is_absolute() {
        exclude
    } else {
        target.join(exclude)
    };
    validate_ignore_file(&exclude)?;

    let root = root.canonicalize().map_err(|source| ScanError::Metadata {
        path: root.clone(),
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
            findings.push(Finding::new(
                FindingKind::SkippedGitNonUnicodePath,
                None,
                format!("skipped Git-tracked path with non-Unicode bytes: {bytes:?}"),
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

fn git_path_from_output(output: &[u8], kind: &str) -> Result<PathBuf, ScanError> {
    let output = output.strip_suffix(b"\n").unwrap_or(output);
    git_path_from_bytes(output, kind)
}

fn git_path_from_bytes(output: &[u8], kind: &str) -> Result<PathBuf, ScanError> {
    #[cfg(windows)]
    let output = output.strip_suffix(b"\r").unwrap_or(output);

    let path = std::str::from_utf8(output).map_err(|error| {
        ScanError::Git(format!("Git returned a non-Unicode {kind} path: {error}"))
    })?;
    if path.is_empty() {
        return Err(ScanError::Git(format!("Git did not return a {kind} path")));
    }

    Ok(PathBuf::from(path))
}

fn git_paths_from_output(output: &[u8]) -> Option<(PathBuf, PathBuf)> {
    let mut paths = output.split(|byte| *byte == b'\n');
    let root = paths.next()?;
    let exclude = paths.next()?;
    if paths.next()? != b"" || paths.next().is_some() {
        return None;
    }

    Some((
        git_path_from_bytes(root, "worktree").ok()?,
        git_path_from_bytes(exclude, "exclude").ok()?,
    ))
}

fn rev_parse_path(target: &Path, arguments: &[&str], kind: &str) -> Result<PathBuf, ScanError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(target)
        .arg("rev-parse")
        .args(arguments)
        .output()
        .map_err(|error| ScanError::Git(format!("failed to find the Git {kind} path: {error}")))?;
    if !output.status.success() {
        return Err(ScanError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }

    git_path_from_output(&output.stdout, kind)
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
