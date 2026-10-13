mod args;
mod output;

use self::args::{Cli, CliCommand, ColorChoice, ConflictAction, PresetsCommand, RunsCommand};
use clap::{CommandFactory, Parser};
use dirule_core::{CURRENT_CONFIG_VERSION, Config, Plan, parse_config, plan};
use dirule_fs::{ConflictPolicy, JournalStore, apply_journaled, scan, validate_destinations};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct CliError {
    source: Box<dyn Error>,
    color: ColorChoice,
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.source, formatter)
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub fn print_error(error: &CliError) {
    output::error(&error.to_string(), error.color);
}

pub fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let color = cli.color;

    run_command(cli.command, color).map_err(|source| CliError { source, color })
}

fn run_command(command: CliCommand, color: ColorChoice) -> Result<(), Box<dyn Error>> {
    match command {
        CliCommand::Init => initialize(color),
        CliCommand::Check(options) => {
            let format = options.format;
            let (target, result, _) = evaluate(options)?;

            output::plan(&target, &result, format, color, output::PlanContext::Check)?;
            if result.has_changes() || !result.is_conflict_free() {
                return Err("target does not satisfy the configured rules".into());
            }

            Ok(())
        }
        CliCommand::Plan(options) => {
            let format = options.format;
            let (target, result, _) = evaluate(options)?;

            output::plan(
                &target,
                &result,
                format,
                color,
                output::PlanContext::Preview,
            )?;
            if !result.is_conflict_free() {
                return Err(format!("plan contains {} conflict(s)", result.conflicts.len()).into());
            }
            Ok(())
        }
        CliCommand::Apply(options) => {
            let args::ApplyArgs {
                operation,
                yes,
                on_conflict,
            } = options;
            let format = operation.format;
            let (target, result, snapshot) = evaluate(operation)?;

            if format == args::OutputFormat::Human {
                output::plan(&target, &result, format, color, output::PlanContext::Apply)?;
            }
            if !result.is_conflict_free() && on_conflict == ConflictAction::Abort {
                return Err(format!("plan contains {} conflict(s)", result.conflicts.len()).into());
            }
            if !result.has_changes() {
                output::apply_result(
                    &target,
                    &result,
                    output::ApplySummary {
                        status: output::ApplyStatus::NoChanges,
                        moved: 0,
                        skipped: 0,
                        run_id: None,
                    },
                    format,
                    color,
                )?;
                return Ok(());
            }
            if !confirm(
                "Apply this plan?",
                "use --yes to apply without an interactive terminal",
                yes,
            )? {
                output::apply_result(
                    &target,
                    &result,
                    output::ApplySummary {
                        status: output::ApplyStatus::Cancelled,
                        moved: 0,
                        skipped: 0,
                        run_id: None,
                    },
                    format,
                    color,
                )?;
                return Ok(());
            }
            let policy = match on_conflict {
                ConflictAction::Abort => ConflictPolicy::Abort,
                ConflictAction::Skip => ConflictPolicy::Skip,
            };
            let store = JournalStore::new(journal_directory()?);
            let applied = apply_journaled(&target, &result, &snapshot, policy, &store)?;
            output::apply_result(
                &target,
                &result,
                output::ApplySummary {
                    status: output::ApplyStatus::Applied,
                    moved: applied.report.moved.len(),
                    skipped: applied.report.skipped.len(),
                    run_id: applied.run_id.as_deref(),
                },
                format,
                color,
            )?;
            Ok(())
        }
        CliCommand::Presets(args) => match args.command {
            PresetsCommand::List => list_presets(color),
            PresetsCommand::Save(options) => save_preset(options, color),
            PresetsCommand::Remove(options) => remove_preset(options, color),
        },
        CliCommand::Runs(args) => match args.command {
            RunsCommand::List { format } => {
                let runs = JournalStore::new(journal_directory()?).list()?;
                output::runs(&runs, format, color);
                Ok(())
            }
            RunsCommand::Show { run_id, format } => {
                let details = JournalStore::new(journal_directory()?).get(&run_id)?;
                output::run_details(&details, format, color)?;
                Ok(())
            }
        },
        CliCommand::Undo(options) => {
            if !confirm(
                &format!("Undo apply run {}?", options.run_id),
                "use --yes to undo without an interactive terminal",
                options.yes,
            )? {
                output::undo_result(&options.run_id, "cancelled", &[], options.format, color)?;
                return Ok(());
            }
            let store = JournalStore::new(journal_directory()?);
            let report = store.undo(&options.run_id)?;
            output::undo_result(
                &options.run_id,
                "undone",
                &report.retained_directories,
                options.format,
                color,
            )?;
            Ok(())
        }
        CliCommand::Completions { shell } => {
            let mut command = Cli::command();
            let binary_name = command.get_name().to_owned();
            clap_complete::generate(shell, &mut command, binary_name, &mut io::stdout());
            Ok(())
        }
    }
}

fn initialize(color: ColorChoice) -> Result<(), Box<dyn Error>> {
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
    output::created_config(&path, color);
    Ok(())
}

fn evaluate(
    options: args::OperationArgs,
) -> Result<(PathBuf, Plan, dirule_fs::ScanResult), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let (config_path, config_hint) = if let Some(path) = options.rules.config {
        (resolve(&cwd, path), "pass a valid `--config FILE`")
    } else if let Some(name) = options.rules.preset {
        (
            preset_path(&name)?,
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

    let config: Config = parse_config(&fs::read_to_string(&config_path)?)?;
    let target = options
        .target
        .map(|path| resolve(&cwd, path))
        .or_else(|| config.target.clone().map(|path| resolve(&cwd, path)))
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

fn list_presets(color: ColorChoice) -> Result<(), Box<dyn Error>> {
    let directory = preset_directory()?;
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => Some(entries),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
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
            {
                presets.push(name.to_owned());
            }
        }
    }

    presets.sort();
    println!("Presets");
    if presets.is_empty() {
        println!("  No user presets found.");
    } else {
        for preset in presets {
            println!("  {}", output::preset_name(&preset, color));
        }
    }
    Ok(())
}

fn save_preset(options: args::PresetSaveArgs, color: ColorChoice) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let source_path = resolve(
        &cwd,
        options
            .config
            .unwrap_or_else(|| PathBuf::from("dirule.toml")),
    );
    let destination = preset_path(&options.name)?;
    let contents = fs::read_to_string(&source_path)
        .map_err(|error| format!("cannot read {}: {error}", source_path.display()))?;
    parse_config(&contents)?;

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

    output::saved_preset(&options.name, &destination, color);
    Ok(())
}

fn remove_preset(
    options: args::PresetRemoveArgs,
    color: ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let path = preset_path(&options.name)?;
    if !path.is_file() {
        return Err(format!("user preset {:?} does not exist", options.name).into());
    }
    if !confirm(
        &format!("Remove user preset {:?}?", options.name),
        "use --yes to remove a preset without an interactive terminal",
        options.yes,
    )? {
        output::cancelled("Preset removal", color);
        return Ok(());
    }

    fs::remove_file(&path).map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
    output::removed_preset(&options.name, color);
    Ok(())
}

fn preset_path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err("preset names may contain only ASCII letters, digits, '-' and '_'".into());
    }
    Ok(preset_directory()?.join(format!("{name}.toml")))
}

fn preset_directory() -> Result<PathBuf, Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    let base = env_path("APPDATA");
    #[cfg(target_os = "macos")]
    let base = env_path("HOME").map(|home| home.join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = env_path("XDG_CONFIG_HOME")
        .filter(|path| path.is_absolute())
        .or_else(|| env_path("HOME").map(|home| home.join(".config")));
    #[cfg(not(any(unix, target_os = "windows")))]
    let base = env_path("HOME");

    let base = base.ok_or("could not determine the user configuration directory")?;
    Ok(base.join("dirule").join("presets"))
}

fn journal_directory() -> Result<PathBuf, Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    let base = env_path("LOCALAPPDATA");
    #[cfg(target_os = "macos")]
    let base = env_path("HOME").map(|home| home.join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = env_path("XDG_STATE_HOME")
        .filter(|path| path.is_absolute())
        .or_else(|| env_path("HOME").map(|home| home.join(".local/state")));
    #[cfg(not(any(unix, target_os = "windows")))]
    let base = env_path("HOME");

    let base = base.ok_or("could not determine the user state directory")?;
    Ok(base.join("dirule").join("runs"))
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn resolve(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn confirm(prompt: &str, non_interactive_hint: &str, yes: bool) -> Result<bool, Box<dyn Error>> {
    if yes {
        return Ok(true);
    }
    if !io::stdin().is_terminal() {
        return Err(non_interactive_hint.into());
    }
    eprint!("{prompt} [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
