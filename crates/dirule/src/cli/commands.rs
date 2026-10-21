use super::{args, assistant, categories, config, journal, output, presets};
use clap::CommandFactory;
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
            let target = preview.target();
            let result = preview.plan();

            output::plan(
                target,
                result,
                preview.reference_updates(),
                format,
                color,
                output::PlanContext::Check,
                verbose,
            )?;
            if result.has_changes() || result.has_check_violations() || !result.is_conflict_free() {
                return Err(Box::new(CheckFailure));
            }

            Ok(())
        }
        args::CliCommand::Validate(options) => config::validate(options, color),
        args::CliCommand::Plan(options) => {
            let format = options.format;
            let verbose = options.verbose;
            let preview = config::evaluate(options)?;
            let target = preview.target();
            let result = preview.plan();

            output::plan(
                target,
                result,
                preview.reference_updates(),
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
            super::apply::execute(
                preview,
                super::apply::ApplyOptions {
                    format,
                    color,
                    yes,
                    on_conflict,
                    verbose,
                },
            )
        }
        args::CliCommand::Sort(options) => super::quick::sort(options, color),
        args::CliCommand::Rename(options) => super::quick::rename(options, color),
        args::CliCommand::Categories(options) => match options.command {
            args::CategoriesCommand::List { format } => categories::list(format, color),
        },
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
        args::CliCommand::Ask(options) => assistant::ask(options, color),
        args::CliCommand::Lsp(options) => match options.command {
            args::LspCommand::Status(status) => config::lsp_status(status.rules),
        },
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
