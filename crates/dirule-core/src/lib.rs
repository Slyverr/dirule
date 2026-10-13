mod acronyms;
mod config;
mod error;
mod matching;
mod plan;
mod template;
mod transform;

pub use acronyms::{AcronymLexicon, AcronymOptions};
pub use config::{CURRENT_CONFIG_VERSION, CompiledConfig, Config, ConfigVersion, Rule, RuleKind};
pub use error::{Error, Result};
pub use matching::{Captures, Matcher};
pub use plan::{Conflict, Entry, EntryKind, Plan, PlannedEntry, plan, plan_compiled};
pub use template::{CompiledTemplate, DestinationValues, expand_template, validate_template};
pub use transform::{Transform, transform, transform_filename};

pub fn parse_config(source: &str) -> Result<Config> {
    config::parse(source)
}
