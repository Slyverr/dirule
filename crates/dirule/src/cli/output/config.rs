use super::super::args::{ColorChoice, OutputFormat};
use super::{CURRENT_OUTPUT_VERSION, Colorizer};
use std::io;
use std::path::Path;

pub(in crate::cli) fn created_config(path: &Path, color: ColorChoice) -> io::Result<()> {
    let styles = Colorizer::stdout(color);

    println!("{} {}", styles.style("Created", "32"), path.display());

    Ok(())
}

pub(in crate::cli) fn validated_config(
    source_kind: &str,
    source_name: &str,
    rule_count: usize,
    format: OutputFormat,
    color: ColorChoice,
) -> io::Result<()> {
    if format == OutputFormat::Json {
        let value = serde_json::json!({
            "schema_version": CURRENT_OUTPUT_VERSION,
            "valid": true,
            "source": {
                "kind": source_kind,
                "name": source_name,
            },
            "rule_count": rule_count,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("validation result is serializable")
        );

        return Ok(());
    }

    let label = match source_kind {
        "preset" => "Preset is valid:",
        _ => "Configuration is valid:",
    };
    let rules = if rule_count == 1 { "rule" } else { "rules" };
    let styles = Colorizer::stdout(color);

    println!(
        "{} {} ({} {})",
        styles.style(label, "32"),
        source_name,
        rule_count,
        rules
    );

    Ok(())
}
