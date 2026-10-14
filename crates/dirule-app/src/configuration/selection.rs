use super::ConfigurationSource;
use crate::paths::{HomeDirectoryUnavailable, resolve_target};
use crate::presets::{PresetError, PresetStore};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub enum ConfigurationSelectionError {
    Target(HomeDirectoryUnavailable),
    LocalConfig {
        path: PathBuf,
        source: io::Error,
    },
    Presets(PresetError),
    TargetPath {
        path: PathBuf,
        source: io::Error,
    },
    AmbiguousPresetTarget {
        target: PathBuf,
        presets: Vec<String>,
    },
}

pub fn discover_source(
    base: &Path,
    target: Option<PathBuf>,
    presets: Option<&PresetStore>,
) -> Result<ConfigurationSource, ConfigurationSelectionError> {
    let target = target
        .map(|path| resolve_target(base, path))
        .transpose()
        .map_err(ConfigurationSelectionError::Target)?
        .unwrap_or_else(|| base.to_path_buf());
    let local = ConfigurationSource::project_default(&target);

    if path_is_present(local.path()).map_err(|source| ConfigurationSelectionError::LocalConfig {
        path: local.path().to_path_buf(),
        source,
    })? {
        return Ok(local);
    }

    let Some(presets) = presets else {
        return Ok(local);
    };
    let comparison_target = comparable_target(&target)?;
    let mut matches = Vec::new();

    for name in presets
        .list()
        .map_err(ConfigurationSelectionError::Presets)?
    {
        let configured_target = presets
            .configured_target(&name)
            .map_err(ConfigurationSelectionError::Presets)?;
        let Some(configured_target) = configured_target else {
            continue;
        };
        if !is_absolute_or_home_target(&configured_target) {
            continue;
        }

        let resolved_target =
            resolve_target(base, configured_target).map_err(ConfigurationSelectionError::Target)?;
        if comparable_target(&resolved_target)? == comparison_target {
            matches.push(name);
        }
    }

    match matches.as_slice() {
        [] => Ok(local),
        [name] => {
            ConfigurationSource::preset(presets, name).map_err(ConfigurationSelectionError::Presets)
        }
        _ => Err(ConfigurationSelectionError::AmbiguousPresetTarget {
            target,
            presets: matches,
        }),
    }
}

fn path_is_present(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn is_absolute_or_home_target(path: &Path) -> bool {
    if path.is_absolute() {
        return true;
    }

    let Some(path) = path.to_str() else {
        return false;
    };

    if path == "~" || path.starts_with("~/") {
        return true;
    }

    #[cfg(windows)]
    return path.starts_with("~\\");

    #[cfg(not(windows))]
    false
}

fn comparable_target(path: &Path) -> Result<PathBuf, ConfigurationSelectionError> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(normalize_path(path)),
        Err(source) => Err(ConfigurationSelectionError::TargetPath {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match normalized.components().next_back() {
                Some(Component::Normal(_)) => {
                    normalized.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                Some(Component::CurDir) => {}
                Some(Component::ParentDir) | None if !path.is_absolute() => {
                    normalized.push(component.as_os_str());
                }
                Some(Component::ParentDir) | None => {}
            },
            component => normalized.push(component.as_os_str()),
        }
    }

    normalized
}

impl fmt::Display for ConfigurationSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Target(source) => fmt::Display::fmt(source, formatter),
            Self::LocalConfig { path, source } => {
                write!(
                    formatter,
                    "cannot inspect local configuration {}: {source}",
                    path.display()
                )
            }
            Self::Presets(source) => {
                write!(formatter, "cannot discover a matching preset: {source}")
            }
            Self::TargetPath { path, source } => {
                write!(
                    formatter,
                    "cannot resolve target {}: {source}",
                    path.display()
                )
            }
            Self::AmbiguousPresetTarget { target, presets } => write!(
                formatter,
                "multiple presets target {}: {}; pass `--preset NAME` to choose one",
                target.display(),
                presets.join(", ")
            ),
        }
    }
}

impl Error for ConfigurationSelectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Target(source) => Some(source),
            Self::LocalConfig { source, .. } | Self::TargetPath { source, .. } => Some(source),
            Self::Presets(source) => Some(source),
            Self::AmbiguousPresetTarget { .. } => None,
        }
    }
}
