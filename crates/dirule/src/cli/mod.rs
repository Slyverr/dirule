mod apply;
mod args;
mod assistant;
mod categories;
mod commands;
mod config;
mod error;
mod journal;
mod output;
mod presets;
mod prompt;
mod quick;

use self::args::Cli;
pub use self::error::{CliError, print_error};
use clap::Parser;

pub fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let color = cli.color;
    let format = cli.command.output_format();

    match commands::run(cli.command, color) {
        Ok(()) => Ok(()),
        Err(source) if output::is_broken_pipe(source.as_ref()) => Ok(()),
        Err(source) => Err(CliError::new(source, color, format)),
    }
}
