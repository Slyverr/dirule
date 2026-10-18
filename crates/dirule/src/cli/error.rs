use super::{args, commands, output};
use std::error::Error;
use std::fmt;

const CHECK_FAILURE_EXIT_CODE: u8 = 1;
const COMMAND_ERROR_EXIT_CODE: u8 = 2;

#[derive(Clone, Copy, Debug)]
enum CliErrorKind {
    CheckFailure,
    CommandError,
}

impl CliErrorKind {
    fn code(self) -> &'static str {
        match self {
            Self::CheckFailure => "check_failed",
            Self::CommandError => "command_error",
        }
    }

    fn exit_code(self) -> u8 {
        match self {
            Self::CheckFailure => CHECK_FAILURE_EXIT_CODE,
            Self::CommandError => COMMAND_ERROR_EXIT_CODE,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum CliErrorCategory {
    CheckFailure,
    Configuration,
    ConfigurationSelection,
    Target,
    Scan,
    Apply,
    Journal,
    Preset,
    Command,
}

impl CliErrorCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::CheckFailure => "check_failure",
            Self::Configuration => "configuration",
            Self::ConfigurationSelection => "configuration_selection",
            Self::Target => "target",
            Self::Scan => "scan",
            Self::Apply => "apply",
            Self::Journal => "journal",
            Self::Preset => "preset",
            Self::Command => "command",
        }
    }
}

#[derive(Debug)]
pub struct CliError {
    source: Box<dyn Error>,
    color: args::ColorChoice,
    format: args::OutputFormat,
    kind: CliErrorKind,
    category: CliErrorCategory,
}

impl CliError {
    pub(super) fn new(
        source: Box<dyn Error>,
        color: args::ColorChoice,
        format: args::OutputFormat,
    ) -> Self {
        let (kind, category) = if source.downcast_ref::<commands::CheckFailure>().is_some() {
            (CliErrorKind::CheckFailure, CliErrorCategory::CheckFailure)
        } else {
            (
                CliErrorKind::CommandError,
                classify_error_category(source.as_ref()),
            )
        };

        Self {
            source,
            color,
            format,
            kind,
            category,
        }
    }

    pub fn exit_code(&self) -> u8 {
        self.kind.exit_code()
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.source, formatter)
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub fn print_error(error: &CliError) {
    let _ = output::error(
        &error.to_string(),
        error.color,
        error.format,
        error.kind.code(),
        error.kind.exit_code(),
        error.category.as_str(),
    );
}

fn classify_error_category(error: &(dyn Error + 'static)) -> CliErrorCategory {
    let mut cause = Some(error);
    while let Some(source) = cause {
        cause = source.source();

        if let Some(error) = source.downcast_ref::<dirule_app::PreviewError>() {
            match error {
                dirule_app::PreviewError::Target { .. } => return CliErrorCategory::Target,
                dirule_app::PreviewError::Scan(_)
                | dirule_app::PreviewError::EntryLimitExceeded { .. } => {
                    return CliErrorCategory::Scan;
                }
                dirule_app::PreviewError::Plan(_) => return CliErrorCategory::Configuration,
                dirule_app::PreviewError::Destinations(_) => return CliErrorCategory::Apply,
                dirule_app::PreviewError::Operation(_) => return CliErrorCategory::Command,
            }
        }

        if let Some(error) = source.downcast_ref::<dirule_app::PreviewSourceError>() {
            match error {
                dirule_app::PreviewSourceError::Target(_) => return CliErrorCategory::Target,
                dirule_app::PreviewSourceError::Configuration(_)
                | dirule_app::PreviewSourceError::Preview(_) => {}
            }
        }

        if let Some(error) =
            source.downcast_ref::<dirule_app::configuration::ConfigurationSelectionError>()
        {
            match error {
                dirule_app::configuration::ConfigurationSelectionError::Target(_) => {
                    return CliErrorCategory::Target;
                }
                dirule_app::configuration::ConfigurationSelectionError::LocalConfig { .. } => {
                    return CliErrorCategory::Configuration;
                }
                dirule_app::configuration::ConfigurationSelectionError::Presets(_) => {
                    return CliErrorCategory::Preset;
                }
                dirule_app::configuration::ConfigurationSelectionError::TargetPath { .. } => {
                    return CliErrorCategory::Target;
                }
                dirule_app::configuration::ConfigurationSelectionError::AmbiguousPresetTarget {
                    ..
                } => {
                    return CliErrorCategory::ConfigurationSelection;
                }
            }
        }

        if source.is::<dirule_app::configuration::ConfigurationLoadError>()
            || source.is::<dirule_app::configuration::ConfigEditorError>()
            || source.is::<dirule_app::CategoryCatalogError>()
            || source.is::<dirule_core::Error>()
        {
            return CliErrorCategory::Configuration;
        }

        if source.is::<dirule_app::paths::HomeDirectoryUnavailable>() {
            return CliErrorCategory::Target;
        }

        if source.is::<dirule_fs::ScanError>() {
            return CliErrorCategory::Scan;
        }

        if let Some(error) = source.downcast_ref::<dirule_fs::ApplyError>() {
            return match error {
                dirule_fs::ApplyError::Journal(_) => CliErrorCategory::Journal,
                _ => CliErrorCategory::Apply,
            };
        }

        if source.is::<dirule_fs::JournalError>()
            || source.is::<dirule_app::paths::UserStateDirectoryUnavailable>()
        {
            return CliErrorCategory::Journal;
        }

        if source.is::<dirule_app::presets::PresetError>() {
            return CliErrorCategory::Preset;
        }
    }

    CliErrorCategory::Command
}
