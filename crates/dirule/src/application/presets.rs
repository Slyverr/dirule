use dirule_core::{Config, parse_config};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct PresetStore {
    directory: PathBuf,
}

#[derive(Debug)]
pub struct Preset {
    pub name: String,
    pub path: PathBuf,
    pub config: Config,
    pub contents: String,
}

#[derive(Debug)]
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
    pub fn user() -> Result<Self, PresetError> {
        let base = user_config_directory().ok_or(PresetError::UserDirectoryUnavailable)?;

        Self::new(base.join("dirule").join("presets"))
    }

    pub fn new(directory: impl Into<PathBuf>) -> Result<Self, PresetError> {
        let directory = directory.into();
        if !directory.is_absolute() {
            return Err(PresetError::RelativeDirectory(directory));
        }

        Ok(Self { directory })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn path(&self, name: &str) -> Result<PathBuf, PresetError> {
        if !valid_name(name) {
            return Err(PresetError::InvalidName(name.to_owned()));
        }

        Ok(self.directory.join(format!("{name}.toml")))
    }

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

    pub fn save(&self, name: &str, source_path: &Path) -> Result<PathBuf, PresetError> {
        let destination = self.path(name)?;
        let contents = fs::read_to_string(source_path).map_err(|source| PresetError::Io {
            action: "read",
            path: source_path.to_path_buf(),
            source,
        })?;

        self.write(destination, &contents, source_path)
    }

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
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|source| {
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
        if let Err(source) = file.write_all(contents.as_bytes()) {
            drop(file);
            let _ = fs::remove_file(&destination);

            return Err(PresetError::Io {
                action: "write",
                path: destination,
                source,
            });
        }

        Ok(destination)
    }

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

fn user_config_directory() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = absolute_env_path("APPDATA");
    #[cfg(target_os = "macos")]
    let base = absolute_env_path("HOME").map(|home| home.join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = absolute_env_path("XDG_CONFIG_HOME")
        .or_else(|| absolute_env_path("HOME").map(|home| home.join(".config")));
    #[cfg(not(any(unix, target_os = "windows")))]
    let base = absolute_env_path("HOME");

    base
}

fn absolute_env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}
