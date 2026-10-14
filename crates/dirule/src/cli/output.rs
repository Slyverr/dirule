use super::args::ColorChoice;
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, IsTerminal};

macro_rules! println {
    ($($argument:tt)*) => {{
        use std::io::Write as _;
        writeln!(std::io::stdout().lock(), $($argument)*)?
    }};
}

macro_rules! eprintln {
    ($($argument:tt)*) => {{
        use std::io::Write as _;
        writeln!(std::io::stderr().lock(), $($argument)*)?
    }};
}

const CURRENT_OUTPUT_VERSION: &str = "1.2";

mod apply;
mod config;
mod errors;
mod journal;
mod json;
mod plan;
mod presets;

pub(super) use apply::{ApplyStatus, ApplySummary, apply_result};
pub(super) use config::{created_config, validated_config};
pub(super) use errors::error;
pub(super) use journal::{run_details, runs, undo_result};
pub(super) use plan::{PlanContext, json_apply_preview_needed, plan, plan_to_stderr};
pub(super) use presets::{presets, removed_preset, saved_preset};

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
    use super::super::args::OutputFormat;
    use super::{
        ColorChoice, Colorizer, PlanContext, color_enabled_for, json_apply_preview_needed,
        plan::write_text_plan,
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

        assert!(json_apply_preview_needed(OutputFormat::Json, false, &plan));
        assert!(!json_apply_preview_needed(OutputFormat::Json, true, &plan));
        assert!(!json_apply_preview_needed(
            OutputFormat::Human,
            false,
            &plan
        ));
        assert!(!json_apply_preview_needed(
            OutputFormat::Json,
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
