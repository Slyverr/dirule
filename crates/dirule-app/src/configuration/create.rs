use dirule_core::{CURRENT_CONFIG_VERSION, parse_config};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
/// Creating a configuration file failed because its path, content, or write was invalid.
pub enum ConfigCreationError {
    RelativePath(PathBuf),
    InvalidStarter(dirule_core::Error),
    InvalidConfiguration(dirule_core::Error),
    Io { path: PathBuf, source: io::Error },
}

/// Create a validated starter config without overwriting an existing file.
pub fn create_starter(path: impl Into<PathBuf>) -> Result<(), ConfigCreationError> {
    let path = path.into();
    if !path.is_absolute() {
        return Err(ConfigCreationError::RelativePath(path));
    }

    let contents = starter_document();
    parse_config(&contents).map_err(ConfigCreationError::InvalidStarter)?;

    create_file(&path, &contents)
}

/// Create validated configuration content without replacing an existing file.
pub fn create_config(path: impl Into<PathBuf>, contents: &str) -> Result<(), ConfigCreationError> {
    let path = path.into();
    if !path.is_absolute() {
        return Err(ConfigCreationError::RelativePath(path));
    }

    parse_config(contents).map_err(ConfigCreationError::InvalidConfiguration)?;

    create_file(&path, contents)
}

fn create_file(path: &Path, contents: &str) -> Result<(), ConfigCreationError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| ConfigCreationError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    if let Err(source) = file.write_all(contents.as_bytes()) {
        remove_partial(path, file);

        return Err(ConfigCreationError::Io {
            path: path.to_path_buf(),
            source,
        });
    }

    if let Err(source) = file.sync_all() {
        remove_partial(path, file);

        return Err(ConfigCreationError::Io {
            path: path.to_path_buf(),
            source,
        });
    }

    Ok(())
}

fn remove_partial(path: &Path, file: fs::File) {
    drop(file);
    let _ = fs::remove_file(path);
}

fn starter_document() -> String {
    format!(
        "version = \"{CURRENT_CONFIG_VERSION}\"\nname = \"Local rules\"\ntarget = \".\"\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n"
    )
}

impl fmt::Display for ConfigCreationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(path) => write!(
                formatter,
                "configuration path must be absolute: {}",
                path.display()
            ),
            Self::InvalidStarter(source) => {
                write!(
                    formatter,
                    "invalid built-in starter configuration: {source}"
                )
            }
            Self::InvalidConfiguration(source) => {
                write!(formatter, "invalid configuration: {source}")
            }
            Self::Io { path, source } => {
                write!(formatter, "cannot create {}: {source}", path.display())
            }
        }
    }
}

impl Error for ConfigCreationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidStarter(source) | Self::InvalidConfiguration(source) => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::RelativePath(_) => None,
        }
    }
}
