use serde::Serialize;
use std::fmt;
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    CheckOnlyViolation,
    SkippedGitNonUnicodePath,
    SkippedNonRegularEntry,
    SkippedNonUnicodePath,
    SkippedSymlink,
}

impl FindingKind {
    pub fn suggestion(self) -> &'static str {
        match self {
            Self::CheckOnlyViolation => {
                "Change the rule mode to `apply` if Dirule should rename this path."
            }
            Self::SkippedGitNonUnicodePath | Self::SkippedNonUnicodePath => {
                "Use Unicode paths so Dirule can display and process the entry safely."
            }
            Self::SkippedNonRegularEntry => {
                "Dirule processes regular files and directories; manage this entry separately."
            }
            Self::SkippedSymlink => "Dirule skips symlinks; manage their targets separately.",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub message: String,
    pub path: Option<String>,
}

impl Finding {
    pub fn new(kind: FindingKind, path: Option<&Path>, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            path: path.map(portable_path),
        }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

fn portable_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
