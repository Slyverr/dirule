use crate::ProviderError;
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

const MAX_CREDENTIALS_FILE_BYTES: u64 = 64 * 1024;

pub(crate) fn resolve_api_key(variable_name: &str) -> Result<String, ProviderError> {
    match std::env::var(variable_name) {
        Ok(value) if !value.trim().is_empty() => return Ok(value),
        Ok(_) | Err(std::env::VarError::NotPresent) => {}
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(ProviderError::new(
                "API-key environment variable is not valid UTF-8",
            ));
        }
    }

    let config_directory = dirule_app::paths::user_config_directory()
        .ok_or_else(|| ProviderError::new("cannot determine the Dirule user config directory"))?;
    let credentials_path = config_directory.join("dirule").join("credentials.env");
    let credentials = read_credentials_file(&credentials_path)?;
    let Some(api_key) = parse_credentials(&credentials_path, &credentials, variable_name)? else {
        return Err(ProviderError::new(format!(
            "API key {variable_name:?} was not found in the environment or {}",
            credentials_path.display()
        )));
    };
    if api_key.trim().is_empty() {
        return Err(ProviderError::new(format!(
            "API key {variable_name:?} is empty in {}",
            credentials_path.display()
        )));
    }

    Ok(api_key)
}

fn read_credentials_file(path: &Path) -> Result<Vec<u8>, ProviderError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => {
            return Err(ProviderError::new(format!(
                "cannot access Dirule credentials file at {}",
                path.display()
            )));
        }
    };
    if !metadata.is_file() {
        return Err(ProviderError::new(format!(
            "Dirule credentials path is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_CREDENTIALS_FILE_BYTES {
        return Err(ProviderError::new(format!(
            "Dirule credentials file exceeds the {MAX_CREDENTIALS_FILE_BYTES}-byte limit"
        )));
    }

    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(ProviderError::new(format!(
            "Dirule credentials file must be private to its owner; set permissions to 0600: {}",
            path.display()
        )));
    }

    let mut contents = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|file| {
            file.take(MAX_CREDENTIALS_FILE_BYTES + 1)
                .read_to_end(&mut contents)
        })
        .map_err(|_| {
            ProviderError::new(format!(
                "cannot read Dirule credentials file at {}",
                path.display()
            ))
        })?;
    if contents.len() as u64 > MAX_CREDENTIALS_FILE_BYTES {
        return Err(ProviderError::new(format!(
            "Dirule credentials file exceeds the {MAX_CREDENTIALS_FILE_BYTES}-byte limit"
        )));
    }

    Ok(contents)
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn parse_credentials(
    path: &Path,
    contents: &[u8],
    requested_name: &str,
) -> Result<Option<String>, ProviderError> {
    let contents = std::str::from_utf8(contents).map_err(|_| {
        ProviderError::new(format!(
            "Dirule credentials file is not valid UTF-8: {}",
            path.display()
        ))
    })?;

    let mut names = BTreeSet::new();
    let mut requested_value = None;
    for (index, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((name, value)) = line.split_once('=') else {
            return Err(credentials_line_error(path, index + 1));
        };
        let name = name.trim();
        if !valid_environment_name(name) || !names.insert(name) {
            return Err(credentials_line_error(path, index + 1));
        }

        let value =
            parse_value(value.trim()).ok_or_else(|| credentials_line_error(path, index + 1))?;
        if name == requested_name {
            requested_value = Some(value.to_owned());
        }
    }

    Ok(requested_value)
}

fn parse_value(value: &str) -> Option<&str> {
    match value.as_bytes().first() {
        Some(b'\'') | Some(b'"') => {
            let quote = value.as_bytes()[0];
            (value.as_bytes().last() == Some(&quote) && value.len() >= 2)
                .then(|| &value[1..value.len() - 1])
        }
        _ => Some(value),
    }
}

fn credentials_line_error(path: &Path, line: usize) -> ProviderError {
    ProviderError::new(format!(
        "invalid Dirule credentials file at {} line {line}",
        path.display()
    ))
}

pub(crate) fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };

    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
