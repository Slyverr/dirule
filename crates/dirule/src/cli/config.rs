use super::{args, output};
use crate::application::configuration::{self, ConfigurationSource};
use crate::application::presets::PresetStore;
use dirule_core::CURRENT_CONFIG_VERSION;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::Path;

pub(super) fn initialize(color: args::ColorChoice) -> Result<(), Box<dyn Error>> {
    let path = std::env::current_dir()?.join("dirule.toml");
    let content = format!(
        "version = \"{CURRENT_CONFIG_VERSION}\"\nname = \"Local rules\"\ntarget = \".\"\n\n[[rules]]\nname = \"Normalize file names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n"
    );

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    file.write_all(content.as_bytes())?;
    output::created_config(&path, color)?;
    Ok(())
}

pub(super) fn validate(
    options: args::ValidateArgs,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let source = config_source(&cwd, options.rules)?;
    ensure_config_exists(&source)?;

    let loaded = configuration::load(source.path())?;
    output::validated_config(
        source.kind().as_str(),
        source.name(),
        loaded.config().rules.len(),
        options.format,
        color,
    )?;

    Ok(())
}

pub(super) fn evaluate(
    options: args::OperationArgs,
) -> Result<crate::application::Preview, Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let source = config_source(&cwd, options.rules)?;
    ensure_config_exists(&source)?;

    let loaded = configuration::load(source.path())?;
    let config = loaded.config();
    let target = crate::application::paths::resolve_operation_target(
        &cwd,
        options.target,
        config.target.clone(),
    )?;
    Ok(crate::application::preview(config, &target)?)
}

fn config_source(
    cwd: &Path,
    rules: args::RuleSourceArgs,
) -> Result<ConfigurationSource, Box<dyn Error>> {
    if let Some(path) = rules.config {
        return Ok(ConfigurationSource::explicit_file(cwd, path));
    }

    if let Some(name) = rules.preset {
        return ConfigurationSource::preset(&PresetStore::user()?, &name).map_err(Into::into);
    }

    Ok(ConfigurationSource::project_default(cwd))
}

fn ensure_config_exists(source: &ConfigurationSource) -> Result<(), Box<dyn Error>> {
    if !source.path().is_file() {
        return Err(format!(
            "configuration not found at {}; {}",
            source.path().display(),
            missing_config_hint(source.kind()),
        )
        .into());
    }

    Ok(())
}

fn missing_config_hint(
    kind: crate::application::configuration::ConfigurationSourceKind,
) -> &'static str {
    use crate::application::configuration::ConfigurationSourceKind;

    match kind {
        ConfigurationSourceKind::ProjectDefault => "run `dirule init` or pass `--config FILE`",
        ConfigurationSourceKind::ExplicitFile => "pass a valid `--config FILE`",
        ConfigurationSourceKind::Preset => "check `dirule presets list` or create that preset",
    }
}
