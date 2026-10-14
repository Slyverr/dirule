use super::super::args::{ColorChoice, OutputFormat};
use super::{CURRENT_OUTPUT_VERSION, Colorizer};
use std::io;

pub(in crate::cli) fn error(
    message: &str,
    color: ColorChoice,
    format: OutputFormat,
    code: &str,
    exit_code: u8,
) -> io::Result<()> {
    if format == OutputFormat::Json {
        let document = serde_json::json!({
            "schema_version": CURRENT_OUTPUT_VERSION,
            "error": {
                "code": code,
                "message": message,
                "exit_code": exit_code,
            },
        });
        let output = serde_json::to_string_pretty(&document).expect("error is serializable");
        eprintln!("{output}");

        return Ok(());
    }

    let styles = Colorizer::stderr(color);

    eprintln!("{}: {message}", styles.style("dirule", "31"));

    Ok(())
}
