use dirule_core::{AcronymOptions, Config, Plan, Rule, RuleKind, Transform, plan};
use dirule_fs::{
    ApplyError, ApplyReport, ConflictPolicy, JournalStore, ScanError, ScanResult, apply_journaled,
    scan, validate_destinations,
};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

pub mod presets;

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

pub fn sort_extension_preview(
    target: &Path,
    extension: &str,
    recursive: bool,
) -> Result<Preview, PreviewError> {
    let extension = extension.strip_prefix('.').unwrap_or(extension);
    if extension.is_empty()
        || !extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(PreviewError::Operation(
            "extension must contain only ASCII letters and digits".into(),
        ));
    }

    let extension = extension.to_ascii_lowercase();
    let pattern = format!("?*.(?i){extension}");
    let pattern = if recursive {
        format!("**/{pattern}")
    } else {
        pattern
    };
    let rules = vec![sort_rule(
        "Sort matching files",
        pattern,
        "${extension|kebab-case}s/${name}",
    )];

    preview(&quick_config(rules), target)
}

pub fn sort_by_extension_preview(target: &Path, recursive: bool) -> Result<Preview, PreviewError> {
    let pattern = if recursive { "**/?*.*" } else { "?*.*" };
    let rules = vec![sort_rule(
        "Sort files by extension",
        pattern.into(),
        "${extension|kebab-case}s/${name}",
    )];

    preview(&quick_config(rules), target)
}

fn sort_rule(name: &str, pattern: String, destination: &str) -> Rule {
    Rule {
        name: name.into(),
        pattern,
        kind: RuleKind::File,
        exclude: Vec::new(),
        to: format!("template:{destination}"),
    }
}

fn quick_config(rules: Vec<Rule>) -> Config {
    Config {
        name: Some("Quick operation".into()),
        target: None,
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
    let mut snapshot = scan(&target).map_err(PreviewError::Scan)?;
    let mut plan = plan(config, &snapshot.entries).map_err(PreviewError::Plan)?;
    validate_destinations(&target, &mut plan).map_err(PreviewError::Destinations)?;
    plan.findings.append(&mut snapshot.findings);

    Ok(Preview {
        target,
        plan,
        snapshot,
    })
}

pub fn apply_authorized(
    preview: Preview,
    policy: ConflictPolicy,
    journal: &JournalStore,
) -> Result<Applied, ApplyError> {
    let Preview {
        target,
        plan,
        snapshot,
    } = preview;
    let applied = apply_journaled(&target, &plan, &snapshot, policy, journal)?;

    Ok(Applied {
        target,
        plan,
        report: applied.report,
        run_id: applied.run_id,
    })
}
