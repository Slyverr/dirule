use super::{args, output, prompt};
use crate::application::presets::PresetStore;
use std::error::Error;
use std::io::{self, Write};
use std::path::PathBuf;

pub(super) fn list(
    format: args::OutputFormat,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let presets = PresetStore::user()?.list()?;
    output::presets(&presets, format, color)?;
    Ok(())
}

pub(super) fn save(
    options: args::PresetSaveArgs,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let source_path = crate::application::paths::resolve_from(
        &cwd,
        options
            .config
            .unwrap_or_else(|| PathBuf::from("dirule.toml")),
    );
    let destination = PresetStore::user()?.save(&options.name, &source_path)?;

    output::saved_preset(&options.name, &destination, color)?;
    Ok(())
}

pub(super) fn export(name: String) -> Result<(), Box<dyn Error>> {
    let preset = PresetStore::user()?.load(&name)?;
    io::stdout().lock().write_all(preset.contents.as_bytes())?;

    Ok(())
}

pub(super) fn remove(
    options: args::PresetRemoveArgs,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let store = PresetStore::user()?;
    let path = store.path(&options.name)?;
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

    store.remove(&options.name)?;
    output::removed_preset(&options.name, color)?;
    Ok(())
}
