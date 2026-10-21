use super::super::args::{ColorChoice, OutputFormat};
use super::{Colorizer, json};
use dirule_app::ReferenceUpdatePreview;
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
    reference_updates: &ReferenceUpdatePreview,
    format: OutputFormat,
    color: ColorChoice,
    context: PlanContext,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if format == OutputFormat::Json {
        json::write_plan(target, plan, reference_updates)?;

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
    write_reference_updates(&mut writer, reference_updates, Colorizer::stdout(color))?;

    Ok(())
}

pub(in crate::cli) fn plan_to_stderr(
    target: &Path,
    plan: &Plan,
    reference_updates: &ReferenceUpdatePreview,
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
    )?;
    write_reference_updates(&mut writer, reference_updates, Colorizer::stderr(color))
}

fn write_reference_updates<W: Write>(
    writer: &mut W,
    updates: &ReferenceUpdatePreview,
    styles: Colorizer,
) -> io::Result<()> {
    if !updates.changes.is_empty() {
        writeln!(writer, "\nReference updates:")?;
        for change in &updates.changes {
            writeln!(writer, "  {}", change.path.display())?;
            match line_diff(&change.before, &change.after) {
                Some(lines) => write_diff_hunks(writer, &lines, styles)?,
                None => writeln!(
                    writer,
                    "    (document changed; detailed diff exceeds the preview limit)"
                )?,
            }
        }
    }
    for warning in &updates.warnings {
        writeln!(writer, "  {} {warning}", styles.style("LSP warning:", "33"))?;
    }
    Ok(())
}

const MAX_DIFF_CELLS: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffLine<'a> {
    Context(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

fn line_diff<'a>(before: &'a str, after: &'a str) -> Option<Vec<DiffLine<'a>>> {
    let before = before.lines().collect::<Vec<_>>();
    let after = after.lines().collect::<Vec<_>>();
    let columns = after.len().checked_add(1)?;
    let cells = before.len().checked_add(1)?.checked_mul(columns)?;
    if cells > MAX_DIFF_CELLS {
        return None;
    }

    let mut lcs = vec![0usize; cells];
    for before_index in (0..before.len()).rev() {
        for after_index in (0..after.len()).rev() {
            let cell = before_index * columns + after_index;
            lcs[cell] = if before[before_index] == after[after_index] {
                lcs[(before_index + 1) * columns + after_index + 1] + 1
            } else {
                lcs[(before_index + 1) * columns + after_index]
                    .max(lcs[before_index * columns + after_index + 1])
            };
        }
    }

    let mut result = Vec::new();
    let (mut before_index, mut after_index) = (0, 0);
    while before_index < before.len() || after_index < after.len() {
        if before_index < before.len()
            && after_index < after.len()
            && before[before_index] == after[after_index]
        {
            result.push(DiffLine::Context(before[before_index]));
            before_index += 1;
            after_index += 1;
        } else if before_index < before.len()
            && (after_index == after.len()
                || lcs[(before_index + 1) * columns + after_index]
                    >= lcs[before_index * columns + after_index + 1])
        {
            result.push(DiffLine::Removed(before[before_index]));
            before_index += 1;
        } else {
            result.push(DiffLine::Added(after[after_index]));
            after_index += 1;
        }
    }
    Some(result)
}

fn write_diff_hunks<W: Write>(
    writer: &mut W,
    lines: &[DiffLine<'_>],
    styles: Colorizer,
) -> io::Result<()> {
    let changed = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (!matches!(line, DiffLine::Context(_))).then_some(index))
        .collect::<Vec<_>>();
    if changed.is_empty() {
        writeln!(writer, "    (text changed without a line-level difference)")?;
        return Ok(());
    }

    let mut hunks = Vec::<(usize, usize)>::new();
    for index in changed {
        let start = index.saturating_sub(1);
        let end = (index + 2).min(lines.len());
        if let Some((_, previous_end)) = hunks.last_mut()
            && start <= *previous_end
        {
            *previous_end = end.max(*previous_end);
        } else {
            hunks.push((start, end));
        }
    }

    let mut previous_end = 0;
    for (start, end) in hunks {
        if start > previous_end {
            writeln!(writer, "    ...")?;
        }
        for line in &lines[start..end] {
            match line {
                DiffLine::Context(text) => {
                    if text.is_empty() {
                        writeln!(writer)?;
                    } else {
                        writeln!(writer, "    {} {text}", styles.style(" ", "2"))?;
                    }
                }
                DiffLine::Removed(text) => {
                    writeln!(writer, "    {} {text}", styles.style("-", "31"))?;
                }
                DiffLine::Added(text) => {
                    writeln!(writer, "    {} {text}", styles.style("+", "32"))?;
                }
            }
        }
        previous_end = end;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{DiffLine, line_diff};

    #[test]
    fn reference_diff_keeps_unchanged_lines_between_edits_as_context() {
        let diff = line_diff(
            "use OldA;\nkeep this;\nuse OldB;",
            "use NewA;\nkeep this;\nuse NewB;",
        )
        .expect("small source diff");

        assert_eq!(
            diff,
            [
                DiffLine::Removed("use OldA;"),
                DiffLine::Added("use NewA;"),
                DiffLine::Context("keep this;"),
                DiffLine::Removed("use OldB;"),
                DiffLine::Added("use NewB;"),
            ]
        );
    }
}
