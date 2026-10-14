use super::super::args::{ColorChoice, OutputFormat};
use super::{CURRENT_OUTPUT_VERSION, Colorizer, Styled};
use std::io;
use std::path::Path;

fn preset_name(name: &str, styles: Colorizer) -> Styled<&str> {
    styles.style(name, "36")
}

pub(in crate::cli) fn presets(
    names: &[String],
    format: OutputFormat,
    color: ColorChoice,
) -> io::Result<()> {
    if format == OutputFormat::Json {
        let value = serde_json::json!({
            "schema_version": CURRENT_OUTPUT_VERSION,
            "presets": names,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("preset listing is serializable")
        );
        return Ok(());
    }

    let styles = Colorizer::stdout(color);

    println!("Presets");
    if names.is_empty() {
        println!("  No user presets found.");
    } else {
        for name in names {
            println!("  {}", preset_name(name, styles));
        }
    }

    Ok(())
}

pub(in crate::cli) fn saved_preset(name: &str, path: &Path, color: ColorChoice) -> io::Result<()> {
    let styles = Colorizer::stdout(color);

    println!(
        "{} Saved preset {} to {}",
        styles.style("Done.", "32"),
        preset_name(name, styles),
        path.display()
    );

    Ok(())
}

pub(in crate::cli) fn removed_preset(name: &str, color: ColorChoice) -> io::Result<()> {
    let styles = Colorizer::stdout(color);

    println!(
        "{} Removed preset {}",
        styles.style("Done.", "32"),
        preset_name(name, styles)
    );

    Ok(())
}
