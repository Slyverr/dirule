use super::{CURRENT_OUTPUT_VERSION, Colorizer};
use crate::cli::args::{ColorChoice, OutputFormat};
use dirule_app::{CategoryCatalog, SortCategory};
use std::io;

pub(in crate::cli) fn category_list(
    catalog: &CategoryCatalog,
    format: OutputFormat,
    color: ColorChoice,
) -> io::Result<()> {
    if format == OutputFormat::Json {
        let categories = catalog
            .iter()
            .map(|category| {
                serde_json::json!({
                    "destination": category.destination,
                    "extensions": category.extensions,
                    "name": category.name,
                })
            })
            .collect::<Vec<_>>();
        let document = serde_json::json!({
            "categories": categories,
            "schema_version": CURRENT_OUTPUT_VERSION,
        });

        println!(
            "{}",
            serde_json::to_string_pretty(&document).expect("category list is serializable")
        );

        return Ok(());
    }

    let styles = Colorizer::stdout(color);
    let categories = catalog.iter().collect::<Vec<_>>();
    let name_width = categories
        .iter()
        .map(|category| category.name.len())
        .max()
        .unwrap_or_default();

    println!("Categories");
    for category in categories {
        write_category(category, name_width, styles)?;
    }

    Ok(())
}

fn write_category(category: &SortCategory, name_width: usize, styles: Colorizer) -> io::Result<()> {
    let name = format!("{:width$}", category.name, width = name_width);

    println!("  {} -> {}", styles.style(name, "36"), category.destination);
    println!("    Extensions: {}", category.extensions.join(", "));

    Ok(())
}
