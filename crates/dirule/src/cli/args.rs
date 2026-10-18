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
    /// Validate a config without scanning its target.
    Validate(ValidateArgs),
    /// Preview planned filesystem changes.
    Plan(OperationArgs),
    /// Review and apply a plan.
    Apply(ApplyArgs),
    /// Sort files by extension or category.
    Sort(SortArgs),
    /// Rename entry names with a built-in transformation.
    Rename(RenameArgs),
    /// List the effective built-in and user-defined categories.
    Categories(CategoriesArgs),
    /// Manage global presets.
    Presets(PresetsArgs),
    /// Inspect saved apply runs.
    Runs(RunsArgs),
    /// Undo a previously applied run.
    Undo(UndoArgs),
    /// Ask a configured model to inspect and preview Dirule operations.
    Ask(AskArgs),
    /// Generate shell completions.
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: Shell,
    },
}

impl CliCommand {
    pub(super) fn output_format(&self) -> OutputFormat {
        match self {
            Self::Check(options) | Self::Plan(options) => options.format,
            Self::Validate(options) => options.format,
            Self::Apply(options) => options.operation.format,
            Self::Sort(options) => options.operation.format,
            Self::Rename(options) => options.operation.format,
            Self::Categories(options) => match &options.command {
                CategoriesCommand::List { format } => *format,
            },
            Self::Presets(options) => match &options.command {
                PresetsCommand::List { format } => *format,
                PresetsCommand::Save(_)
                | PresetsCommand::Export { .. }
                | PresetsCommand::Remove(_) => OutputFormat::Human,
            },
            Self::Runs(options) => match &options.command {
                RunsCommand::List { format } | RunsCommand::Show { format, .. } => *format,
            },
            Self::Undo(options) => options.format,
            Self::Init | Self::Ask(_) | Self::Completions { .. } => OutputFormat::Human,
        }
    }
}

#[derive(Debug, Args)]
pub(super) struct AskArgs {
    /// Task to ask Dirule; reads standard input when omitted.
    #[arg(value_name = "PROMPT", num_args = 0..)]
    pub(super) prompt: Vec<String>,

    /// Directory to inspect and preview; defaults to the current directory.
    #[arg(long, value_name = "PATH")]
    pub(super) target: Option<PathBuf>,

    /// Select a profile from the user-level assistant config.
    #[arg(long, value_name = "NAME")]
    pub(super) profile: Option<String>,

    /// Read assistant profiles from this file instead of the user config path.
    #[arg(long, value_name = "FILE")]
    pub(super) assistant_config: Option<PathBuf>,

    #[command(flatten)]
    pub(super) rules: RuleSourceArgs,
}

#[derive(Debug, Args)]
pub(super) struct OperationArgs {
    /// Directory to check or organize.
    #[arg(short, long, value_name = "PATH")]
    pub(super) target: Option<PathBuf>,

    #[command(flatten)]
    pub(super) rules: RuleSourceArgs,

    /// Show rule ownership and inherited paths in text output.
    #[arg(short, long)]
    pub(super) verbose: bool,

    /// Output as human-readable text or JSON.
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
    pub(super) format: OutputFormat,
}

#[derive(Debug, Args)]
#[group(multiple = false)]
pub(super) struct RuleSourceArgs {
    /// Read rules from this TOML file.
    #[arg(short, long, value_name = "FILE")]
    pub(super) config: Option<PathBuf>,

    /// Select a named preset.
    #[arg(short, long, value_name = "NAME")]
    pub(super) preset: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct ValidateArgs {
    #[command(flatten)]
    pub(super) rules: RuleSourceArgs,

    /// Output as human-readable text or JSON.
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
    pub(super) format: OutputFormat,
}

#[derive(Debug, Args)]
pub(super) struct ApplyArgs {
    #[command(flatten)]
    pub(super) operation: OperationArgs,

    /// Skip the interactive confirmation prompt.
    #[arg(short, long)]
    pub(super) yes: bool,

    /// Choose whether conflicts abort the operation or skip affected paths.
    #[arg(long, value_enum, default_value_t = ConflictAction::Abort)]
    pub(super) on_conflict: ConflictAction,
}

#[derive(Debug, Args)]
pub(super) struct SortArgs {
    /// Extensions or categories to sort; omit them to use every category in the catalog.
    #[arg(
        value_name = "SELECTOR",
        num_args = 0..,
        conflicts_with = "by"
    )]
    pub(super) selectors: Vec<String>,

    /// Sort by a supported file property instead of selected extensions or categories.
    #[arg(long, value_enum, conflicts_with = "selectors")]
    pub(super) by: Option<SortBy>,

    #[command(flatten)]
    pub(super) operation: QuickOperationArgs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub(super) enum SortBy {
    Extension,
}

#[derive(Debug, Args)]
pub(super) struct RenameArgs {
    /// Built-in naming transformation.
    #[arg(value_enum)]
    pub(super) transform: NameTransform,

    #[command(flatten)]
    pub(super) operation: QuickOperationArgs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub(super) enum NameTransform {
    #[value(name = "snake-case")]
    Snake,
    #[value(name = "kebab-case")]
    Kebab,
    #[value(name = "camel-case")]
    Camel,
    #[value(name = "pascal-case")]
    Pascal,
}

impl From<NameTransform> for dirule_core::Transform {
    fn from(transform: NameTransform) -> Self {
        match transform {
            NameTransform::Snake => Self::SnakeCase,
            NameTransform::Kebab => Self::KebabCase,
            NameTransform::Camel => Self::CamelCase,
            NameTransform::Pascal => Self::PascalCase,
        }
    }
}

#[derive(Debug, Args)]
pub(super) struct QuickOperationArgs {
    /// Directory to process; defaults to the current directory.
    #[arg(short, long, value_name = "PATH")]
    pub(super) target: Option<PathBuf>,

    /// Include entries below the target directory.
    #[arg(long)]
    pub(super) recursive: bool,

    /// Apply the preview after confirmation.
    #[arg(long)]
    pub(super) apply: bool,

    /// Skip the confirmation prompt; requires --apply.
    #[arg(short, long, requires = "apply")]
    pub(super) yes: bool,

    /// Choose whether conflicts abort or skip affected paths.
    #[arg(long, value_enum, default_value_t = ConflictAction::Abort)]
    pub(super) on_conflict: ConflictAction,

    /// Output as human-readable text or JSON.
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
    pub(super) format: OutputFormat,
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

#[derive(Debug, Args)]
pub(super) struct CategoriesArgs {
    #[command(subcommand)]
    pub(super) command: CategoriesCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum CategoriesCommand {
    /// List effective built-in and user-defined categories.
    List {
        /// Output as human-readable text or JSON.
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
}

#[derive(Debug, Subcommand)]
pub(super) enum PresetsCommand {
    /// List available presets.
    List {
        /// Output as human-readable text or JSON.
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
    /// Save a project config as a global preset.
    Save(PresetSaveArgs),
    /// Export a saved preset as TOML.
    Export {
        /// Name of the preset to export.
        name: String,
    },
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
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
    /// Show the renames recorded for a run.
    Show {
        /// Identifier shown by `dirule runs list` or after `dirule apply`.
        run_id: String,

        /// Output as human-readable text or JSON.
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
}

#[derive(Debug, Args)]
pub(super) struct UndoArgs {
    /// Identifier shown by `dirule runs list` or after `dirule apply`.
    pub(super) run_id: String,

    /// Skip the interactive confirmation prompt.
    #[arg(short, long)]
    pub(super) yes: bool,

    /// Output as human-readable text or JSON.
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
    pub(super) format: OutputFormat,
}

#[derive(Debug, Args)]
pub(super) struct PresetSaveArgs {
    /// Name to assign to the preset.
    pub(super) name: String,

    /// Read rules from this TOML file instead of ./dirule.toml.
    #[arg(short, long, value_name = "FILE")]
    pub(super) config: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub(super) struct PresetRemoveArgs {
    /// Name of the preset to remove.
    pub(super) name: String,

    /// Remove without an interactive confirmation.
    #[arg(short, long)]
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
    use super::{
        ApplyArgs, AskArgs, CategoriesCommand, Cli, CliCommand, ConflictAction, OutputFormat,
        PresetsCommand, ValidateArgs,
    };
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
    fn ask_parses_target_profile_and_rule_source_options() {
        let cli = Cli::try_parse_from([
            "dirule",
            "ask",
            "--target",
            "/tmp/files",
            "--profile",
            "google",
            "--config",
            "rules.toml",
            "Explain these rules",
        ])
        .expect("valid ask options");

        let CliCommand::Ask(AskArgs {
            prompt,
            target,
            profile,
            rules,
            ..
        }) = cli.command
        else {
            panic!("expected ask command");
        };

        assert_eq!(prompt, ["Explain these rules"]);
        assert_eq!(target.as_deref(), Some(std::path::Path::new("/tmp/files")));
        assert_eq!(profile.as_deref(), Some("google"));
        assert_eq!(
            rules.config.as_deref(),
            Some(std::path::Path::new("rules.toml"))
        );
    }

    #[test]
    fn ask_rejects_config_and_preset_together() {
        let error = Cli::try_parse_from([
            "dirule",
            "ask",
            "--config",
            "rules.toml",
            "--preset",
            "downloads",
            "Explain the rules",
        ])
        .expect_err("config and preset conflict for ask");

        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn output_format_rejects_unknown_values() {
        let error = Cli::try_parse_from(["dirule", "check", "--format", "yaml"])
            .expect_err("unknown output format");

        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
    }

    #[test]
    fn validate_parses_config_source_and_output_format() {
        let cli = Cli::try_parse_from([
            "dirule",
            "validate",
            "--config",
            "rules.toml",
            "--format",
            "json",
        ])
        .expect("valid validate options");

        let CliCommand::Validate(ValidateArgs { rules, format }) = cli.command else {
            panic!("expected validate command");
        };

        assert_eq!(
            rules.config.as_deref(),
            Some(std::path::Path::new("rules.toml"))
        );
        assert_eq!(format, OutputFormat::Json);
    }

    #[test]
    fn category_listing_parses_json_output() {
        let cli = Cli::try_parse_from(["dirule", "categories", "list", "--format", "json"])
            .expect("valid category listing options");

        assert_eq!(cli.command.output_format(), OutputFormat::Json);
        assert!(matches!(
            cli.command,
            CliCommand::Categories(super::CategoriesArgs {
                command: CategoriesCommand::List {
                    format: OutputFormat::Json
                }
            })
        ));
    }

    #[test]
    fn quick_operations_parse_preview_and_apply_options() {
        let sort = Cli::try_parse_from([
            "dirule",
            "sort",
            "--by",
            "extension",
            "--target",
            "/tmp/files",
            "--recursive",
            "--apply",
            "--yes",
            "--format",
            "json",
        ])
        .expect("valid extension sort options");
        let CliCommand::Sort(sort) = sort.command else {
            panic!("expected sort command");
        };
        assert_eq!(sort.by, Some(super::SortBy::Extension));
        assert!(sort.selectors.is_empty());
        assert!(sort.operation.recursive);
        assert!(sort.operation.apply);
        assert!(sort.operation.yes);
        assert_eq!(sort.operation.format, OutputFormat::Json);

        let rename =
            Cli::try_parse_from(["dirule", "rename", "kebab-case", "--target", "/tmp/files"])
                .expect("valid rename preview options");
        let CliCommand::Rename(rename) = rename.command else {
            panic!("expected rename command");
        };
        assert_eq!(rename.transform, super::NameTransform::Kebab);
        assert!(!rename.operation.apply);
    }

    #[test]
    fn sort_parses_multiple_extension_and_category_selectors() {
        let cli = Cli::try_parse_from(["dirule", "sort", "pdf", "images", "videos", "--recursive"])
            .expect("valid sort selectors");
        let CliCommand::Sort(sort) = cli.command else {
            panic!("expected sort command");
        };

        assert_eq!(sort.selectors, ["pdf", "images", "videos"]);
        assert_eq!(sort.by, None);
        assert!(sort.operation.recursive);
    }

    #[test]
    fn sort_parses_without_selectors_to_use_the_effective_categories() {
        let cli = Cli::try_parse_from(["dirule", "sort"]).expect("bare sort is valid");
        let CliCommand::Sort(sort) = cli.command else {
            panic!("expected sort command");
        };

        assert!(sort.selectors.is_empty());
        assert_eq!(sort.by, None);
    }

    #[test]
    fn quick_operations_require_confirmation_skip_to_follow_apply() {
        let error = Cli::try_parse_from(["dirule", "rename", "snake-case", "--yes"])
            .expect_err("--yes without --apply");

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
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
