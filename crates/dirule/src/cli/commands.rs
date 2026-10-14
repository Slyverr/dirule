use super::{args, config, journal, output, presets, prompt};
use clap::CommandFactory;
use dirule_fs::ConflictPolicy;
use std::error::Error;
use std::fmt;
use std::io::{self, Write};

#[derive(Debug)]
pub(super) struct CheckFailure;

impl fmt::Display for CheckFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("target does not satisfy the configured rules")
    }
}

impl Error for CheckFailure {}

pub(super) fn run(
    command: args::CliCommand,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    match command {
        args::CliCommand::Init => config::initialize(color),
        args::CliCommand::Check(options) => {
            let format = options.format;
            let verbose = options.verbose;
            let preview = config::evaluate(options)?;
            let target = &preview.target;
            let result = &preview.plan;

            output::plan(
                target,
                result,
                format,
                color,
                output::PlanContext::Check,
                verbose,
            )?;
            if result.has_changes() || !result.is_conflict_free() {
                return Err(Box::new(CheckFailure));
            }

            Ok(())
        }
        args::CliCommand::Validate(options) => config::validate(options, color),
        args::CliCommand::Plan(options) => {
            let format = options.format;
            let verbose = options.verbose;
            let preview = config::evaluate(options)?;
            let target = &preview.target;
            let result = &preview.plan;

            output::plan(
                target,
                result,
                format,
                color,
                output::PlanContext::Preview,
                verbose,
            )?;
            if !result.is_conflict_free() {
                return Err(format!("plan contains {} conflict(s)", result.conflicts.len()).into());
            }
            Ok(())
        }
        args::CliCommand::Apply(options) => {
            let args::ApplyArgs {
                operation,
                yes,
                on_conflict,
            } = options;
            let format = operation.format;
            let verbose = operation.verbose;
            let preview = config::evaluate(operation)?;
            let target = &preview.target;
            let result = &preview.plan;

            if format == args::OutputFormat::Human {
                output::plan(
                    target,
                    result,
                    format,
                    color,
                    output::PlanContext::Apply,
                    verbose,
                )?;
            } else if output::json_apply_preview_needed(format, yes, result) {
                output::plan_to_stderr(target, result, color, output::PlanContext::Apply, verbose)?;
            }
            if !result.is_conflict_free() && on_conflict == args::ConflictAction::Abort {
                return Err(format!("plan contains {} conflict(s)", result.conflicts.len()).into());
            }
            if !result.has_changes() {
                output::apply_result(
                    target,
                    result,
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
            if !prompt::confirm(
                "Apply this plan?",
                "use --yes to apply without an interactive terminal",
                yes,
            )? {
                output::apply_result(
                    target,
                    result,
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
                args::ConflictAction::Abort => ConflictPolicy::Abort,
                args::ConflictAction::Skip => ConflictPolicy::Skip,
            };
            let store = journal::store()?;
            let applied = crate::application::apply_authorized(preview, policy, &store)?;
            output::apply_result(
                &applied.target,
                &applied.plan,
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
        args::CliCommand::Presets(options) => match options.command {
            args::PresetsCommand::List { format } => presets::list(format, color),
            args::PresetsCommand::Save(options) => presets::save(options, color),
            args::PresetsCommand::Export { name } => presets::export(name),
            args::PresetsCommand::Remove(options) => presets::remove(options, color),
        },
        args::CliCommand::Runs(options) => match options.command {
            args::RunsCommand::List { format } => journal::list(format, color),
            args::RunsCommand::Show { run_id, format } => journal::show(run_id, format, color),
        },
        args::CliCommand::Undo(options) => journal::undo(options, color),
        args::CliCommand::Completions { shell } => {
            let mut command = args::Cli::command();
            let binary_name = command.get_name().to_owned();
            let mut script = Vec::new();
            clap_complete::generate(shell, &mut command, binary_name, &mut script);
            io::stdout().lock().write_all(&script)?;
            Ok(())
        }
    }
}
