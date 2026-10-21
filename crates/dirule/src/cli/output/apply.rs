use super::super::args::{ColorChoice, OutputFormat};
use super::{Colorizer, cancelled, json, no_changes};
use dirule_app::ReferenceUpdatePreview;
use dirule_core::Plan;
use std::path::Path;

#[derive(Clone, Copy)]
pub(in crate::cli) enum ApplyStatus {
    Applied,
    NoChanges,
    Cancelled,
}

pub(in crate::cli) struct ApplySummary<'a> {
    pub(in crate::cli) status: ApplyStatus,
    pub(in crate::cli) moved: usize,
    pub(in crate::cli) reference_files: usize,
    pub(in crate::cli) skipped: usize,
    pub(in crate::cli) run_id: Option<&'a str>,
}

pub(in crate::cli) fn apply_result(
    target: &Path,
    plan: &Plan,
    reference_updates: &ReferenceUpdatePreview,
    summary: ApplySummary<'_>,
    format: OutputFormat,
    color: ColorChoice,
) -> Result<(), Box<dyn std::error::Error>> {
    if format == OutputFormat::Json {
        json::write_apply(target, plan, reference_updates, &summary)?;

        return Ok(());
    }

    let styles = Colorizer::stdout(color);

    match summary.status {
        ApplyStatus::Applied => {
            if summary.skipped == 0 {
                if summary.reference_files == 0 {
                    println!(
                        "{} Applied {} rename(s).",
                        styles.style("Done.", "32"),
                        summary.moved
                    );
                } else {
                    println!(
                        "{} Applied {} rename(s) and updated {} source file(s).",
                        styles.style("Done.", "32"),
                        summary.moved,
                        summary.reference_files
                    );
                }
            } else if summary.reference_files == 0 {
                println!(
                    "{} Applied {} rename(s); skipped {} conflict(s).",
                    styles.style("Done.", "32"),
                    summary.moved,
                    summary.skipped
                );
            } else {
                println!(
                    "{} Applied {} rename(s), updated {} source file(s), and skipped {} conflict(s).",
                    styles.style("Done.", "32"),
                    summary.moved,
                    summary.reference_files,
                    summary.skipped
                );
            }
            if let Some(run_id) = summary.run_id {
                println!("Undo with: dirule undo {run_id}");
            }
        }
        ApplyStatus::NoChanges => no_changes(color)?,
        ApplyStatus::Cancelled => cancelled("Apply", color)?,
    }
    Ok(())
}

impl ApplyStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::NoChanges => "no_changes",
            Self::Cancelled => "cancelled",
        }
    }
}
