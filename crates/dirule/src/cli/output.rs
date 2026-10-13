mod json;

use super::args::{ColorChoice, OutputFormat};
use dirule_core::Plan;
use dirule_fs::{JournalDetails, JournalRun, JournalStatus};
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

macro_rules! println {
    ($($argument:tt)*) => {
        writeln!(io::stdout().lock(), $($argument)*)?
    };
}

macro_rules! eprintln {
    ($($argument:tt)*) => {
        writeln!(io::stderr().lock(), $($argument)*)?
    };
}

const CURRENT_OUTPUT_VERSION: &str = "1.0";

pub(super) fn is_broken_pipe(error: &(dyn std::error::Error + 'static)) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
        || error
            .downcast_ref::<serde_json::Error>()
            .is_some_and(|error| error.io_error_kind() == Some(io::ErrorKind::BrokenPipe))
}

#[derive(Clone, Copy)]
struct Colorizer {
    enabled: bool,
}

struct Styled<T> {
    value: T,
    ansi_code: &'static str,
    enabled: bool,
}

impl Colorizer {
    fn stdout(choice: ColorChoice) -> Self {
        Self::new(choice, std::io::stdout().is_terminal())
    }

    fn stderr(choice: ColorChoice) -> Self {
        Self::new(choice, std::io::stderr().is_terminal())
    }

    fn new(choice: ColorChoice, is_terminal: bool) -> Self {
        Self {
            enabled: color_enabled(choice, is_terminal),
        }
    }

    fn style<T: fmt::Display>(self, value: T, ansi_code: &'static str) -> Styled<T> {
        Styled {
            value,
            ansi_code,
            enabled: self.enabled,
        }
    }
}

impl<T: fmt::Display> fmt::Display for Styled<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.enabled {
            write!(formatter, "\x1b[{}m", self.ansi_code)?;
        }

        write!(formatter, "{}", self.value)?;

        if self.enabled {
            formatter.write_str("\x1b[0m")?;
        }

        Ok(())
    }
}

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

pub(super) fn created_config(path: &Path, color: ColorChoice) -> io::Result<()> {
    let styles = Colorizer::stdout(color);

    println!("{} {}", styles.style("Created", "32"), path.display());

    Ok(())
}

fn preset_name(name: &str, styles: Colorizer) -> Styled<&str> {
    styles.style(name, "36")
}

pub(super) fn presets(
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

pub(super) fn saved_preset(name: &str, path: &Path, color: ColorChoice) -> io::Result<()> {
    let styles = Colorizer::stdout(color);

    println!(
        "{} Saved preset {} to {}",
        styles.style("Done.", "32"),
        preset_name(name, styles),
        path.display()
    );

    Ok(())
}

pub(super) fn removed_preset(name: &str, color: ColorChoice) -> io::Result<()> {
    let styles = Colorizer::stdout(color);

    println!(
        "{} Removed preset {}",
        styles.style("Done.", "32"),
        preset_name(name, styles)
    );

    Ok(())
}

pub(super) fn error(message: &str, color: ColorChoice) -> io::Result<()> {
    let styles = Colorizer::stderr(color);

    eprintln!("{}: {message}", styles.style("dirule", "31"));

    Ok(())
}

pub(super) fn plan(
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

pub(super) fn plan_to_stderr(
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

pub(super) fn json_apply_preview_needed(format: OutputFormat, yes: bool, plan: &Plan) -> bool {
    format == OutputFormat::Json && !yes && (plan.has_changes() || !plan.is_conflict_free())
}

fn write_text_plan<W: Write>(
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
        PlanContext::Check if changes == 0 && plan.conflicts.is_empty() => ("Check passed", "32"),
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
            writeln!(
                writer,
                "  {} {} ({})",
                styles.style("!", "31"),
                conflict.path.display(),
                conflict.reason
            )?;
        }
    }

    if !plan.findings.is_empty() {
        writeln!(writer, "\nFindings:")?;
        for finding in &plan.findings {
            writeln!(writer, "  {} {finding}", styles.style("i", "36"))?;
        }
    }

    if matches!(context, PlanContext::Check) && changes == 0 && plan.conflicts.is_empty() {
        writeln!(writer, "Target matches the configured rules.")?;
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
        json::write_apply(target, plan, &summary)?;

        return Ok(());
    }

    let styles = Colorizer::stdout(color);

    match summary.status {
        ApplyStatus::Applied => {
            if summary.skipped == 0 {
                println!(
                    "{} Applied {} rename(s).",
                    styles.style("Done.", "32"),
                    summary.moved
                );
            } else {
                println!(
                    "{} Applied {} rename(s); skipped {} conflict(s).",
                    styles.style("Done.", "32"),
                    summary.moved,
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
    fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::NoChanges => "no_changes",
            Self::Cancelled => "cancelled",
        }
    }
}

pub(super) fn runs(
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

pub(super) fn run_details(
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

pub(super) fn no_changes(color: ColorChoice) -> io::Result<()> {
    let styles = Colorizer::stdout(color);

    println!("{}", styles.style("No changes to apply.", "32"));

    Ok(())
}

pub(super) fn cancelled(action: &str, color: ColorChoice) -> io::Result<()> {
    let styles = Colorizer::stdout(color);
    let message = format!("{action} cancelled");

    println!("{}.", styles.style(message, "33"));

    Ok(())
}

fn color_enabled(choice: ColorChoice, is_terminal: bool) -> bool {
    let no_color = std::env::var_os("NO_COLOR");
    let terminal = std::env::var_os("TERM");

    color_enabled_for(
        choice,
        is_terminal,
        no_color.as_deref(),
        terminal.as_deref(),
    )
}

fn color_enabled_for(
    choice: ColorChoice,
    is_terminal: bool,
    no_color: Option<&OsStr>,
    terminal: Option<&OsStr>,
) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            is_terminal
                && no_color.is_none_or(OsStr::is_empty)
                && terminal != Some(OsStr::new("dumb"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ColorChoice, Colorizer, PlanContext, color_enabled_for, json_apply_preview_needed,
        write_text_plan,
    };
    use dirule_core::{EntryKind, Plan, PlannedEntry};
    use std::ffi::OsStr;
    use std::path::Path;

    #[test]
    fn auto_color_requires_a_terminal_and_color_support() {
        assert!(color_enabled_for(ColorChoice::Auto, true, None, None));
        assert!(!color_enabled_for(ColorChoice::Auto, false, None, None));
        assert!(!color_enabled_for(
            ColorChoice::Auto,
            true,
            Some(OsStr::new("1")),
            None
        ));
        assert!(!color_enabled_for(
            ColorChoice::Auto,
            true,
            None,
            Some(OsStr::new("dumb"))
        ));
        assert!(color_enabled_for(
            ColorChoice::Auto,
            true,
            Some(OsStr::new("")),
            None
        ));
    }

    #[test]
    fn explicit_color_choices_override_environment_hints() {
        assert!(color_enabled_for(
            ColorChoice::Always,
            false,
            Some(OsStr::new("1")),
            Some(OsStr::new("dumb"))
        ));
        assert!(!color_enabled_for(ColorChoice::Never, true, None, None));
    }

    #[test]
    fn styles_values_without_changing_the_plain_representation() {
        let color = Colorizer { enabled: true };
        let plain = Colorizer { enabled: false };

        assert_eq!(
            format!("{}", color.style("source.txt", "31")),
            "\x1b[31msource.txt\x1b[0m"
        );
        assert_eq!(format!("{}", plain.style("source.txt", "31")), "source.txt");
    }

    #[test]
    fn json_apply_preview_is_shown_before_confirmation_only_when_needed() {
        let plan = Plan {
            entries: vec![PlannedEntry {
                source: Path::new("MyFile.txt").to_owned(),
                destination: Path::new("my-file.txt").to_owned(),
                kind: EntryKind::File,
                rule: None,
            }],
            ..Plan::default()
        };

        assert!(json_apply_preview_needed(
            super::OutputFormat::Json,
            false,
            &plan
        ));
        assert!(!json_apply_preview_needed(
            super::OutputFormat::Json,
            true,
            &plan
        ));
        assert!(!json_apply_preview_needed(
            super::OutputFormat::Human,
            false,
            &plan
        ));
        assert!(!json_apply_preview_needed(
            super::OutputFormat::Json,
            false,
            &Plan::default()
        ));
    }

    #[test]
    fn text_plan_preview_can_be_rendered_to_an_independent_stream() {
        let plan = Plan {
            entries: vec![PlannedEntry {
                source: Path::new("MyFile.txt").to_owned(),
                destination: Path::new("my-file.txt").to_owned(),
                kind: EntryKind::File,
                rule: None,
            }],
            ..Plan::default()
        };
        let mut output = Vec::new();

        write_text_plan(
            &mut output,
            Path::new("/project"),
            &plan,
            Colorizer { enabled: false },
            PlanContext::Apply,
            false,
        )
        .expect("render plan preview");

        let output = String::from_utf8(output).expect("UTF-8 plan output");
        assert!(output.contains("Apply preview"));
        assert!(output.contains("MyFile.txt -> my-file.txt"));
    }
}
