use crate::config::CompiledDestination;
use crate::{
    CompiledConfig, Config, DestinationValues, Error, Finding, FindingKind, Result, RuleKind,
    transform_directory, transform_filename,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    DuplicateDestination,
    DestinationOccupied,
    FileDestinationParent,
    UnsafeParent,
    DependentMove,
}

impl ConflictKind {
    pub fn suggestion(self) -> &'static str {
        match self {
            Self::DuplicateDestination => {
                "Adjust the rules so each entry maps to a unique destination."
            }
            Self::DestinationOccupied => {
                "Choose an unoccupied destination or include its current owner in the plan."
            }
            Self::FileDestinationParent => {
                "Choose destinations that do not place entries beneath a file."
            }
            Self::UnsafeParent => {
                "Choose a destination with safe parent directories inside the target."
            }
            Self::DependentMove => "Resolve the related conflict and review the plan again.",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Conflict {
    pub path: PathBuf,
    pub kind: ConflictKind,
    pub related_paths: Vec<PathBuf>,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Plan {
    pub entries: Vec<PlannedEntry>,
    pub conflicts: Vec<Conflict>,
    pub findings: Vec<Finding>,
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
            result.findings.push(Finding::new(
                FindingKind::SkippedNonUnicodePath,
                None,
                format!(
                    "skipped path that cannot be represented as Unicode: {}",
                    entry.path.display()
                ),
            ));
        } else {
            ordered.push(entry);
        }
    }

    ordered.sort_by_cached_key(|entry| (depth(&entry.path), entry.path.as_path()));
    let original_paths: HashSet<PathBuf> = ordered
        .iter()
        .map(|entry| clean_path(&entry.path))
        .collect::<Result<_>>()?;

    let mut directory_destinations = HashMap::<PathBuf, PathBuf>::new();
    for entry in ordered {
        let source = clean_path(&entry.path)?;
        if entry.kind == EntryKind::Symlink {
            result.findings.push(Finding::new(
                FindingKind::SkippedSymlink,
                Some(&source),
                format!("skipped symlink {}", display_path(&source)),
            ));
            continue;
        }

        let planned_parent = virtual_parent(&source, &directory_destinations);
        let source_text = display_path(&source);
        let mut destination = source.clone();
        let mut owner = None;
        for rule in &config.rules {
            if !rule.kind.applies_to(entry.kind) {
                continue;
            }

            let captures = match &rule.destination {
                CompiledDestination::Builtin(_) => {
                    if !rule.matcher.is_match(&source_text) {
                        continue;
                    }

                    None
                }
                CompiledDestination::Template(_) => {
                    let Some(captures) = rule.matcher.captures(&source_text) else {
                        continue;
                    };

                    Some(captures)
                }
            };

            if rule
                .exclude
                .iter()
                .any(|pattern| pattern.is_match(&source_text))
            {
                continue;
            }

            let basename = source
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    Error::Config(format!("path is not valid Unicode: {source_text}"))
                })?;
            let (stem, extension) = split_extension(basename);
            destination = match &rule.destination {
                CompiledDestination::Builtin(operation) => {
                    let transformed = match entry.kind {
                        EntryKind::Directory => {
                            transform_directory(basename, *operation, &config.acronyms)
                        }
                        EntryKind::File => {
                            transform_filename(basename, *operation, &config.acronyms)
                        }
                        EntryKind::Symlink => unreachable!("symlinks are skipped before matching"),
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
                        captures: captures
                            .as_ref()
                            .expect("template rules collect their named captures"),
                    };
                    template.expand(&values, &config.acronyms)?
                }
            };
            owner = Some(rule.name.clone());
            break;
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
    let moved_sources = result
        .entries
        .iter()
        .filter(|entry| entry.source != entry.destination)
        .map(|entry| entry.source.clone())
        .collect();
    find_conflicts(&mut result, &original_paths, &moved_sources);
    Ok(result)
}

impl RuleKind {
    fn applies_to(self, entry_kind: EntryKind) -> bool {
        matches!(
            (self, entry_kind),
            (Self::Any, EntryKind::File | EntryKind::Directory)
                | (Self::File, EntryKind::File)
                | (Self::Directory, EntryKind::Directory)
        )
    }
}

fn find_conflicts(
    plan: &mut Plan,
    original_paths: &HashSet<PathBuf>,
    moved_sources: &HashSet<PathBuf>,
) {
    let mut destinations = HashMap::<PathBuf, (usize, Vec<PathBuf>)>::new();
    for entry in &plan.entries {
        if entry.source != entry.destination {
            let (owners, related_paths) =
                destinations.entry(entry.destination.clone()).or_default();
            *owners += 1;
            related_paths.extend([entry.source.clone(), entry.destination.clone()]);
        }
    }

    let mut destination_conflicts = destinations
        .into_iter()
        .filter_map(|(destination, (owners, mut related_paths))| {
            related_paths.sort_unstable();
            related_paths.dedup();

            if owners > 1 {
                Some(Conflict {
                    path: destination,
                    kind: ConflictKind::DuplicateDestination,
                    related_paths,
                    reason: "multiple entries have the same planned destination".into(),
                })
            } else if original_paths.contains(&destination) && !moved_sources.contains(&destination)
            {
                Some(Conflict {
                    path: destination,
                    kind: ConflictKind::DestinationOccupied,
                    related_paths,
                    reason: "destination already exists in the scanned tree".into(),
                })
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    destination_conflicts.sort_by(|left, right| left.path.cmp(&right.path));
    plan.conflicts.extend(destination_conflicts);
    let mut file_destinations = HashMap::<PathBuf, Vec<usize>>::new();
    for (index, entry) in plan.entries.iter().enumerate() {
        if entry.kind == EntryKind::File && entry.source != entry.destination {
            file_destinations
                .entry(entry.destination.clone())
                .or_default()
                .push(index);
        }
    }

    let mut file_parent_conflicts = vec![Vec::new(); plan.entries.len()];
    for entry in plan
        .entries
        .iter()
        .filter(|entry| entry.source != entry.destination)
    {
        let mut parent = entry.destination.parent();
        while let Some(path) = parent {
            if let Some(file_indices) = file_destinations.get(path) {
                for index in file_indices {
                    let related_paths = &mut file_parent_conflicts[*index];
                    related_paths.extend([
                        plan.entries[*index].source.clone(),
                        plan.entries[*index].destination.clone(),
                        entry.source.clone(),
                        entry.destination.clone(),
                    ]);
                }
            }

            parent = path.parent();
        }
    }

    for (index, mut related_paths) in file_parent_conflicts.into_iter().enumerate() {
        if !related_paths.is_empty() {
            related_paths.sort_unstable();
            related_paths.dedup();

            plan.conflicts.push(Conflict {
                path: plan.entries[index].destination.clone(),
                kind: ConflictKind::FileDestinationParent,
                related_paths,
                reason: "a file destination is also a parent of another destination".into(),
            });
        }
    }
}

fn virtual_parent(path: &Path, destinations: &HashMap<PathBuf, PathBuf>) -> PathBuf {
    let mut ancestor = path.parent();
    while let Some(parent) = ancestor {
        if let Some(destination) = destinations.get(parent) {
            let suffix = path.strip_prefix(parent).unwrap_or(path);
            return suffix
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map_or_else(|| destination.clone(), |parent| destination.join(parent));
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
