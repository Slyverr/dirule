use crate::paths::resolve_from;
use crate::presets::{PresetError, PresetStore};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// The kind of configuration selected for an operation.
pub enum ConfigurationSourceKind {
    ProjectDefault,
    ExplicitFile,
    Preset,
}

impl ConfigurationSourceKind {
    /// Return the stable CLI-facing name for this source kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectDefault | Self::ExplicitFile => "config",
            Self::Preset => "preset",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A configuration file selected for loading and display.
pub struct ConfigurationSource {
    path: PathBuf,
    name: String,
    kind: ConfigurationSourceKind,
}

impl ConfigurationSource {
    /// Select `dirule.toml` directly inside a target directory.
    pub fn project_default(directory: &Path) -> Self {
        Self {
            path: directory.join("dirule.toml"),
            name: "dirule.toml".into(),
            kind: ConfigurationSourceKind::ProjectDefault,
        }
    }

    /// Select an explicit config path, resolving relative paths from `directory`.
    pub fn explicit_file(directory: &Path, path: PathBuf) -> Self {
        let name = path.display().to_string();

        Self {
            path: resolve_from(directory, path),
            name,
            kind: ConfigurationSourceKind::ExplicitFile,
        }
    }

    /// Select a named user preset from `store`.
    pub fn preset(store: &PresetStore, name: &str) -> Result<Self, PresetError> {
        Ok(Self {
            path: store.path(name)?,
            name: name.into(),
            kind: ConfigurationSourceKind::Preset,
        })
    }

    /// Return the path to the selected TOML file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the display name for the selected source.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return whether this source is a project default, explicit file, or preset.
    pub fn kind(&self) -> ConfigurationSourceKind {
        self.kind
    }
}
