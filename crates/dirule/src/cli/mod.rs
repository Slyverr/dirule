mod apply;
mod args;
mod commands;
mod config;
mod journal;
mod output;
mod paths;
mod presets;
mod prompt;
mod quick;

use self::args::{Cli, ColorChoice};
use clap::Parser;
use std::error::Error;
use std::fmt;

const CHECK_FAILURE_EXIT_CODE: u8 = 1;
const COMMAND_ERROR_EXIT_CODE: u8 = 2;

#[derive(Debug)]
pub struct CliError {
    source: Box<dyn Error>,
    color: ColorChoice,
    format: args::OutputFormat,
    code: &'static str,
    exit_code: u8,
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
        error.code,
        error.exit_code,
    );
}

impl CliError {
    pub fn exit_code(&self) -> u8 {
        self.exit_code
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
            let (code, exit_code) = if source.downcast_ref::<commands::CheckFailure>().is_some() {
                ("check_failed", CHECK_FAILURE_EXIT_CODE)
            } else {
                ("command_error", COMMAND_ERROR_EXIT_CODE)
            };

            Err(CliError {
                source,
                color,
                format,
                code,
                exit_code,
            })
        }
    }
}
