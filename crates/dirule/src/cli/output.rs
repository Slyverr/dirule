use super::args::{ColorChoice, OutputFormat};
use dirule_core::Plan;
use dirule_fs::{JournalRun, JournalStatus};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

const CURRENT_OUTPUT_VERSION: &str = "1.0";

#[derive(Clone, Copy)]
pub(super) enum PlanContext {
    Preview,
    Check,
    Apply,
}

#[derive(Clone, Copy)]
pub(super) enum ApplyStatus {
    Applied,
    NoChanges,
    Cancelled,
}

pub(super) struct ApplySummary<'a> {
    pub(super) status: ApplyStatus,
    pub(super) moved: usize,
    pub(super) skipped: usize,
    pub(super) run_id: Option<&'a str>,
}

pub(super) fn created_config(path: &Path, color: ColorChoice) {
    println!("{} {}", paint("Created", "32", color), path.display());
}

pub(super) fn preset_name(name: &str, color: ColorChoice) -> String {
    paint(name, "36", color)
}

pub(super) fn saved_preset(name: &str, path: &Path, color: ColorChoice) {
    println!(
        "{} Saved preset {} to {}",
        paint("Done.", "32", color),
        preset_name(name, color),
        path.display()
    );
}

pub(super) fn removed_preset(name: &str, color: ColorChoice) {
    println!(
        "{} Removed preset {}",
        paint("Done.", "32", color),
        preset_name(name, color)
    );
}

pub(super) fn error(message: &str, color: ColorChoice) {
    let enabled = color_enabled(color, std::io::stderr().is_terminal());
    if enabled {
        eprintln!("\x1b[31mdirule\x1b[0m: {message}");
    } else {
        eprintln!("dirule: {message}");
    }
}

pub(super) fn plan(
    target: &Path,
    plan: &Plan,
    format: OutputFormat,
    color: ColorChoice,
    context: PlanContext,
) -> Result<(), Box<dyn std::error::Error>> {
    if format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&plan_value(target, plan))?
        );
        return Ok(());
    }

    let changes = plan
        .entries
        .iter()
        .filter(|entry| entry.source != entry.destination)
        .count();
    let (label, heading_color) = match context {
        PlanContext::Preview => ("Plan preview", "36"),
        PlanContext::Check if changes == 0 && plan.conflicts.is_empty() => ("Check passed", "32"),
        PlanContext::Check => ("Check failed", "31"),
        PlanContext::Apply => ("Apply preview", "36"),
    };
    println!("{}", paint(label, heading_color, color));
    println!(
        "  {} {}",
        paint("Target:", "36", color),
        paint(&target.display().to_string(), "36", color)
    );
    println!(
        "  {} rename(s), {} conflict(s), {} finding(s)",
        changes,
        plan.conflicts.len(),
        plan.findings.len()
    );

    if changes > 0 {
        println!("\nChanges:");
        for entry in &plan.entries {
            if entry.source != entry.destination {
                println!(
                    "  {} -> {}",
                    paint(&entry.source.display().to_string(), "31", color),
                    paint(&entry.destination.display().to_string(), "32", color)
                );
            }
        }
    } else if plan.conflicts.is_empty() && plan.findings.is_empty() {
        println!("\nNo changes needed.");
    }

    if !plan.conflicts.is_empty() {
        println!("\nConflicts:");
        for conflict in &plan.conflicts {
            println!(
                "  {} {} ({})",
                paint("!", "31", color),
                conflict.path.display(),
                conflict.reason
            );
        }
    }

    if !plan.findings.is_empty() {
        println!("\nFindings:");
        for finding in &plan.findings {
            println!("  {} {finding}", paint("i", "36", color));
        }
    }

    if matches!(context, PlanContext::Check) && changes == 0 && plan.conflicts.is_empty() {
        println!("Target matches the configured rules.");
    }

    Ok(())
}

pub(super) fn apply_result(
    target: &Path,
    plan: &Plan,
    summary: ApplySummary<'_>,
    format: OutputFormat,
    color: ColorChoice,
) -> Result<(), Box<dyn std::error::Error>> {
    if format == OutputFormat::Json {
        let value = serde_json::json!({
            "schema_version": CURRENT_OUTPUT_VERSION,
            "plan": plan_value(target, plan),
            "execution": {
                "status": summary.status.as_str(),
                "moved": summary.moved,
                "skipped": summary.skipped,
                "run_id": summary.run_id,
            }
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    match summary.status {
        ApplyStatus::Applied => {
            if summary.skipped == 0 {
                println!(
                    "{} Applied {} rename(s).",
                    paint("Done.", "32", color),
                    summary.moved
                );
            } else {
                println!(
                    "{} Applied {} rename(s); skipped {} conflict(s).",
                    paint("Done.", "32", color),
                    summary.moved,
                    summary.skipped
                );
            }
            if let Some(run_id) = summary.run_id {
                println!("Undo with: dirule undo {run_id}");
            }
        }
        ApplyStatus::NoChanges => no_changes(color),
        ApplyStatus::Cancelled => cancelled("Apply", color),
    }
    Ok(())
}

impl ApplyStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::NoChanges => "no_changes",
            Self::Cancelled => "cancelled",
        }
    }
}

fn plan_value(target: &Path, plan: &Plan) -> serde_json::Value {
    serde_json::json!({
        "schema_version": CURRENT_OUTPUT_VERSION,
        "target": target,
        "entries": plan.entries.iter().map(|entry| serde_json::json!({
            "source": portable_path(&entry.source),
            "destination": portable_path(&entry.destination),
            "kind": entry.kind,
            "rule": entry.rule,
        })).collect::<Vec<_>>(),
        "conflicts": plan.conflicts.iter().map(|conflict| serde_json::json!({
            "path": portable_path(&conflict.path),
            "reason": conflict.reason,
        })).collect::<Vec<_>>(),
        "findings": plan.findings,
    })
}

pub(super) fn runs(runs: &[JournalRun], format: OutputFormat, color: ColorChoice) {
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
        return;
    }

    if runs.is_empty() {
        println!("No apply runs found.");
        return;
    }
    println!("Apply runs");
    for run in runs {
        println!(
            "  {}  {}  {} rename(s)  started {}  {}",
            paint(&run.id, "36", color),
            status_name(run.status),
            run.operations,
            run.started_at,
            run.target.display()
        );
    }
}

pub(super) fn undo_result(
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
        println!(
            "{} Undid run {}.",
            paint("Done.", "32", color),
            paint(run_id, "36", color)
        );
        for directory in retained_directories {
            println!(
                "{} Left non-empty directory {} in place.",
                paint("Note:", "33", color),
                directory.display()
            );
        }
    } else {
        cancelled("Undo", color);
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

pub(super) fn no_changes(color: ColorChoice) {
    println!("{}", paint("No changes to apply.", "32", color));
}

pub(super) fn cancelled(action: &str, color: ColorChoice) {
    println!("{}.", paint(&format!("{action} cancelled"), "33", color));
}

fn paint(text: &str, ansi_code: &str, choice: ColorChoice) -> String {
    let enabled = color_enabled(choice, std::io::stdout().is_terminal());
    if enabled {
        format!("\x1b[{ansi_code}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

fn color_enabled(choice: ColorChoice, is_terminal: bool) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            is_terminal
                && std::env::var_os("NO_COLOR").is_none()
                && !matches!(std::env::var("TERM").as_deref(), Ok("dumb"))
        }
    }
}
