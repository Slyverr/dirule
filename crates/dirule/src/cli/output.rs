use super::args::{ColorChoice, OutputFormat};
use dirule_core::Plan;
use std::io::IsTerminal;
use std::path::Path;

#[derive(Clone, Copy)]
pub(super) enum PlanContext {
    Preview,
    Check,
    Apply,
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
    plan: &Plan,
    format: OutputFormat,
    color: ColorChoice,
    context: PlanContext,
) -> Result<(), Box<dyn std::error::Error>> {
    if format == OutputFormat::Json {
        let report = serde_json::json!({
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
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
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

pub(super) fn applied(moved: usize, skipped: usize, color: ColorChoice) {
    if skipped == 0 {
        println!("{} Applied {moved} rename(s).", paint("Done.", "32", color));
    } else {
        println!(
            "{} Applied {moved} rename(s); skipped {skipped} conflict(s).",
            paint("Done.", "32", color)
        );
    }
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
