use dirule_core::{CURRENT_CONFIG_VERSION, Config, Rule, RuleKind, parse_config};
use std::error::Error;
use std::fmt;
use std::fs::{self, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use toml_edit::{DocumentMut, TomlError};

/// Edit a valid TOML configuration while preserving unrelated formatting.
pub struct ConfigEditor {
    path: PathBuf,
    original: String,
    document: DocumentMut,
}

#[derive(Debug)]
/// A configuration edit was invalid, unsafe to write, or raced with another edit.
pub enum ConfigEditorError {
    RelativePath(PathBuf),
    NotRegularFile(PathBuf),
    NonUnicodeTarget(PathBuf),
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
    /// Open an absolute, regular, valid configuration file for editing.
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

    /// Return the file being edited.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Set or remove the optional configuration name.
    pub fn set_name(&mut self, name: Option<&str>) -> Result<(), ConfigEditorError> {
        self.set_optional_string("name", name)
    }

    /// Set or remove the target path stored in the configuration.
    pub fn set_target(&mut self, target: Option<&Path>) -> Result<(), ConfigEditorError> {
        let target = target
            .map(|path| {
                path.to_str()
                    .ok_or_else(|| ConfigEditorError::NonUnicodeTarget(path.to_path_buf()))
            })
            .transpose()?;

        self.set_optional_string("target", target)
    }

    /// Render the current in-memory document without writing it.
    pub fn contents(&self) -> String {
        self.document.to_string()
    }

    /// Parse the current in-memory document into a validated configuration.
    pub fn config(&self) -> Result<Config, ConfigEditorError> {
        let contents = self.contents();

        parse_config(&contents).map_err(|source| ConfigEditorError::Configuration {
            path: self.path.clone(),
            source,
        })
    }

    /// Append a rule and keep the edit only if the resulting config is valid.
    pub fn add_rule(&mut self, rule: Rule) -> Result<(), ConfigEditorError> {
        let original = self.document.clone();
        let Some(rules) = self.document["rules"].as_array_of_tables_mut() else {
            return Err(self.invalid_config("rules must be an array of tables"));
        };
        rules.push(rule_table(&rule));

        self.validate_or_restore(original)
    }

    /// Replace a rule by its zero-based index and validate the resulting config.
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

    /// Remove a rule by its zero-based index and return the removed rule.
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

    /// Move a rule from one zero-based index to another.
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

    /// Replace the top-level target-wide ignore patterns.
    pub fn set_ignore_patterns(&mut self, patterns: &[String]) -> Result<(), ConfigEditorError> {
        let original = self.document.clone();
        let existing_ignore = self
            .document
            .get("ignore")
            .and_then(toml_edit::Item::as_value);
        let ignore = string_array_preserving_entries(existing_ignore, patterns);
        set_value_preserving_decor(
            self.document.as_table_mut(),
            "version",
            toml_edit::value(CURRENT_CONFIG_VERSION.to_string()),
        );
        set_value_preserving_decor(
            self.document.as_table_mut(),
            "ignore",
            toml_edit::value(ignore),
        );

        self.validate_or_restore(original)
    }

    /// Replace the acronym entries used by built-in name transformations.
    pub fn set_acronym_entries(&mut self, entries: &[String]) -> Result<(), ConfigEditorError> {
        if entries.is_empty() && self.document.get("acronyms").is_none() {
            return Ok(());
        }

        let original = self.document.clone();

        if let Some(acronyms) = self.document.as_table_mut().get_mut("acronyms") {
            match acronyms {
                toml_edit::Item::Table(table) => {
                    let existing = table.get("entries").and_then(toml_edit::Item::as_value);
                    let values = string_array_preserving_entries(existing, entries);
                    set_value_preserving_decor(table, "entries", toml_edit::value(values));
                }
                toml_edit::Item::Value(toml_edit::Value::InlineTable(table)) => {
                    let existing = table.get("entries");
                    let values = string_array_preserving_entries(existing, entries);
                    let mut replacement = toml_edit::Value::from(values);
                    let existing = table.get("entries").map(|value| value.decor().clone());
                    if let Some(decor) = existing {
                        *replacement.decor_mut() = decor;
                    }
                    table["entries"] = replacement;
                }
                _ => return Err(self.invalid_config("acronyms must be a table")),
            }
        } else {
            let values = string_array_preserving_entries(None, entries);
            let mut acronyms = toml_edit::Table::new();
            acronyms["entries"] = toml_edit::value(values);
            self.document["acronyms"] = toml_edit::Item::Table(acronyms);
        }

        self.validate_or_restore(original)
    }

    fn set_optional_string(
        &mut self,
        field: &'static str,
        value: Option<&str>,
    ) -> Result<(), ConfigEditorError> {
        let original = self.document.clone();
        match value {
            Some(value) => {
                set_value_preserving_decor(
                    self.document.as_table_mut(),
                    field,
                    toml_edit::value(value),
                );
            }
            None => {
                self.document.as_table_mut().remove(field);
            }
        }

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

    /// Validate and atomically write changes if the file has not changed externally.
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
    set_value_preserving_decor(table, "name", toml_edit::value(&rule.name));
    set_value_preserving_decor(table, "match", toml_edit::value(&rule.pattern));
    set_value_preserving_decor(table, "to", toml_edit::value(&rule.to));
    match rule.kind {
        RuleKind::Any => {
            table.remove("kind");
        }
        RuleKind::File => set_value_preserving_decor(table, "kind", toml_edit::value("file")),
        RuleKind::Directory => {
            set_value_preserving_decor(table, "kind", toml_edit::value("directory"));
        }
    }
    if rule.exclude.is_empty() {
        table.remove("exclude");
    } else {
        let existing = table.get("exclude").and_then(toml_edit::Item::as_value);
        let exclude = string_array_preserving_entries(existing, &rule.exclude);
        set_value_preserving_decor(table, "exclude", toml_edit::value(exclude));
    }
}

fn string_array_preserving_entries(
    existing: Option<&toml_edit::Value>,
    entries: &[String],
) -> toml_edit::Array {
    let mut array = existing
        .and_then(toml_edit::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut existing_values = array.iter().cloned().collect::<Vec<_>>();
    array.clear();

    for entry in entries {
        let value = existing_values
            .iter()
            .position(|value| value.as_str() == Some(entry))
            .map(|index| existing_values.remove(index))
            .unwrap_or_else(|| toml_edit::Value::from(entry.as_str()));
        array.push_formatted(value);
    }

    array
}

fn set_value_preserving_decor(table: &mut toml_edit::Table, key: &str, mut item: toml_edit::Item) {
    if let (Some(existing), Some(replacement)) = (
        table.get(key).and_then(toml_edit::Item::as_value),
        item.as_value_mut(),
    ) {
        *replacement.decor_mut() = existing.decor().clone();
    }

    table[key] = item;
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
            Self::NonUnicodeTarget(path) => write!(
                formatter,
                "configuration target must be valid Unicode: {}",
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
            | Self::NonUnicodeTarget(_)
            | Self::ConcurrentModification(_)
            | Self::InvalidRuleIndex { .. } => None,
        }
    }
}
