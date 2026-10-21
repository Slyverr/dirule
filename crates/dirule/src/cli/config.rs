use super::{args, output};
use dirule_app::configuration::{self, ConfigurationLoadError, ConfigurationSource};
use dirule_app::presets::PresetStore;
use std::error::Error;
use std::path::Path;

pub(super) fn initialize(color: args::ColorChoice) -> Result<(), Box<dyn Error>> {
    let path = std::env::current_dir()?.join("dirule.toml");
    dirule_app::configuration::create_starter(&path)?;

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

pub(super) fn lsp_status(rules: args::RuleSourceArgs) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let source = config_source(&cwd, rules, None)?;
    let loaded = load_configuration(&source)?;
    let servers = &loaded.config().lsp.servers;

    if servers.is_empty() {
        println!(
            "No language servers are configured in {}.",
            loaded.path().display()
        );

        return Ok(());
    }

    println!("Language servers from {}:", loaded.path().display());
    let config_directory = loaded.path().parent().unwrap_or(&cwd);
    let mut missing = Vec::new();

    for server in servers {
        let languages = server
            .languages
            .iter()
            .map(|(extension, language)| format!(".{extension} ({language})"))
            .collect::<Vec<_>>()
            .join(", ");
        match resolve_executable(&server.command, config_directory) {
            Some(path) => println!(
                "  {} [{languages}] -> {}",
                server.command.display(),
                path.display()
            ),
            None => {
                println!("  {} [{languages}] -> not found", server.command.display());
                missing.push(server.command.display().to_string());
            }
        }
    }

    if !missing.is_empty() {
        return Err(format!(
            "{} configured language-server executable(s) could not be found in PATH or relative to the config file: {}",
            missing.len(),
            missing.join(", ")
        )
        .into());
    }

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

fn resolve_executable(command: &Path, config_directory: &Path) -> Option<std::path::PathBuf> {
    let has_path_component = command.is_absolute() || command.components().count() > 1;
    let candidates = if has_path_component {
        let path = if command.is_absolute() {
            command.to_path_buf()
        } else {
            config_directory.join(command)
        };
        executable_candidates(&path)
    } else {
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .flat_map(|directory| executable_candidates(&directory.join(command)))
            .collect()
    };

    candidates.into_iter().find(|path| is_executable_file(path))
}

#[cfg(windows)]
fn executable_candidates(path: &Path) -> Vec<std::path::PathBuf> {
    if path.extension().is_some() {
        return vec![path.to_path_buf()];
    }

    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    extensions
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(|extension| path.with_extension(extension.trim_start_matches('.')))
        .collect()
}

#[cfg(not(windows))]
fn executable_candidates(path: &Path) -> Vec<std::path::PathBuf> {
    vec![path.to_path_buf()]
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}
