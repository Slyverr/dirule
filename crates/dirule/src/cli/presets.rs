use super::{args, config, output, paths, prompt};
use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

pub(super) fn list(
    format: args::OutputFormat,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let directory = paths::preset_directory()?;
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => Some(entries),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("cannot read {}: {error}", directory.display()).into()),
    };
    let mut presets = Vec::new();
    if let Some(entries) = entries {
        for entry in entries {
            let path = entry
                .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
                .path();
            if path
                .extension()
                .is_some_and(|extension| extension == "toml")
                && let Some(name) = path.file_stem().and_then(|name| name.to_str())
                && valid_name(name)
            {
                presets.push(name.to_owned());
            }
        }
    }

    presets.sort();
    output::presets(&presets, format, color)?;
    Ok(())
}

pub(super) fn save(
    options: args::PresetSaveArgs,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let source_path = config::resolve(
        &cwd,
        options
            .config
            .unwrap_or_else(|| PathBuf::from("dirule.toml")),
    );
    let destination = path(&options.name)?;
    let (_, contents) = config::load_config(&source_path)?;

    let directory = destination
        .parent()
        .ok_or("preset path has no parent directory")?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                format!(
                    "preset {:?} already exists at {}",
                    options.name,
                    destination.display()
                )
            } else {
                format!("cannot create {}: {error}", destination.display())
            }
        })?;
    if let Err(error) = file.write_all(contents.as_bytes()) {
        drop(file);
        let _ = fs::remove_file(&destination);
        return Err(format!("cannot write {}: {error}", destination.display()).into());
    }

    output::saved_preset(&options.name, &destination, color)?;
    Ok(())
}

pub(super) fn remove(
    options: args::PresetRemoveArgs,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let path = path(&options.name)?;
    if !path.is_file() {
        return Err(format!("user preset {:?} does not exist", options.name).into());
    }
    if !prompt::confirm(
        &format!("Remove user preset {:?}?", options.name),
        "use --yes to remove a preset without an interactive terminal",
        options.yes,
    )? {
        output::cancelled("Preset removal", color)?;
        return Ok(());
    }

    fs::remove_file(&path).map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
    output::removed_preset(&options.name, color)?;
    Ok(())
}

pub(super) fn path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    if !valid_name(name) {
        return Err("preset names may contain only ASCII letters, digits, '-' and '_'".into());
    }
    Ok(paths::preset_directory()?.join(format!("{name}.toml")))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}
