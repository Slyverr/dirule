use super::{args, output};
use dirule_core::{CURRENT_CONFIG_VERSION, Config, Plan, parse_config};
use dirule_fs::ScanResult;
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

    let (config, _) = load_config(&source.path)?;
    output::validated_config(
        source.kind,
        &source.name,
        config.rules.len(),
        options.format,
        color,
    )?;

    Ok(())
}

pub(super) fn evaluate(
    options: args::OperationArgs,
) -> Result<(PathBuf, Plan, ScanResult), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let source = config_source(&cwd, options.rules)?;
    ensure_config_exists(&source)?;

    let (config, _) = load_config(&source.path)?;
    let target = options
        .target
        .map(|path| resolve_target(&cwd, path))
        .or_else(|| config.target.clone().map(|path| resolve_target(&cwd, path)))
        .transpose()?
        .unwrap_or(cwd);
    let preview = crate::application::preview(&config, &target)?;

    Ok((preview.target, preview.plan, preview.snapshot))
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
            path: resolve(cwd, path),
            kind: "config",
            name,
            missing_hint: "pass a valid `--config FILE`",
        });
    }

    if let Some(name) = rules.preset {
        return Ok(ConfigSource {
            path: super::presets::path(&name)?,
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

pub(super) fn load_config(path: &Path) -> Result<(Config, String), Box<dyn Error>> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("cannot read configuration {}: {error}", path.display()))?;
    let config = parse_config(&contents)
        .map_err(|error| format!("invalid configuration {}: {error}", path.display()))?;

    Ok((config, contents))
}

pub(super) fn resolve(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn resolve_target(base: &Path, path: PathBuf) -> Result<PathBuf, Box<dyn Error>> {
    let Some(path_text) = path.to_str() else {
        return Ok(resolve(base, path));
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
        return Ok(resolve(base, path));
    };

    let home = super::paths::home_directory()
        .ok_or("cannot expand target: user home directory is unavailable")?;
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
