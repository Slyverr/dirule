mod args;
mod commands;
mod config;
mod journal;
mod output;
mod paths;
mod presets;
mod prompt;

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
    let _ = output::error(&error.to_string(), error.color);
}

impl CliError {
    pub fn exit_code(&self) -> u8 {
        self.exit_code
    }
}

pub fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let color = cli.color;

    match commands::run(cli.command, color) {
        Ok(()) => Ok(()),
        Err(source) if output::is_broken_pipe(source.as_ref()) => Ok(()),
        Err(source) => {
            let exit_code = if source.downcast_ref::<commands::CheckFailure>().is_some() {
                CHECK_FAILURE_EXIT_CODE
            } else {
                COMMAND_ERROR_EXIT_CODE
            };

            Err(CliError {
                source,
                color,
                exit_code,
            })
        }
    }
}
