use dirule_core::{Config, parse_config};
use std::error::Error;
use std::fmt;
use std::fs::{self, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use toml_edit::{DocumentMut, TomlError};

mod loader;

pub use loader::{ConfigurationLoadError, LoadedConfiguration, load};

pub struct ConfigEditor {
    path: PathBuf,
    original: String,
    document: DocumentMut,
}

#[derive(Debug)]
pub enum ConfigEditorError {
    RelativePath(PathBuf),
    NotRegularFile(PathBuf),
    ConcurrentModification(PathBuf),
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Toml {
        path: PathBuf,
        source: TomlError,
    },
    Configuration {
        path: PathBuf,
        source: dirule_core::Error,
    },
}

impl ConfigEditor {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, ConfigEditorError> {
        let path = path.into();
        if !path.is_absolute() {
            return Err(ConfigEditorError::RelativePath(path));
        }

        let _ = regular_file_metadata(&path)?;
        let original = fs::read_to_string(&path).map_err(|source| ConfigEditorError::Io {
            action: "read",
            path: path.clone(),
            source,
        })?;
        parse_config(&original).map_err(|source| ConfigEditorError::Configuration {
            path: path.clone(),
            source,
        })?;
        let document =
            original
                .parse::<DocumentMut>()
                .map_err(|source| ConfigEditorError::Toml {
                    path: path.clone(),
                    source,
                })?;

        Ok(Self {
            path,
            original,
            document,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn document(&self) -> &DocumentMut {
        &self.document
    }

    pub fn document_mut(&mut self) -> &mut DocumentMut {
        &mut self.document
    }

    pub fn contents(&self) -> String {
        self.document.to_string()
    }

    pub fn config(&self) -> Result<Config, ConfigEditorError> {
        let contents = self.contents();

        parse_config(&contents).map_err(|source| ConfigEditorError::Configuration {
            path: self.path.clone(),
            source,
        })
    }

    pub fn save(&mut self) -> Result<Config, ConfigEditorError> {
        let rendered = self.contents();
        let contents = if rendered == format!("{}\n", self.original) {
            self.original.clone()
        } else {
            rendered
        };
        let config =
            parse_config(&contents).map_err(|source| ConfigEditorError::Configuration {
                path: self.path.clone(),
                source,
            })?;
        let metadata = regular_file_metadata(&self.path)?;
        let current = fs::read_to_string(&self.path).map_err(|source| ConfigEditorError::Io {
            action: "read",
            path: self.path.clone(),
            source,
        })?;
        if current != self.original {
            return Err(ConfigEditorError::ConcurrentModification(self.path.clone()));
        }

        if contents == self.original {
            return Ok(config);
        }

        self.replace_contents(&contents, metadata)?;
        self.original = contents;

        Ok(config)
    }

    fn replace_contents(
        &self,
        contents: &str,
        metadata: Metadata,
    ) -> Result<(), ConfigEditorError> {
        let parent = self.path.parent().ok_or_else(|| ConfigEditorError::Io {
            action: "locate parent directory for",
            path: self.path.clone(),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "configuration path has no parent directory",
            ),
        })?;
        let mut temporary =
            NamedTempFile::new_in(parent).map_err(|source| ConfigEditorError::Io {
                action: "create a temporary file beside",
                path: self.path.clone(),
                source,
            })?;
        temporary
            .write_all(contents.as_bytes())
            .map_err(|source| ConfigEditorError::Io {
                action: "write temporary configuration for",
                path: self.path.clone(),
                source,
            })?;
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|source| ConfigEditorError::Io {
                action: "preserve permissions for",
                path: self.path.clone(),
                source,
            })?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| ConfigEditorError::Io {
                action: "sync temporary configuration for",
                path: self.path.clone(),
                source,
            })?;
        temporary
            .persist(&self.path)
            .map_err(|error| ConfigEditorError::Io {
                action: "replace",
                path: self.path.clone(),
                source: error.error,
            })?;

        Ok(())
    }
}

fn regular_file_metadata(path: &Path) -> Result<Metadata, ConfigEditorError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| ConfigEditorError::Io {
        action: "inspect",
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(ConfigEditorError::NotRegularFile(path.to_path_buf()));
    }

    Ok(metadata)
}

impl fmt::Display for ConfigEditorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(path) => {
                write!(
                    formatter,
                    "configuration path must be absolute: {}",
                    path.display()
                )
            }
            Self::NotRegularFile(path) => write!(
                formatter,
                "configuration path is not a regular file: {}",
                path.display()
            ),
            Self::ConcurrentModification(path) => write!(
                formatter,
                "configuration changed after it was opened: {}",
                path.display()
            ),
            Self::Io {
                action,
                path,
                source,
            } => write!(formatter, "cannot {action} {}: {source}", path.display()),
            Self::Toml { path, source } => {
                write!(formatter, "cannot edit TOML {}: {source}", path.display())
            }
            Self::Configuration { path, source } => {
                write!(
                    formatter,
                    "invalid configuration {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl Error for ConfigEditorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Toml { source, .. } => Some(source),
            Self::Configuration { source, .. } => Some(source),
            Self::RelativePath(_) | Self::NotRegularFile(_) | Self::ConcurrentModification(_) => {
                None
            }
        }
    }
}
