use super::ApplyError;
use dirule_core::{EntryKind, Plan};
use std::path::{Component, Path, PathBuf};

pub(super) fn ensure_relative(path: &Path) -> Result<(), ApplyError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ApplyError::UnsafePath(path.to_path_buf()));
    }

    #[cfg(windows)]
    if path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_none_or(invalid_windows_component)
    }) {
        return Err(ApplyError::UnsafePath(path.to_path_buf()));
    }

    Ok(())
}

#[cfg(windows)]
fn invalid_windows_component(value: &str) -> bool {
    if value.ends_with(' ') || value.ends_with('.') {
        return true;
    }

    if value.chars().any(|character| {
        character <= '\u{001f}'
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    }) {
        return true;
    }

    let device_name = value
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }

    ["COM", "LPT"].iter().any(|prefix| {
        device_name
            .strip_prefix(prefix)
            .and_then(|suffix| {
                let mut characters = suffix.chars();
                let digit = characters.next()?;
                (characters.next().is_none() && matches!(digit, '1'..='9' | '¹' | '²' | '³'))
                    .then_some(())
            })
            .is_some()
    })
}

pub(super) fn ensure_parent_safe(root: &Path, path: &Path) -> Result<(), ApplyError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| ApplyError::UnsafePath(path.to_path_buf()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ApplyError::UnsafePath(current));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(ApplyError::ExistingDestination(current));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(ApplyError::Io {
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

pub(super) fn destination_exists(path: &Path) -> Result<bool, ApplyError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ApplyError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub(super) fn nearest_existing_directory(
    root: &Path,
    destination: &Path,
) -> Result<PathBuf, ApplyError> {
    let mut current = destination.parent().unwrap_or(root).to_path_buf();
    loop {
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                return Ok(current);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ApplyError::Io {
                    path: current,
                    source,
                });
            }
        }
        if current == root || !current.pop() {
            return Err(ApplyError::UnsafePath(destination.to_path_buf()));
        }
    }
}

pub(super) fn will_be_vacated(target: &Path, path: &Path, plan: &Plan) -> bool {
    let Ok(relative) = path.strip_prefix(target) else {
        return false;
    };

    plan.entries.iter().any(|entry| {
        entry.source != entry.destination
            && (entry.source == relative
                || (entry.kind == EntryKind::Directory && relative.starts_with(&entry.source))
                || same_existing_path(&target.join(&entry.source), path))
    })
}

pub(super) fn same_existing_path(left: &Path, right: &Path) -> bool {
    let Ok(left_metadata) = std::fs::symlink_metadata(left) else {
        return false;
    };
    let Ok(right_metadata) = std::fs::symlink_metadata(right) else {
        return false;
    };
    if left_metadata.file_type().is_symlink() || right_metadata.file_type().is_symlink() {
        return false;
    }

    matches!(
        (std::fs::canonicalize(left), std::fs::canonicalize(right)),
        (Ok(left), Ok(right)) if left == right
    )
}
