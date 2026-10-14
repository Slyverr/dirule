use super::{args, output};
use std::error::Error;

pub(super) fn list(
    format: args::OutputFormat,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let categories = dirule_app::CategoryCatalog::user_if_available()?;

    output::category_list(&categories, format, color)?;

    Ok(())
}
