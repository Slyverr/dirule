mod document;
mod migrations;
mod version;

pub use version::{CURRENT_CONFIG_VERSION, ConfigVersion};

use crate::{AcronymLexicon, AcronymOptions, CompiledTemplate, Error, Matcher, Result, Transform};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub name: Option<String>,
    pub target: Option<PathBuf>,
    pub acronyms: AcronymOptions,
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug)]
pub struct CompiledConfig {
    pub(crate) rules: Vec<CompiledRule>,
    pub(crate) acronyms: AcronymLexicon,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledRule {
    pub(crate) name: String,
    pub(crate) matcher: Matcher,
    pub(crate) kind: RuleKind,
    pub(crate) destination: CompiledDestination,
}

#[derive(Clone, Debug)]
pub(crate) enum CompiledDestination {
    Builtin(Transform),
    Template(CompiledTemplate),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleKind {
    #[default]
    Any,
    File,
    Directory,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub name: String,

    #[serde(rename = "match")]
    pub pattern: String,

    #[serde(default)]
    pub kind: RuleKind,

    pub to: String,
}

pub fn parse(source: &str) -> Result<Config> {
    let document: toml::Value = toml::from_str(source)?;
    let version = version::read(&document)?;
    let document = migrations::upgrade(document, version)?;
    let document: document::ConfigDocument = document.try_into()?;

    if document.version != CURRENT_CONFIG_VERSION {
        return Err(Error::Config(format!(
            "configuration migration ended at {}, expected {}",
            document.version, CURRENT_CONFIG_VERSION
        )));
    }

    let config = Config {
        name: document.name,
        target: document.target,
        acronyms: document.acronyms,
        rules: document.rules,
    };

    config.validate()?;
    Ok(config)
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        self.compile().map(|_| ())
    }

    pub fn compile(&self) -> Result<CompiledConfig> {
        if self.rules.is_empty() {
            return Err(Error::Config("at least one rule is required".into()));
        }

        let acronyms = self.acronyms.compile()?;
        let mut compiled_rules = Vec::with_capacity(self.rules.len());

        for (index, rule) in self.rules.iter().enumerate() {
            if rule.name.trim().is_empty() {
                return Err(Error::Config(format!(
                    "rule {} has an empty name",
                    index + 1
                )));
            }

            let matcher = Matcher::new(&rule.pattern)
                .map_err(|error| Error::Config(format!("rule {:?}: {error}", rule.name)))?;

            let (namespace, value) = rule.to.split_once(':').ok_or_else(|| {
                Error::Config(format!(
                    "rule {:?}: `to` must use `builtin:` or `template:`",
                    rule.name
                ))
            })?;

            let destination = match namespace {
                "builtin" => CompiledDestination::Builtin(
                    Transform::parse(value)
                        .map_err(|error| Error::Config(format!("rule {:?}: {error}", rule.name)))?,
                ),
                "template" => {
                    let captures = matcher
                        .capture_names()
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    CompiledDestination::Template(
                        CompiledTemplate::parse(value, &captures).map_err(|error| {
                            Error::Config(format!("rule {:?}: {error}", rule.name))
                        })?,
                    )
                }
                other => {
                    return Err(Error::Config(format!(
                        "rule {:?}: unknown destination namespace `{other}`",
                        rule.name
                    )));
                }
            };

            compiled_rules.push(CompiledRule {
                name: rule.name.clone(),
                matcher,
                kind: rule.kind,
                destination,
            });
        }

        Ok(CompiledConfig {
            rules: compiled_rules,
            acronyms,
        })
    }
}
