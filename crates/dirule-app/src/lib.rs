use self::configuration::{ConfigurationLoadError, ConfigurationSource};
use self::paths::HomeDirectoryUnavailable;
use dirule_core::{AcronymOptions, Config, Plan, Rule, RuleKind, Transform, plan_compiled};
use dirule_fs::{
    ApplyError, ApplyReport, ConflictPolicy, ScanError, ScanResult, apply_journaled,
    scan_with_ignore_set, validate_destinations,
};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

pub mod categories;
pub mod configuration;
pub mod journal;
pub mod paths;
pub mod presets;
mod sort;

pub use categories::{
    CURRENT_CATEGORY_CATALOG_VERSION, CategoryCatalog, CategoryCatalogError, SortCategory,
};
pub use sort::{sort_by_extension_preview, sort_extension_preview, sort_selectors_preview};

pub struct Preview {
    pub target: PathBuf,
    pub plan: Plan,
    pub snapshot: ScanResult,
}

pub struct Applied {
    pub target: PathBuf,
    pub plan: Plan,
    pub report: ApplyReport,
    pub run_id: Option<String>,
}

#[derive(Debug)]
pub enum PreviewError {
    Operation(String),
    Target {
        path: PathBuf,
        source: std::io::Error,
    },
    Scan(ScanError),
    Plan(dirule_core::Error),
    Destinations(ApplyError),
}

impl fmt::Display for PreviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operation(message) => formatter.write_str(message),
            Self::Target { path, source } => {
                write!(
                    formatter,
                    "cannot access target {}: {source}",
                    path.display()
                )
            }
            Self::Scan(source) => fmt::Display::fmt(source, formatter),
            Self::Plan(source) => fmt::Display::fmt(source, formatter),
            Self::Destinations(source) => fmt::Display::fmt(source, formatter),
        }
    }
}

impl Error for PreviewError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Operation(_) => None,
            Self::Target { source, .. } => Some(source),
            Self::Scan(source) => Some(source),
            Self::Plan(source) => Some(source),
            Self::Destinations(source) => Some(source),
        }
    }
}

#[derive(Debug)]
pub enum PreviewSourceError {
    Configuration(ConfigurationLoadError),
    Target(HomeDirectoryUnavailable),
    Preview(PreviewError),
}

impl fmt::Display for PreviewSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(source) => fmt::Display::fmt(source, formatter),
            Self::Target(source) => fmt::Display::fmt(source, formatter),
            Self::Preview(source) => fmt::Display::fmt(source, formatter),
        }
    }
}

impl Error for PreviewSourceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Configuration(source) => Some(source),
            Self::Target(source) => Some(source),
            Self::Preview(source) => Some(source),
        }
    }
}

pub fn rename_preview(
    target: &Path,
    transform: Transform,
    recursive: bool,
) -> Result<Preview, PreviewError> {
    let pattern = if recursive { "**/*" } else { "*" };
    let config = quick_config(vec![Rule {
        name: "Rename entries".into(),
        pattern: pattern.into(),
        kind: RuleKind::Any,
        exclude: Vec::new(),
        to: format!("builtin:{}", transform.as_str()),
    }]);

    preview(&config, target)
}

fn quick_config(rules: Vec<Rule>) -> Config {
    Config {
        name: Some("Quick operation".into()),
        target: None,
        ignore: Vec::new(),
        acronyms: AcronymOptions::default(),
        rules,
    }
}

pub fn preview(config: &Config, target: &Path) -> Result<Preview, PreviewError> {
    let target = target
        .canonicalize()
        .map_err(|source| PreviewError::Target {
            path: target.to_path_buf(),
            source,
        })?;
    let compiled = config.compile().map_err(PreviewError::Plan)?;
    let mut snapshot =
        scan_with_ignore_set(&target, compiled.ignore_patterns()).map_err(PreviewError::Scan)?;
    let mut plan = plan_compiled(&compiled, &snapshot.entries).map_err(PreviewError::Plan)?;
    validate_destinations(&target, &mut plan).map_err(PreviewError::Destinations)?;
    plan.findings.append(&mut snapshot.findings);

    Ok(Preview {
        target,
        plan,
        snapshot,
    })
}

pub fn preview_source(
    source: &ConfigurationSource,
    base: &Path,
    explicit_target: Option<PathBuf>,
) -> Result<Preview, PreviewSourceError> {
    let loaded = configuration::load_source(source).map_err(PreviewSourceError::Configuration)?;

    let target =
        paths::resolve_operation_target(base, explicit_target, loaded.config().target.clone())
            .map_err(PreviewSourceError::Target)?;

    preview(loaded.config(), &target).map_err(PreviewSourceError::Preview)
}

pub fn apply_authorized(
    preview: Preview,
    policy: ConflictPolicy,
    journal: &journal::JournalService,
) -> Result<Applied, ApplyError> {
    let Preview {
        target,
        plan,
        snapshot,
    } = preview;
    let applied = apply_journaled(&target, &plan, &snapshot, policy, journal.store())?;

    Ok(Applied {
        target,
        plan,
        report: applied.report,
        run_id: applied.run_id,
    })
}
