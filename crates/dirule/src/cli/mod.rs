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
    let _ = output::error(&error.to_string(), error.color);
}

pub fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let color = cli.color;

    match commands::run(cli.command, color) {
        Ok(()) => Ok(()),
        Err(source) if output::is_broken_pipe(source.as_ref()) => Ok(()),
        Err(source) => Err(CliError { source, color }),
    }
}
