use super::ConfigVersion;
use crate::{AcronymOptions, LspOptions, Rule};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfigDocument {
    pub version: ConfigVersion,
    pub name: Option<String>,
    pub target: Option<PathBuf>,
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub acronyms: AcronymOptions,
    #[serde(default)]
    pub lsp: LspOptions,
    #[serde(default)]
    pub rules: Vec<Rule>,
}
