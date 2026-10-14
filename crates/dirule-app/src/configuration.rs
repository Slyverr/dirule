use dirule_core::{CURRENT_CONFIG_VERSION, Config, Rule, RuleKind, parse_config};
use std::error::Error;
use std::fmt;
use std::fs::{self, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use toml_edit::{DocumentMut, TomlError};

mod create;
mod loader;
mod selection;
mod source;

pub use create::{ConfigCreationError, create_starter};
pub use loader::{ConfigurationLoadError, LoadedConfiguration, load, load_source};
pub use selection::{ConfigurationSelectionError, discover_source};
pub use source::{ConfigurationSource, ConfigurationSourceKind};

pub struct ConfigEditor {
    path: PathBuf,
    original: String,
    document: DocumentMut,
}

#[derive(Debug)]
pub enum ConfigEditorError {
    RelativePath(PathBuf),
    NotRegularFile(PathBuf),
    ConcurrentModification(PathBuf),
    InvalidRuleIndex {
        index: usize,
        rule_count: usize,
    },
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Toml {
        path: PathBuf,
        source: TomlError,
    },
    Configuration {
        path: PathBuf,
        source: dirule_core::Error,
    },
}

impl ConfigEditor {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, ConfigEditorError> {
        let path = path.into();
        if !path.is_absolute() {
            return Err(ConfigEditorError::RelativePath(path));
        }

        let _ = regular_file_metadata(&path)?;
        let original = fs::read_to_string(&path).map_err(|source| ConfigEditorError::Io {
            action: "read",
            path: path.clone(),
            source,
        })?;
        parse_config(&original).map_err(|source| ConfigEditorError::Configuration {
            path: path.clone(),
            source,
        })?;
        let document =
            original
                .parse::<DocumentMut>()
                .map_err(|source| ConfigEditorError::Toml {
                    path: path.clone(),
                    source,
                })?;

        Ok(Self {
            path,
            original,
            document,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn document(&self) -> &DocumentMut {
        &self.document
    }

    pub fn document_mut(&mut self) -> &mut DocumentMut {
        &mut self.document
    }

    pub fn contents(&self) -> String {
        self.document.to_string()
    }

    pub fn config(&self) -> Result<Config, ConfigEditorError> {
        let contents = self.contents();

        parse_config(&contents).map_err(|source| ConfigEditorError::Configuration {
            path: self.path.clone(),
            source,
        })
    }

    pub fn add_rule(&mut self, rule: Rule) -> Result<(), ConfigEditorError> {
        let original = self.document.clone();
        let Some(rules) = self.document["rules"].as_array_of_tables_mut() else {
            return Err(self.invalid_config("rules must be an array of tables"));
        };
        rules.push(rule_table(&rule));

        self.validate_or_restore(original)
    }

    pub fn update_rule(&mut self, index: usize, rule: Rule) -> Result<(), ConfigEditorError> {
        let original = self.document.clone();
        let Some(rules) = self.document["rules"].as_array_of_tables_mut() else {
            return Err(self.invalid_config("rules must be an array of tables"));
        };
        let rule_count = rules.len();
        let Some(table) = rules.get_mut(index) else {
            return Err(ConfigEditorError::InvalidRuleIndex { index, rule_count });
        };
        update_rule_table(table, &rule);

        self.validate_or_restore(original)
    }

    pub fn remove_rule(&mut self, index: usize) -> Result<Rule, ConfigEditorError> {
        let original_config = self.config()?;
        let rule_count = original_config.rules.len();
        let Some(rule) = original_config.rules.get(index).cloned() else {
            return Err(ConfigEditorError::InvalidRuleIndex { index, rule_count });
        };
        let original = self.document.clone();
        let Some(rules) = self.document["rules"].as_array_of_tables_mut() else {
            return Err(self.invalid_config("rules must be an array of tables"));
        };
        rules.remove(index);

        self.validate_or_restore(original)?;

        Ok(rule)
    }

    pub fn move_rule(&mut self, from: usize, to: usize) -> Result<(), ConfigEditorError> {
        let original = self.document.clone();
        let Some(rules) = self.document["rules"].as_array_of_tables_mut() else {
            return Err(self.invalid_config("rules must be an array of tables"));
        };
        let rule_count = rules.len();
        if from >= rule_count {
            return Err(ConfigEditorError::InvalidRuleIndex {
                index: from,
                rule_count,
            });
        }
        if to >= rule_count {
            return Err(ConfigEditorError::InvalidRuleIndex {
                index: to,
                rule_count,
            });
        }

        if from != to {
            let rule = rules.remove(from);
            rules.insert(to, rule);
        }

        self.validate_or_restore(original)
    }

    pub fn set_ignore_patterns(&mut self, patterns: &[String]) -> Result<(), ConfigEditorError> {
        let original = self.document.clone();
        let mut ignore = toml_edit::Array::new();
        for pattern in patterns {
            ignore.push(pattern.as_str());
        }
        self.document["version"] = toml_edit::value(CURRENT_CONFIG_VERSION.to_string());
        self.document["ignore"] = toml_edit::value(ignore);

        self.validate_or_restore(original)
    }

    fn validate_or_restore(&mut self, original: DocumentMut) -> Result<(), ConfigEditorError> {
        if let Err(error) = self.config() {
            self.document = original;

            return Err(error);
        }

        Ok(())
    }

    fn invalid_config(&self, message: &str) -> ConfigEditorError {
        ConfigEditorError::Configuration {
            path: self.path.clone(),
            source: dirule_core::Error::Config(message.into()),
        }
    }

    pub fn save(&mut self) -> Result<Config, ConfigEditorError> {
        let rendered = self.contents();
        let contents = if rendered == format!("{}\n", self.original) {
            self.original.clone()
        } else {
            rendered
        };
        let config =
            parse_config(&contents).map_err(|source| ConfigEditorError::Configuration {
                path: self.path.clone(),
                source,
            })?;
        let metadata = regular_file_metadata(&self.path)?;
        let current = fs::read_to_string(&self.path).map_err(|source| ConfigEditorError::Io {
            action: "read",
            path: self.path.clone(),
            source,
        })?;
        if current != self.original {
            return Err(ConfigEditorError::ConcurrentModification(self.path.clone()));
        }

        if contents == self.original {
            return Ok(config);
        }

        self.replace_contents(&contents, metadata)?;
        self.original = contents;

        Ok(config)
    }

    fn replace_contents(
        &self,
        contents: &str,
        metadata: Metadata,
    ) -> Result<(), ConfigEditorError> {
        let parent = self.path.parent().ok_or_else(|| ConfigEditorError::Io {
            action: "locate parent directory for",
            path: self.path.clone(),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "configuration path has no parent directory",
            ),
        })?;
        let mut temporary =
            NamedTempFile::new_in(parent).map_err(|source| ConfigEditorError::Io {
                action: "create a temporary file beside",
                path: self.path.clone(),
                source,
            })?;
        temporary
            .write_all(contents.as_bytes())
            .map_err(|source| ConfigEditorError::Io {
                action: "write temporary configuration for",
                path: self.path.clone(),
                source,
            })?;
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|source| ConfigEditorError::Io {
                action: "preserve permissions for",
                path: self.path.clone(),
                source,
            })?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| ConfigEditorError::Io {
                action: "sync temporary configuration for",
                path: self.path.clone(),
                source,
            })?;
        temporary
            .persist(&self.path)
            .map_err(|error| ConfigEditorError::Io {
                action: "replace",
                path: self.path.clone(),
                source: error.error,
            })?;

        Ok(())
    }
}

fn regular_file_metadata(path: &Path) -> Result<Metadata, ConfigEditorError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| ConfigEditorError::Io {
        action: "inspect",
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(ConfigEditorError::NotRegularFile(path.to_path_buf()));
    }

    Ok(metadata)
}

fn rule_table(rule: &Rule) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    update_rule_table(&mut table, rule);

    table
}

fn update_rule_table(table: &mut toml_edit::Table, rule: &Rule) {
    table["name"] = toml_edit::value(&rule.name);
    table["match"] = toml_edit::value(&rule.pattern);
    table["to"] = toml_edit::value(&rule.to);
    match rule.kind {
        RuleKind::Any => {
            table.remove("kind");
        }
        RuleKind::File => table["kind"] = toml_edit::value("file"),
        RuleKind::Directory => table["kind"] = toml_edit::value("directory"),
    }
    if rule.exclude.is_empty() {
        table.remove("exclude");
    } else {
        let mut exclude = toml_edit::Array::new();
        for pattern in &rule.exclude {
            exclude.push(pattern.as_str());
        }
        table["exclude"] = toml_edit::value(exclude);
    }
}

impl fmt::Display for ConfigEditorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(path) => {
                write!(
                    formatter,
                    "configuration path must be absolute: {}",
                    path.display()
                )
            }
            Self::NotRegularFile(path) => write!(
                formatter,
                "configuration path is not a regular file: {}",
                path.display()
            ),
            Self::ConcurrentModification(path) => write!(
                formatter,
                "configuration changed after it was opened: {}",
                path.display()
            ),
            Self::InvalidRuleIndex { index, rule_count } => write!(
                formatter,
                "rule index {index} is out of range for {rule_count} rule(s)"
            ),
            Self::Io {
                action,
                path,
                source,
            } => write!(formatter, "cannot {action} {}: {source}", path.display()),
            Self::Toml { path, source } => {
                write!(formatter, "cannot edit TOML {}: {source}", path.display())
            }
            Self::Configuration { path, source } => {
                write!(
                    formatter,
                    "invalid configuration {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl Error for ConfigEditorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Toml { source, .. } => Some(source),
            Self::Configuration { source, .. } => Some(source),
            Self::RelativePath(_)
            | Self::NotRegularFile(_)
            | Self::ConcurrentModification(_)
            | Self::InvalidRuleIndex { .. } => None,
        }
    }
}
