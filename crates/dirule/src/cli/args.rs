use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "A local-first filesystem rules CLI.",
    arg_required_else_help = true,
    propagate_version = true
)]
pub(super) struct Cli {
    /// Control colored terminal output.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto, global = true)]
    pub(super) color: ColorChoice,

    #[command(subcommand)]
    pub(super) command: CliCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum CliCommand {
    /// Create a starter local config.
    Init,
    /// Check a target without changing files.
    Check(OperationArgs),
    /// Preview planned filesystem changes.
    Plan(OperationArgs),
    /// Review and apply a plan.
    Apply(ApplyArgs),
    /// Manage global presets.
    Presets(PresetsArgs),
    /// Inspect saved apply runs.
    Runs(RunsArgs),
    /// Undo a previously applied run.
    Undo(UndoArgs),
    /// Generate shell completions.
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Debug, Args)]
pub(super) struct OperationArgs {
    /// Directory to check or organize.
    #[arg(long, value_name = "PATH")]
    pub(super) target: Option<PathBuf>,

    #[command(flatten)]
    pub(super) rules: RuleSourceArgs,

    /// Output as human-readable text or JSON.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub(super) format: OutputFormat,
}

#[derive(Debug, Args)]
#[group(multiple = false)]
pub(super) struct RuleSourceArgs {
    /// Read rules from this TOML file.
    #[arg(long, value_name = "FILE")]
    pub(super) config: Option<PathBuf>,

    /// Select a named preset.
    #[arg(long, value_name = "NAME")]
    pub(super) preset: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct ApplyArgs {
    #[command(flatten)]
    pub(super) operation: OperationArgs,

    /// Skip the interactive confirmation prompt.
    #[arg(long)]
    pub(super) yes: bool,

    /// Choose whether conflicts abort the operation or skip affected paths.
    #[arg(long, value_enum, default_value_t = ConflictAction::Abort)]
    pub(super) on_conflict: ConflictAction,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(super) enum ConflictAction {
    #[default]
    Abort,
    Skip,
}

#[derive(Debug, Args)]
pub(super) struct PresetsArgs {
    #[command(subcommand)]
    pub(super) command: PresetsCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum PresetsCommand {
    /// List available presets.
    List,
    /// Save a project config as a global preset.
    Save(PresetSaveArgs),
    /// Remove a user preset.
    Remove(PresetRemoveArgs),
}

#[derive(Debug, Args)]
pub(super) struct RunsArgs {
    #[command(subcommand)]
    pub(super) command: RunsCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum RunsCommand {
    /// List recorded apply runs.
    List {
        /// Output as human-readable text or JSON.
        #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
}

#[derive(Debug, Args)]
pub(super) struct UndoArgs {
    /// Identifier shown by `dirule runs list` or after `dirule apply`.
    pub(super) run_id: String,

    /// Skip the interactive confirmation prompt.
    #[arg(long)]
    pub(super) yes: bool,

    /// Output as human-readable text or JSON.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub(super) format: OutputFormat,
}

#[derive(Debug, Args)]
pub(super) struct PresetSaveArgs {
    /// Name to assign to the preset.
    pub(super) name: String,

    /// Read rules from this TOML file instead of ./dirule.toml.
    #[arg(long, value_name = "FILE")]
    pub(super) config: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub(super) struct PresetRemoveArgs {
    /// Name of the preset to remove.
    pub(super) name: String,

    /// Remove without an interactive confirmation.
    #[arg(long)]
    pub(super) yes: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(super) enum OutputFormat {
    #[default]
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(super) enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

#[cfg(test)]
mod tests {
    use super::{ApplyArgs, Cli, CliCommand, ConflictAction, OutputFormat, PresetsCommand};
    use clap::Parser;

    #[test]
    fn check_parses_operation_options_after_subcommand() {
        let cli = Cli::try_parse_from([
            "dirule",
            "check",
            "--target",
            "/tmp/files",
            "--config",
            "rules.toml",
            "--format",
            "json",
        ])
        .expect("valid check options");

        let CliCommand::Check(options) = cli.command else {
            panic!("expected check command");
        };

        assert_eq!(
            options.target.as_deref(),
            Some(std::path::Path::new("/tmp/files"))
        );
        assert_eq!(
            options.rules.config.as_deref(),
            Some(std::path::Path::new("rules.toml"))
        );
        assert_eq!(options.format, OutputFormat::Json);
    }

    #[test]
    fn apply_parses_shared_options_and_confirmation_flag() {
        let cli = Cli::try_parse_from(["dirule", "apply", "--preset", "downloads", "--yes"])
            .expect("valid apply options");

        let CliCommand::Apply(ApplyArgs {
            operation,
            yes,
            on_conflict,
        }) = cli.command
        else {
            panic!("expected apply command");
        };

        assert!(yes);
        assert_eq!(on_conflict, ConflictAction::Abort);
        assert_eq!(operation.rules.preset.as_deref(), Some("downloads"));
        assert_eq!(operation.format, OutputFormat::Human);
    }

    #[test]
    fn rule_config_and_preset_are_exclusive() {
        let error = Cli::try_parse_from([
            "dirule",
            "plan",
            "--config",
            "rules.toml",
            "--preset",
            "downloads",
        ])
        .expect_err("config and preset conflict");

        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn output_format_rejects_unknown_values() {
        let error = Cli::try_parse_from(["dirule", "check", "--format", "yaml"])
            .expect_err("unknown output format");

        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
    }

    #[test]
    fn preset_save_accepts_name_and_optional_config_path() {
        let cli = Cli::try_parse_from([
            "dirule",
            "presets",
            "save",
            "downloads",
            "--config",
            "rules.toml",
        ])
        .expect("valid preset save options");

        let CliCommand::Presets(args) = cli.command else {
            panic!("expected presets command");
        };
        let PresetsCommand::Save(options) = args.command else {
            panic!("expected preset save command");
        };

        assert_eq!(options.name, "downloads");
        assert_eq!(
            options.config.as_deref(),
            Some(std::path::Path::new("rules.toml"))
        );
    }
}
