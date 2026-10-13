use crate::config::CompiledDestination;
use crate::{
    CompiledConfig, Config, DestinationValues, Error, Result, transform, transform_filename,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub kind: EntryKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlannedEntry {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub kind: EntryKind,
    pub rule: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Conflict {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Plan {
    pub entries: Vec<PlannedEntry>,
    pub conflicts: Vec<Conflict>,
    pub findings: Vec<String>,
}

impl Plan {
    pub fn has_changes(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.source != entry.destination)
    }

    pub fn is_conflict_free(&self) -> bool {
        self.conflicts.is_empty()
    }
}

pub fn plan(config: &Config, entries: &[Entry]) -> Result<Plan> {
    let compiled = config.compile()?;
    plan_compiled(&compiled, entries)
}

pub fn plan_compiled(config: &CompiledConfig, entries: &[Entry]) -> Result<Plan> {
    let mut result = Plan::default();
    let mut ordered = Vec::with_capacity(entries.len());
    for entry in entries {
        if has_non_unicode_component(&entry.path) {
            result.findings.push(format!(
                "skipped path that cannot be represented as Unicode: {}",
                entry.path.display()
            ));
        } else {
            ordered.push(entry.clone());
        }
    }

    ordered.sort_by(|left, right| {
        depth(&left.path)
            .cmp(&depth(&right.path))
            .then_with(|| left.path.cmp(&right.path))
    });
    let original_paths: BTreeSet<PathBuf> = ordered
        .iter()
        .map(|entry| clean_path(&entry.path))
        .collect::<Result<_>>()?;

    let mut directory_destinations = BTreeMap::<PathBuf, PathBuf>::new();
    for entry in ordered {
        let source = clean_path(&entry.path)?;
        if entry.kind == EntryKind::Symlink {
            result
                .findings
                .push(format!("skipped symlink {}", display_path(&source)));
            continue;
        }

        let planned_parent = virtual_parent(&source, &directory_destinations);
        let source_text = display_path(&source);
        let mut destination = source.clone();
        let mut owner = None;
        for rule in &config.rules {
            if let Some(captures) = rule.matcher.captures(&source_text) {
                let basename = source
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| {
                        Error::Config(format!("path is not valid Unicode: {source_text}"))
                    })?;
                let (stem, extension) = split_extension(basename);
                destination = match &rule.destination {
                    CompiledDestination::Builtin(operation) => {
                        let transformed = if entry.kind == EntryKind::Directory {
                            transform(basename, *operation, &config.acronyms)
                        } else {
                            transform_filename(basename, *operation, &config.acronyms)
                        };
                        planned_parent.join(transformed)
                    }
                    CompiledDestination::Template(template) => {
                        let parent = display_path(&planned_parent);
                        let values = DestinationValues {
                            path: &source_text,
                            parent: &parent,
                            name: basename,
                            stem,
                            extension,
                            captures: &captures,
                        };
                        template.expand(&values, &config.acronyms)?
                    }
                };
                owner = Some(rule.name.clone());
                break;
            }
        }
        if owner.is_none() {
            let basename = source.file_name().unwrap_or_default();
            destination = planned_parent.join(basename);
        }

        if entry.kind == EntryKind::Directory {
            directory_destinations.insert(source.clone(), destination.clone());
        }
        result.entries.push(PlannedEntry {
            source,
            destination,
            kind: entry.kind,
            rule: owner,
        });
    }
    find_conflicts(&mut result, &original_paths);
    Ok(result)
}

fn find_conflicts(plan: &mut Plan, original_paths: &BTreeSet<PathBuf>) {
    let mut destinations = BTreeMap::<PathBuf, Vec<usize>>::new();
    for (index, entry) in plan.entries.iter().enumerate() {
        if entry.source != entry.destination {
            destinations
                .entry(entry.destination.clone())
                .or_default()
                .push(index);
        }
    }

    for (destination, owners) in destinations {
        if owners.len() > 1 {
            plan.conflicts.push(Conflict {
                path: destination,
                reason: "multiple entries have the same planned destination".into(),
            });
        } else if original_paths.contains(&destination) {
            plan.conflicts.push(Conflict {
                path: destination,
                reason: "destination already exists in the scanned tree".into(),
            });
        }
    }
    let mut file_destinations = BTreeMap::<PathBuf, Vec<usize>>::new();
    for (index, entry) in plan.entries.iter().enumerate() {
        if entry.kind == EntryKind::File && entry.source != entry.destination {
            file_destinations
                .entry(entry.destination.clone())
                .or_default()
                .push(index);
        }
    }

    let mut file_parent_conflicts = vec![false; plan.entries.len()];
    for entry in plan
        .entries
        .iter()
        .filter(|entry| entry.source != entry.destination)
    {
        let mut parent = entry.destination.parent();
        while let Some(path) = parent {
            if let Some(file_indices) = file_destinations.get(path) {
                for index in file_indices {
                    file_parent_conflicts[*index] = true;
                }
            }

            parent = path.parent();
        }
    }

    for (index, entry) in plan.entries.iter().enumerate() {
        if file_parent_conflicts[index] {
            plan.conflicts.push(Conflict {
                path: entry.destination.clone(),
                reason: "a file destination is also a parent of another destination".into(),
            });
        }
    }
}

fn virtual_parent(path: &Path, destinations: &BTreeMap<PathBuf, PathBuf>) -> PathBuf {
    let mut ancestor = path.parent();
    while let Some(parent) = ancestor {
        if let Some(destination) = destinations.get(parent) {
            let suffix = path.strip_prefix(parent).unwrap_or(path);
            return destination.join(suffix.parent().unwrap_or_else(|| Path::new("")));
        }
        ancestor = parent.parent();
    }
    path.parent().unwrap_or_else(|| Path::new("")).to_path_buf()
}

fn clean_path(path: &Path) -> Result<PathBuf> {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) if value.to_str().is_some() => output.push(value),
            _ => {
                return Err(Error::Config(format!(
                    "entry path must be a Unicode target-relative path: {}",
                    path.display()
                )));
            }
        }
    }
    if output.as_os_str().is_empty() {
        return Err(Error::Config("entry path cannot be empty".into()));
    }
    Ok(output)
}

fn depth(path: &Path) -> usize {
    path.components().count()
}

fn has_non_unicode_component(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str().to_str().is_none())
}

fn display_path(path: &Path) -> String {
    if cfg!(windows) {
        path.to_string_lossy().replace('\\', "/")
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 => (&name[..index], &name[index + 1..]),
        _ => (name, ""),
    }
}
