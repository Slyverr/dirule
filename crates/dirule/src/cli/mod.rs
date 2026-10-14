mod apply;
mod args;
mod commands;
mod config;
mod journal;
mod output;
mod presets;
mod prompt;
mod quick;

use self::args::{Cli, ColorChoice};
use clap::Parser;
use std::error::Error;
use std::fmt;

const CHECK_FAILURE_EXIT_CODE: u8 = 1;
const COMMAND_ERROR_EXIT_CODE: u8 = 2;

#[derive(Clone, Copy, Debug)]
enum CliErrorKind {
    CheckFailure,
    CommandError,
}

impl CliErrorKind {
    fn code(self) -> &'static str {
        match self {
            Self::CheckFailure => "check_failed",
            Self::CommandError => "command_error",
        }
    }

    fn exit_code(self) -> u8 {
        match self {
            Self::CheckFailure => CHECK_FAILURE_EXIT_CODE,
            Self::CommandError => COMMAND_ERROR_EXIT_CODE,
        }
    }
}

#[derive(Debug)]
pub struct CliError {
    source: Box<dyn Error>,
    color: ColorChoice,
    format: args::OutputFormat,
    kind: CliErrorKind,
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
    let _ = output::error(
        &error.to_string(),
        error.color,
        error.format,
        error.kind.code(),
        error.kind.exit_code(),
    );
}

impl CliError {
    pub fn exit_code(&self) -> u8 {
        self.kind.exit_code()
    }
}

pub fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let color = cli.color;
    let format = cli.command.output_format();

    match commands::run(cli.command, color) {
        Ok(()) => Ok(()),
        Err(source) if output::is_broken_pipe(source.as_ref()) => Ok(()),
        Err(source) => {
            let kind = if source.downcast_ref::<commands::CheckFailure>().is_some() {
                CliErrorKind::CheckFailure
            } else {
                CliErrorKind::CommandError
            };

            Err(CliError {
                source,
                color,
                format,
                kind,
            })
        }
    }
}
