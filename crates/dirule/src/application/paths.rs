use dirule_fs::JournalStore;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserStateDirectoryUnavailable;

impl fmt::Display for UserStateDirectoryUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("could not determine the user state directory")
    }
}

impl Error for UserStateDirectoryUnavailable {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HomeDirectoryUnavailable;

impl fmt::Display for HomeDirectoryUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("cannot expand target: user home directory is unavailable")
    }
}

impl Error for HomeDirectoryUnavailable {}

pub fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        absolute_env_path("USERPROFILE")
    }
    #[cfg(not(windows))]
    {
        absolute_env_path("HOME")
    }
}

pub fn resolve_from(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

pub fn resolve_target(base: &Path, path: PathBuf) -> Result<PathBuf, HomeDirectoryUnavailable> {
    let Some(path_text) = path.to_str() else {
        return Ok(resolve_from(base, path));
    };
    let home_relative = if path_text == "~" {
        Some("")
    } else {
        #[cfg(windows)]
        {
            path_text
                .strip_prefix("~/")
                .or_else(|| path_text.strip_prefix("~\\"))
        }
        #[cfg(not(windows))]
        {
            path_text.strip_prefix("~/")
        }
    };
    let Some(home_relative) = home_relative else {
        return Ok(resolve_from(base, path));
    };

    let home = home_directory().ok_or(HomeDirectoryUnavailable)?;
    let home_relative = home_relative.trim_start_matches(|separator| {
        #[cfg(windows)]
        {
            matches!(separator, '/' | '\\')
        }
        #[cfg(not(windows))]
        {
            separator == '/'
        }
    });

    Ok(home.join(home_relative))
}

pub fn user_journal_store() -> Result<JournalStore, UserStateDirectoryUnavailable> {
    let base = user_state_directory().ok_or(UserStateDirectoryUnavailable)?;

    Ok(JournalStore::new(base.join("dirule").join("runs")))
}

pub(super) fn user_config_directory() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = absolute_env_path("APPDATA");
    #[cfg(target_os = "macos")]
    let base = home_directory().map(|home| home.join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = absolute_env_path("XDG_CONFIG_HOME")
        .or_else(|| home_directory().map(|home| home.join(".config")));
    #[cfg(not(any(unix, target_os = "windows")))]
    let base = home_directory();

    base
}

fn user_state_directory() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = absolute_env_path("LOCALAPPDATA");
    #[cfg(target_os = "macos")]
    let base = home_directory().map(|home| home.join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = absolute_env_path("XDG_STATE_HOME")
        .or_else(|| home_directory().map(|home| home.join(".local/state")));
    #[cfg(not(any(unix, target_os = "windows")))]
    let base = home_directory();

    base
}

fn absolute_env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}
