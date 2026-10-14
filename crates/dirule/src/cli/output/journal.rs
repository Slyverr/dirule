use super::super::args::{ColorChoice, OutputFormat};
use super::{CURRENT_OUTPUT_VERSION, Colorizer, cancelled};
use dirule_fs::{JournalDetails, JournalRun, JournalStatus};
use std::io;
use std::path::{Path, PathBuf};

pub(in crate::cli) fn runs(
    runs: &[JournalRun],
    format: OutputFormat,
    color: ColorChoice,
) -> io::Result<()> {
    if format == OutputFormat::Json {
        let value = serde_json::json!({
            "schema_version": CURRENT_OUTPUT_VERSION,
            "runs": runs.iter().map(|run| serde_json::json!({
                "id": run.id,
                "target": run.target,
                "started_at": run.started_at,
                "status": status_name(run.status),
                "operations": run.operations,
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("run listing is serializable")
        );
        return Ok(());
    }

    if runs.is_empty() {
        println!("No apply runs found.");
        return Ok(());
    }

    let styles = Colorizer::stdout(color);

    println!("Apply runs");
    for run in runs {
        println!(
            "  {}  {}  {} rename(s)  started {}  {}",
            styles.style(&run.id, "36"),
            status_name(run.status),
            run.operations,
            run.started_at,
            run.target.display()
        );
    }

    Ok(())
}

pub(in crate::cli) fn run_details(
    details: &JournalDetails,
    format: OutputFormat,
    color: ColorChoice,
) -> Result<(), Box<dyn std::error::Error>> {
    if format == OutputFormat::Json {
        let value = serde_json::json!({
            "schema_version": CURRENT_OUTPUT_VERSION,
            "run": {
                "id": details.run.id,
                "target": details.run.target,
                "started_at": details.run.started_at,
                "status": status_name(details.run.status),
                "operations": details.run.operations,
            },
            "changes": details.operations.iter().map(|operation| serde_json::json!({
                "source": portable_path(&operation.source),
                "destination": portable_path(&operation.destination),
                "kind": operation.kind,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    let styles = Colorizer::stdout(color);

    println!("Apply run {}", styles.style(&details.run.id, "36"));
    println!("  Status: {}", status_name(details.run.status));
    println!("  Target: {}", details.run.target.display());
    println!("  Renames: {}", details.run.operations);
    if details.operations.is_empty() {
        println!("\nNo rename operations recorded.");
    } else {
        println!("\nChanges:");
        for operation in &details.operations {
            println!(
                "  {} -> {} ({})",
                styles.style(portable_path(&operation.source), "31"),
                styles.style(portable_path(&operation.destination), "32"),
                operation_kind(operation.kind)
            );
        }
    }

    Ok(())
}

fn operation_kind(kind: dirule_core::EntryKind) -> &'static str {
    match kind {
        dirule_core::EntryKind::File => "file",
        dirule_core::EntryKind::Directory => "directory",
        dirule_core::EntryKind::Symlink => "symlink",
    }
}

pub(in crate::cli) fn undo_result(
    run_id: &str,
    status: &str,
    retained_directories: &[PathBuf],
    format: OutputFormat,
    color: ColorChoice,
) -> Result<(), Box<dyn std::error::Error>> {
    if format == OutputFormat::Json {
        let value = serde_json::json!({
            "schema_version": CURRENT_OUTPUT_VERSION,
            "run_id": run_id,
            "status": status,
            "retained_directories": retained_directories
                .iter()
                .map(|path| portable_path(path.as_path()))
                .collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if status == "undone" {
        let styles = Colorizer::stdout(color);

        println!(
            "{} Undid run {}.",
            styles.style("Done.", "32"),
            styles.style(run_id, "36")
        );
        for directory in retained_directories {
            println!(
                "{} Left non-empty directory {} in place.",
                styles.style("Note:", "33"),
                directory.display()
            );
        }
    } else {
        cancelled("Undo", color)?;
    }
    Ok(())
}

fn status_name(status: JournalStatus) -> &'static str {
    match status {
        JournalStatus::Applying => "applying",
        JournalStatus::Applied => "applied",
        JournalStatus::FailedRolledBack => "failed_rolled_back",
        JournalStatus::Interrupted => "interrupted",
        JournalStatus::Undoing => "undoing",
        JournalStatus::PartiallyRecovered => "partially_recovered",
        JournalStatus::Undone => "undone",
    }
}

fn portable_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
