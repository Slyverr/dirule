mod acronyms;
mod config;
mod error;
mod finding;
mod matching;
mod plan;
mod template;
mod transform;

pub use acronyms::{AcronymLexicon, AcronymOptions};
pub use config::{
    CURRENT_CONFIG_VERSION, CompiledConfig, Config, ConfigVersion, LspOptions, LspServerConfig,
    ReferenceUpdatePolicy, Rule, RuleKind, RuleMode,
};
pub use error::{Error, Result};
pub use finding::{Finding, FindingKind};
pub use matching::{Captures, Matcher, MatcherSet};
pub use plan::{Conflict, ConflictKind, Entry, EntryKind, Plan, PlannedEntry, plan, plan_compiled};
pub use template::{CompiledTemplate, DestinationValues, expand_template, validate_template};
pub use transform::{Transform, transform, transform_directory, transform_filename};

pub fn parse_config(source: &str) -> Result<Config> {
    config::parse(source)
}
