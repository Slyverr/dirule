use crate::application::paths::resolve_from;
use crate::application::presets::{PresetError, PresetStore};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationSourceKind {
    ProjectDefault,
    ExplicitFile,
    Preset,
}

impl ConfigurationSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectDefault | Self::ExplicitFile => "config",
            Self::Preset => "preset",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationSource {
    path: PathBuf,
    name: String,
    kind: ConfigurationSourceKind,
}

impl ConfigurationSource {
    pub fn project_default(directory: &Path) -> Self {
        Self {
            path: directory.join("dirule.toml"),
            name: "dirule.toml".into(),
            kind: ConfigurationSourceKind::ProjectDefault,
        }
    }

    pub fn explicit_file(directory: &Path, path: PathBuf) -> Self {
        let name = path.display().to_string();

        Self {
            path: resolve_from(directory, path),
            name,
            kind: ConfigurationSourceKind::ExplicitFile,
        }
    }

    pub fn preset(store: &PresetStore, name: &str) -> Result<Self, PresetError> {
        Ok(Self {
            path: store.path(name)?,
            name: name.into(),
            kind: ConfigurationSourceKind::Preset,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> ConfigurationSourceKind {
        self.kind
    }
}
