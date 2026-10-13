use super::{args, output};
use dirule_core::{CURRENT_CONFIG_VERSION, Config, Plan, parse_config, plan};
use dirule_fs::{ScanResult, scan, validate_destinations};
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

pub(super) fn evaluate(
    options: args::OperationArgs,
) -> Result<(PathBuf, Plan, ScanResult), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let (config_path, config_hint) = if let Some(path) = options.rules.config {
        (resolve(&cwd, path), "pass a valid `--config FILE`")
    } else if let Some(name) = options.rules.preset {
        (
            super::presets::path(&name)?,
            "check `dirule presets list` or create that preset",
        )
    } else {
        (
            cwd.join("dirule.toml"),
            "run `dirule init` or pass `--config FILE`",
        )
    };

    if !config_path.is_file() {
        return Err(format!(
            "configuration not found at {}; {config_hint}",
            config_path.display(),
        )
        .into());
    }

    let (config, _) = load_config(&config_path)?;
    let target = options
        .target
        .map(|path| resolve_target(&cwd, path))
        .or_else(|| config.target.clone().map(|path| resolve_target(&cwd, path)))
        .transpose()?
        .unwrap_or(cwd);
    let target = target
        .canonicalize()
        .map_err(|error| format!("cannot access target {}: {error}", target.display()))?;

    let mut scan = scan(&target)?;
    let mut plan = plan(&config, &scan.entries)?;
    validate_destinations(&target, &mut plan)?;
    plan.findings.append(&mut scan.findings);
    Ok((target, plan, scan))
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
