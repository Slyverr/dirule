use crate::configuration::file::write_new;
use dirule_core::{Config, parse_config};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
/// Access a directory of named, versioned TOML rule configurations.
pub struct PresetStore {
    directory: PathBuf,
}

#[derive(Debug)]
/// A validated preset and its original TOML contents.
pub struct Preset {
    /// Name used to select the preset.
    pub name: String,
    /// Path to the preset file.
    pub path: PathBuf,
    /// Parsed and validated rules.
    pub config: Config,
    /// Original TOML source, including comments and formatting.
    pub contents: String,
}

#[derive(Debug)]
/// A preset could not be resolved, parsed, saved, or removed.
pub enum PresetError {
    UserDirectoryUnavailable,
    RelativeDirectory(PathBuf),
    InvalidName(String),
    NotFound(PathBuf),
    AlreadyExists(PathBuf),
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Configuration {
        path: PathBuf,
        source: dirule_core::Error,
    },
}

impl PresetStore {
    /// Open the platform's user preset directory or return an error.
    pub fn user() -> Result<Self, PresetError> {
        Self::user_if_available()?.ok_or(PresetError::UserDirectoryUnavailable)
    }

    /// Open the platform's user preset directory when it can be resolved.
    pub fn user_if_available() -> Result<Option<Self>, PresetError> {
        let Some(base) = super::paths::user_config_directory() else {
            return Ok(None);
        };

        Self::new(base.join("dirule").join("presets")).map(Some)
    }

    /// Create a store for an absolute directory, without creating it yet.
    pub fn new(directory: impl Into<PathBuf>) -> Result<Self, PresetError> {
        let directory = directory.into();
        if !directory.is_absolute() {
            return Err(PresetError::RelativeDirectory(directory));
        }

        Ok(Self { directory })
    }

    /// Return the directory managed by this store.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Resolve a validated preset name to its TOML path.
    pub fn path(&self, name: &str) -> Result<PathBuf, PresetError> {
        if !valid_name(name) {
            return Err(PresetError::InvalidName(name.to_owned()));
        }

        Ok(self.directory.join(format!("{name}.toml")))
    }

    /// List valid preset names in stable alphabetical order.
    pub fn list(&self) -> Result<Vec<String>, PresetError> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(PresetError::Io {
                    action: "read",
                    path: self.directory.clone(),
                    source,
                });
            }
        };

        let mut names = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|source| PresetError::Io {
                    action: "read",
                    path: self.directory.clone(),
                    source,
                })?
                .path();
            if path
                .extension()
                .is_some_and(|extension| extension == "toml")
                && let Some(name) = path.file_stem().and_then(|name| name.to_str())
                && valid_name(name)
            {
                names.push(name.to_owned());
            }
        }

        names.sort_unstable();

        Ok(names)
    }

    /// Load and validate a named preset, preserving its source text.
    pub fn load(&self, name: &str) -> Result<Preset, PresetError> {
        let path = self.path(name)?;
        let contents = fs::read_to_string(&path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                PresetError::NotFound(path.clone())
            } else {
                PresetError::Io {
                    action: "read",
                    path: path.clone(),
                    source,
                }
            }
        })?;
        let config = parse_config(&contents).map_err(|source| PresetError::Configuration {
            path: path.clone(),
            source,
        })?;

        Ok(Preset {
            name: name.to_owned(),
            path,
            config,
            contents,
        })
    }

    /// Read the target configured by a preset without loading its rules.
    pub fn configured_target(&self, name: &str) -> Result<Option<PathBuf>, PresetError> {
        let path = self.path(name)?;
        let contents = fs::read_to_string(&path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                PresetError::NotFound(path.clone())
            } else {
                PresetError::Io {
                    action: "read",
                    path: path.clone(),
                    source,
                }
            }
        })?;
        let Ok(document) = contents.parse::<toml_edit::DocumentMut>() else {
            return Ok(None);
        };

        Ok(document
            .get("target")
            .and_then(toml_edit::Item::as_str)
            .map(PathBuf::from))
    }

    /// Save a validated config file as a new preset without overwriting.
    pub fn save(&self, name: &str, source_path: &Path) -> Result<PathBuf, PresetError> {
        let destination = self.path(name)?;
        let contents = fs::read_to_string(source_path).map_err(|source| PresetError::Io {
            action: "read",
            path: source_path.to_path_buf(),
            source,
        })?;

        self.write(destination, &contents, source_path)
    }

    /// Save validated TOML contents as a new preset without overwriting.
    pub fn save_contents(&self, name: &str, contents: &str) -> Result<PathBuf, PresetError> {
        let destination = self.path(name)?;

        self.write(destination.clone(), contents, &destination)
    }

    fn write(
        &self,
        destination: PathBuf,
        contents: &str,
        config_path: &Path,
    ) -> Result<PathBuf, PresetError> {
        parse_config(contents).map_err(|source| PresetError::Configuration {
            path: config_path.to_path_buf(),
            source,
        })?;

        fs::create_dir_all(&self.directory).map_err(|source| PresetError::Io {
            action: "create directory",
            path: self.directory.clone(),
            source,
        })?;
        write_new(&destination, contents.as_bytes()).map_err(|source| {
            if source.kind() == io::ErrorKind::AlreadyExists {
                PresetError::AlreadyExists(destination.clone())
            } else {
                PresetError::Io {
                    action: "create",
                    path: destination.clone(),
                    source,
                }
            }
        })?;

        Ok(destination)
    }

    /// Remove a preset by name.
    pub fn remove(&self, name: &str) -> Result<PathBuf, PresetError> {
        let path = self.path(name)?;
        fs::remove_file(&path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                PresetError::NotFound(path.clone())
            } else {
                PresetError::Io {
                    action: "remove",
                    path: path.clone(),
                    source,
                }
            }
        })?;

        Ok(path)
    }
}

impl fmt::Display for PresetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UserDirectoryUnavailable => {
                formatter.write_str("could not determine the user configuration directory")
            }
            Self::RelativeDirectory(path) => {
                write!(
                    formatter,
                    "preset directory must be absolute: {}",
                    path.display()
                )
            }
            Self::InvalidName(name) => write!(
                formatter,
                "preset names may contain only ASCII letters, digits, '-' and '_': {name:?}"
            ),
            Self::NotFound(path) => {
                write!(formatter, "preset does not exist at {}", path.display())
            }
            Self::AlreadyExists(path) => {
                write!(formatter, "preset already exists at {}", path.display())
            }
            Self::Io {
                action,
                path,
                source,
            } => write!(formatter, "cannot {action} {}: {source}", path.display()),
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

impl Error for PresetError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Configuration { source, .. } => Some(source),
            Self::UserDirectoryUnavailable
            | Self::RelativeDirectory(_)
            | Self::InvalidName(_)
            | Self::NotFound(_)
            | Self::AlreadyExists(_) => None,
        }
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}
