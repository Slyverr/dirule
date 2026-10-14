use super::{args, output};
use dirule_app::configuration::{self, ConfigurationLoadError, ConfigurationSource};
use dirule_app::presets::PresetStore;
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
    let source = config_source(&cwd, options.rules, None)?;
    let loaded = load_configuration(&source)?;
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
) -> Result<dirule_app::Preview, Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let target = options.target;
    let source = config_source(&cwd, options.rules, target.clone())?;

    dirule_app::preview_source(&source, &cwd, target).map_err(preview_source_error)
}

fn config_source(
    cwd: &Path,
    rules: args::RuleSourceArgs,
    target: Option<std::path::PathBuf>,
) -> Result<ConfigurationSource, Box<dyn Error>> {
    if let Some(path) = rules.config {
        return Ok(ConfigurationSource::explicit_file(cwd, path));
    }

    if let Some(name) = rules.preset {
        return ConfigurationSource::preset(&PresetStore::user()?, &name).map_err(Into::into);
    }

    let presets = PresetStore::user_if_available()?;

    configuration::discover_source(cwd, target, presets.as_ref()).map_err(Into::into)
}

fn load_configuration(
    source: &ConfigurationSource,
) -> Result<configuration::LoadedConfiguration, Box<dyn Error>> {
    match configuration::load_source(source) {
        Ok(loaded) => Ok(loaded),
        Err(ConfigurationLoadError::Missing { source }) => Err(format!(
            "configuration not found at {}; {}",
            source.path().display(),
            missing_config_hint(source.kind()),
        )
        .into()),
        Err(error) => Err(Box::new(error)),
    }
}

fn missing_config_hint(kind: dirule_app::configuration::ConfigurationSourceKind) -> &'static str {
    use dirule_app::configuration::ConfigurationSourceKind;

    match kind {
        ConfigurationSourceKind::ProjectDefault => "run `dirule init` or pass `--config FILE`",
        ConfigurationSourceKind::ExplicitFile => "pass a valid `--config FILE`",
        ConfigurationSourceKind::Preset => "check `dirule presets list` or create that preset",
    }
}

fn preview_source_error(error: dirule_app::PreviewSourceError) -> Box<dyn Error> {
    match error {
        dirule_app::PreviewSourceError::Configuration(ConfigurationLoadError::Missing {
            source,
        }) => format!(
            "configuration not found at {}; {}",
            source.path().display(),
            missing_config_hint(source.kind()),
        )
        .into(),
        error => Box::new(error),
    }
}
