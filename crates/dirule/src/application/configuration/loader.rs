use dirule_core::{Config, parse_config};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum ConfigurationLoadError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Invalid {
        path: PathBuf,
        source: dirule_core::Error,
    },
}

#[derive(Debug)]
pub struct LoadedConfiguration {
    path: PathBuf,
    config: Config,
}

impl LoadedConfiguration {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
}

pub fn load(path: impl Into<PathBuf>) -> Result<LoadedConfiguration, ConfigurationLoadError> {
    let path = path.into();
    let contents = fs::read_to_string(&path).map_err(|source| ConfigurationLoadError::Io {
        path: path.clone(),
        source,
    })?;
    let config = parse_config(&contents).map_err(|source| ConfigurationLoadError::Invalid {
        path: path.clone(),
        source,
    })?;

    Ok(LoadedConfiguration { path, config })
}

impl fmt::Display for ConfigurationLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(
                    formatter,
                    "cannot read configuration {}: {source}",
                    path.display()
                )
            }
            Self::Invalid { path, source } => {
                write!(
                    formatter,
                    "invalid configuration {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl Error for ConfigurationLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Invalid { source, .. } => Some(source),
        }
    }
}
