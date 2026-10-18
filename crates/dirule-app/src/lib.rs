//! Frontend-neutral services that connect Dirule's rule engine and filesystem layer.
//!
//! Use [`preview`] or one of the quick-operation helpers to inspect a complete plan.
//! Call [`apply_authorized`] only after the caller has obtained explicit approval.

use self::configuration::{ConfigurationLoadError, ConfigurationSource};
use self::paths::HomeDirectoryUnavailable;
use dirule_core::{
    AcronymOptions, Config, Entry, MatcherSet, Plan, Rule, RuleKind, Transform, plan_compiled,
};
use dirule_fs::{
    ApplyError, ApplyReport, ScanError, ScanResult, apply_journaled, scan_with_entry_limit,
    scan_with_ignore_set, validate_destinations,
};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

/// Load and validate custom file-category definitions used by sorting.
pub mod categories;
/// Load, discover, create, and edit rule configurations.
pub mod configuration;
/// List and undo journaled filesystem operations.
pub mod journal;
/// Resolve target, configuration, and user-state paths.
pub mod paths;
/// Read and manage reusable user presets.
pub mod presets;
mod sort;

pub use categories::{
    CURRENT_CATEGORY_CATALOG_VERSION, CategoryCatalog, CategoryCatalogError, SortCategory,
};
pub use dirule_fs::ConflictPolicy;
pub use sort::{
    sort_all_categories_preview, sort_by_extension_preview, sort_extension_preview,
    sort_selectors_preview, sort_selectors_preview_with_entry_limit,
};

/// A read-only scan and validated plan for one target.
///
/// The plan and scan snapshot are paired so applying the plan can revalidate the
/// same sources and destinations. Creating a preview does not mutate the target,
/// and callers cannot replace its plan or snapshot independently.
pub struct Preview {
    /// Canonical target directory used for the scan.
    target: PathBuf,
    /// Proposed renames and any conflicts or findings.
    plan: Plan,
    /// Scanned entries and findings used to validate an apply operation.
    snapshot: ScanResult,
}

/// A bounded, read-only listing of paths visible to Dirule in one target.
pub struct TargetInspection {
    /// Canonical target directory used for the scan.
    pub target: PathBuf,
    /// Visible entries collected up to the requested scan limit.
    pub entries: Vec<Entry>,
    /// Whether the scan stopped because it reached the entry limit.
    pub truncated: bool,
}

impl Preview {
    /// Return the canonical target directory used for the scan.
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// Return the proposed renames, conflicts, and findings.
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Return the scan snapshot paired with the plan for apply revalidation.
    pub fn snapshot(&self) -> &ScanResult {
        &self.snapshot
    }
}

/// The result of applying a previously reviewed preview.
pub struct Applied {
    /// Canonical target directory that was changed.
    pub target: PathBuf,
    /// Plan that was passed to the filesystem apply layer.
    pub plan: Plan,
    /// Successful and skipped filesystem operations.
    pub report: ApplyReport,
    /// Journal run identifier, when the apply created a journal entry.
    pub run_id: Option<String>,
}

#[derive(Debug)]
/// A failure while creating or validating a preview.
pub enum PreviewError {
    /// A quick operation could not be represented as a valid rule.
    Operation(String),
    /// The scan exceeded the caller's entry limit and cannot produce a complete plan.
    EntryLimitExceeded {
        /// Maximum number of entries allowed in the scan.
        maximum: usize,
    },
    /// The target could not be accessed or canonicalized.
    Target {
        /// Requested target path.
        path: PathBuf,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// Scanning the target failed.
    Scan(ScanError),
    /// Configuration compilation or planning failed.
    Plan(dirule_core::Error),
    /// A proposed destination failed filesystem validation.
    Destinations(ApplyError),
}

impl fmt::Display for PreviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operation(message) => formatter.write_str(message),
            Self::EntryLimitExceeded { maximum } => write!(
                formatter,
                "target exceeds the {maximum}-entry scan limit; choose a narrower target"
            ),
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
            Self::Operation(_) | Self::EntryLimitExceeded { .. } => None,
            Self::Target { source, .. } => Some(source),
            Self::Scan(source) => Some(source),
            Self::Plan(source) => Some(source),
            Self::Destinations(source) => Some(source),
        }
    }
}

#[derive(Debug)]
/// A failure while loading a source, resolving its target, or creating its preview.
pub enum PreviewSourceError {
    /// The selected configuration could not be loaded.
    Configuration(ConfigurationLoadError),
    /// The target could not be resolved, including home expansion.
    Target(HomeDirectoryUnavailable),
    /// Scanning, planning, or destination validation failed.
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

/// Preview a case transformation over files and directories in a target.
///
/// By default only entries directly inside `target` are considered. Set
/// `recursive` to include nested entries.
pub fn rename_preview(
    target: &Path,
    transform: Transform,
    recursive: bool,
) -> Result<Preview, PreviewError> {
    rename_preview_with_entry_limit(target, transform, recursive, None)
}

/// Preview a case transformation while bounding the number of scanned entries.
///
/// `maximum_entries` is `None` for an unbounded scan. If the bounded scan is
/// incomplete, the preview fails instead of returning a partial plan.
pub fn rename_preview_with_entry_limit(
    target: &Path,
    transform: Transform,
    recursive: bool,
    maximum_entries: Option<usize>,
) -> Result<Preview, PreviewError> {
    let pattern = if recursive { "**/*" } else { "*" };
    let config = quick_config(vec![Rule {
        name: "Rename entries".into(),
        pattern: pattern.into(),
        kind: RuleKind::Any,
        exclude: Vec::new(),
        to: format!("builtin:{}", transform.as_str()),
    }]);

    preview_with_entry_limit(&config, target, maximum_entries)
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

/// Compile rules, scan a target, build a plan, and validate every destination.
///
/// This operation is read-only. The returned preview can be shown to the user
/// before passing it to [`apply_authorized`].
pub fn preview(config: &Config, target: &Path) -> Result<Preview, PreviewError> {
    preview_with_entry_limit(config, target, None)
}

/// Compile rules, scan a target within an optional entry limit, build a plan,
/// and validate every destination.
///
/// `maximum_entries` is `None` for an unbounded scan. A partial scan never
/// produces a preview.
pub fn preview_with_entry_limit(
    config: &Config,
    target: &Path,
    maximum_entries: Option<usize>,
) -> Result<Preview, PreviewError> {
    let target = target
        .canonicalize()
        .map_err(|source| PreviewError::Target {
            path: target.to_path_buf(),
            source,
        })?;
    let compiled = config.compile().map_err(PreviewError::Plan)?;
    let mut snapshot = match maximum_entries {
        Some(maximum_entries) => {
            scan_with_entry_limit(&target, compiled.ignore_patterns(), maximum_entries)
        }
        None => scan_with_ignore_set(&target, compiled.ignore_patterns()),
    }
    .map_err(PreviewError::Scan)?;
    if snapshot.truncated {
        return Err(PreviewError::EntryLimitExceeded {
            maximum: maximum_entries.expect("a limited scan has a maximum entry count"),
        });
    }

    let mut plan = plan_compiled(&compiled, &snapshot.entries).map_err(PreviewError::Plan)?;
    validate_destinations(&target, &mut plan).map_err(PreviewError::Destinations)?;
    plan.findings.append(&mut snapshot.findings);

    Ok(Preview {
        target,
        plan,
        snapshot,
    })
}

/// Load a selected configuration, resolve its target, and create a preview.
///
/// `explicit_target` takes precedence over a target stored in the configuration.
/// Relative paths are resolved from `base`.
pub fn preview_source(
    source: &ConfigurationSource,
    base: &Path,
    explicit_target: Option<PathBuf>,
) -> Result<Preview, PreviewSourceError> {
    preview_source_with_entry_limit(source, base, explicit_target, None)
}

/// Load a selected configuration, resolve its target, and create a bounded preview.
///
/// `maximum_entries` is `None` for an unbounded scan. A partial scan never
/// produces a preview.
pub fn preview_source_with_entry_limit(
    source: &ConfigurationSource,
    base: &Path,
    explicit_target: Option<PathBuf>,
    maximum_entries: Option<usize>,
) -> Result<Preview, PreviewSourceError> {
    let loaded = configuration::load_source(source).map_err(PreviewSourceError::Configuration)?;

    let target =
        paths::resolve_operation_target(base, explicit_target, loaded.config().target.clone())
            .map_err(PreviewSourceError::Target)?;

    preview_with_entry_limit(loaded.config(), &target, maximum_entries)
        .map_err(PreviewSourceError::Preview)
}

/// Inspect visible target entries without building a plan or reading file contents.
pub fn inspect_target(
    target: &Path,
    maximum_entries: usize,
) -> Result<TargetInspection, PreviewError> {
    let snapshot = scan_with_entry_limit(target, &MatcherSet::default(), maximum_entries)
        .map_err(PreviewError::Scan)?;

    Ok(TargetInspection {
        target: target
            .canonicalize()
            .map_err(|source| PreviewError::Target {
                path: target.to_path_buf(),
                source,
            })?,
        entries: snapshot.entries,
        truncated: snapshot.truncated,
    })
}

/// Apply a reviewed preview using the requested conflict policy and journal.
///
/// The caller is responsible for obtaining explicit approval before calling this
/// function. The filesystem layer revalidates source and destination identities.
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
