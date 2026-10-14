use dirule_core::{Config, Plan, plan};
use dirule_fs::{ApplyError, ScanError, ScanResult, scan, validate_destinations};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

pub struct Preview {
    pub target: PathBuf,
    pub plan: Plan,
    pub snapshot: ScanResult,
}

#[derive(Debug)]
pub enum PreviewError {
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
            Self::Target { source, .. } => Some(source),
            Self::Scan(source) => Some(source),
            Self::Plan(source) => Some(source),
            Self::Destinations(source) => Some(source),
        }
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
