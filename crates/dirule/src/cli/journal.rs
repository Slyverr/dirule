use super::{args, output, paths, prompt};
use dirule_fs::JournalStore;
use std::error::Error;

pub(super) fn list(
    format: args::OutputFormat,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let runs = store()?.list()?;
    output::runs(&runs, format, color)?;
    Ok(())
}

pub(super) fn show(
    run_id: String,
    format: args::OutputFormat,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let details = store()?.get(&run_id)?;
    output::run_details(&details, format, color)?;
    Ok(())
}

pub(super) fn undo(
    options: args::UndoArgs,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    if !prompt::confirm(
        &format!("Undo apply run {}?", options.run_id),
        "use --yes to undo without an interactive terminal",
        options.yes,
    )? {
        output::undo_result(&options.run_id, "cancelled", &[], options.format, color)?;
        return Ok(());
    }

    let report = store()?.undo(&options.run_id)?;
    output::undo_result(
        &options.run_id,
        "undone",
        &report.retained_directories,
        options.format,
        color,
    )?;
    Ok(())
}

pub(super) fn store() -> Result<JournalStore, Box<dyn Error>> {
    Ok(JournalStore::new(paths::journal_directory()?))
}
