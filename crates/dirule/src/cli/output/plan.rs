use super::super::args::{ColorChoice, OutputFormat};
use super::{Colorizer, json};
use dirule_core::Plan;
use std::io::{self, Write};
use std::path::Path;

#[derive(Clone, Copy)]
pub(in crate::cli) enum PlanContext {
    Preview,
    Check,
    Apply,
}

pub(in crate::cli) fn plan(
    target: &Path,
    plan: &Plan,
    format: OutputFormat,
    color: ColorChoice,
    context: PlanContext,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if format == OutputFormat::Json {
        json::write_plan(target, plan)?;

        return Ok(());
    }

    let stdout = io::stdout();
    let mut writer = stdout.lock();

    write_text_plan(
        &mut writer,
        target,
        plan,
        Colorizer::stdout(color),
        context,
        verbose,
    )?;

    Ok(())
}

pub(in crate::cli) fn plan_to_stderr(
    target: &Path,
    plan: &Plan,
    color: ColorChoice,
    context: PlanContext,
    verbose: bool,
) -> io::Result<()> {
    let stderr = io::stderr();
    let mut writer = stderr.lock();

    write_text_plan(
        &mut writer,
        target,
        plan,
        Colorizer::stderr(color),
        context,
        verbose,
    )
}

pub(in crate::cli) fn json_apply_preview_needed(
    format: OutputFormat,
    yes: bool,
    plan: &Plan,
) -> bool {
    format == OutputFormat::Json && !yes && (plan.has_changes() || !plan.is_conflict_free())
}

pub(super) fn write_text_plan<W: Write>(
    writer: &mut W,
    target: &Path,
    plan: &Plan,
    styles: Colorizer,
    context: PlanContext,
    verbose: bool,
) -> io::Result<()> {
    let changes = plan
        .entries
        .iter()
        .filter(|entry| entry.source != entry.destination)
        .count();
    let (label, heading_color) = match context {
        PlanContext::Preview => ("Plan preview", "36"),
        PlanContext::Check
            if changes == 0 && plan.conflicts.is_empty() && !plan.has_check_violations() =>
        {
            ("Check passed", "32")
        }
        PlanContext::Check => ("Check failed", "31"),
        PlanContext::Apply => ("Apply preview", "36"),
    };
    writeln!(writer, "{}", styles.style(label, heading_color))?;
    writeln!(
        writer,
        "  {} {}",
        styles.style("Target:", "36"),
        styles.style(target.display(), "36")
    )?;
    writeln!(
        writer,
        "  {} rename(s), {} conflict(s), {} finding(s)",
        changes,
        plan.conflicts.len(),
        plan.findings.len()
    )?;

    if changes > 0 {
        writeln!(writer, "\nChanges:")?;
        for entry in &plan.entries {
            if entry.source != entry.destination {
                let attribution = if verbose {
                    let detail = entry
                        .rule
                        .as_deref()
                        .map(|rule| format!("rule: {rule}"))
                        .unwrap_or_else(|| "inherited from directory rename".to_owned());
                    format!(" [{}]", styles.style(&detail, "36"))
                } else {
                    String::new()
                };

                writeln!(
                    writer,
                    "  {} -> {}{}",
                    styles.style(entry.source.display(), "31"),
                    styles.style(entry.destination.display(), "32"),
                    attribution
                )?;
            }
        }
    } else if plan.conflicts.is_empty() && plan.findings.is_empty() {
        writeln!(writer, "\nNo changes needed.")?;
    }

    if !plan.conflicts.is_empty() {
        writeln!(writer, "\nConflicts:")?;
        for conflict in &plan.conflicts {
            let related_paths = conflict
                .related_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");

            writeln!(
                writer,
                "  {} {} ({})",
                styles.style("!", "31"),
                conflict.path.display(),
                conflict.reason
            )?;
            writeln!(writer, "    Related paths: {related_paths}")?;
            writeln!(
                writer,
                "    Suggestion: {}",
                styles.style(conflict.kind.suggestion(), "36")
            )?;
        }
    }

    if !plan.findings.is_empty() {
        writeln!(writer, "\nFindings:")?;
        for finding in &plan.findings {
            writeln!(writer, "  {} {finding}", styles.style("i", "36"))?;
        }
    }

    if matches!(context, PlanContext::Check)
        && changes == 0
        && plan.conflicts.is_empty()
        && !plan.has_check_violations()
    {
        writeln!(writer, "Target matches the configured rules.")?;
    }

    Ok(())
}
