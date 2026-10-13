use std::process::ExitCode;

fn main() -> ExitCode {
    match dirule::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            dirule::cli::print_error(&error);
            ExitCode::from(2)
        }
    }
}
