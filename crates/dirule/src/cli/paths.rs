use std::error::Error;
use std::path::PathBuf;

pub(super) fn journal_directory() -> Result<PathBuf, Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    let base = absolute_env_path("LOCALAPPDATA");
    #[cfg(target_os = "macos")]
    let base = absolute_env_path("HOME").map(|home| home.join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = absolute_env_path("XDG_STATE_HOME")
        .or_else(|| absolute_env_path("HOME").map(|home| home.join(".local/state")));
    #[cfg(not(any(unix, target_os = "windows")))]
    let base = absolute_env_path("HOME");

    let base = base.ok_or("could not determine the user state directory")?;
    Ok(base.join("dirule").join("runs"))
}

pub(super) fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        absolute_env_path("USERPROFILE")
    }
    #[cfg(not(windows))]
    {
        absolute_env_path("HOME")
    }
}

fn absolute_env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}
