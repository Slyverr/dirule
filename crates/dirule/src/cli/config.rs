use super::{args, output};
use crate::application::configuration;
use crate::application::presets::PresetStore;
use dirule_core::CURRENT_CONFIG_VERSION;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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

    let loaded = configuration::load(&source.path)?;
    output::validated_config(
        source.kind,
        &source.name,
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

    let loaded = configuration::load(&source.path)?;
    let config = loaded.config();
    let target = options
        .target
        .map(|path| crate::application::paths::resolve_target(&cwd, path))
        .or_else(|| {
            config
                .target
                .clone()
                .map(|path| crate::application::paths::resolve_target(&cwd, path))
        })
        .transpose()?
        .unwrap_or(cwd);
    Ok(crate::application::preview(config, &target)?)
}

struct ConfigSource {
    path: PathBuf,
    kind: &'static str,
    name: String,
    missing_hint: &'static str,
}

fn config_source(cwd: &Path, rules: args::RuleSourceArgs) -> Result<ConfigSource, Box<dyn Error>> {
    if let Some(path) = rules.config {
        let name = path.display().to_string();

        return Ok(ConfigSource {
            path: crate::application::paths::resolve_from(cwd, path),
            kind: "config",
            name,
            missing_hint: "pass a valid `--config FILE`",
        });
    }

    if let Some(name) = rules.preset {
        return Ok(ConfigSource {
            path: PresetStore::user()?.path(&name)?,
            kind: "preset",
            name,
            missing_hint: "check `dirule presets list` or create that preset",
        });
    }

    Ok(ConfigSource {
        path: cwd.join("dirule.toml"),
        kind: "config",
        name: "dirule.toml".to_owned(),
        missing_hint: "run `dirule init` or pass `--config FILE`",
    })
}

fn ensure_config_exists(source: &ConfigSource) -> Result<(), Box<dyn Error>> {
    if !source.path.is_file() {
        return Err(format!(
            "configuration not found at {}; {}",
            source.path.display(),
            source.missing_hint,
        )
        .into());
    }

    Ok(())
}
