use crate::{CategoryCatalog, Preview, PreviewError, preview, preview_with_entry_limit};
use dirule_core::{AcronymOptions, Config, Rule, RuleKind};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Preview sorting files with one extension into a pluralized destination.
///
/// `extension` may include a leading dot. Matching is case-insensitive.
pub fn sort_extension_preview(
    target: &Path,
    extension: &str,
    recursive: bool,
) -> Result<Preview, PreviewError> {
    let selector = if extension.starts_with('.') {
        extension.to_owned()
    } else {
        format!(".{extension}")
    };
    sort_selectors_preview(target, &[selector], &CategoryCatalog::built_in(), recursive)
}

/// Preview a sort selected by file extensions, category names, or both.
///
/// `recursive` includes files in nested directories; destinations remain
/// relative to the target root.
pub fn sort_selectors_preview(
    target: &Path,
    selectors: &[String],
    categories: &CategoryCatalog,
    recursive: bool,
) -> Result<Preview, PreviewError> {
    sort_selectors_preview_with_entry_limit(target, selectors, categories, recursive, None)
}

/// Preview sorting selected extensions or categories with an optional scan limit.
pub fn sort_selectors_preview_with_entry_limit(
    target: &Path,
    selectors: &[String],
    categories: &CategoryCatalog,
    recursive: bool,
    maximum_entries: Option<usize>,
) -> Result<Preview, PreviewError> {
    if selectors.is_empty() {
        return Err(PreviewError::Operation(
            "choose at least one extension or category".into(),
        ));
    }

    let mut destinations = BTreeMap::<String, BTreeSet<String>>::new();
    let mut extension_destinations = BTreeMap::<String, String>::new();
    for selector in selectors {
        let forced_extension = selector.starts_with('.');
        let category_name = selector.to_ascii_lowercase();
        if !forced_extension && let Some(category) = categories.get(&category_name) {
            for extension in &category.extensions {
                assign_destination(
                    &mut extension_destinations,
                    extension,
                    &category.destination,
                )?;
                destinations
                    .entry(category.destination.clone())
                    .or_default()
                    .insert(extension.clone());
            }

            continue;
        }

        let extension = normalize_extension(selector)?;
        let destination = format!("{extension}s");
        assign_destination(&mut extension_destinations, &extension, &destination)?;
        destinations
            .entry(destination)
            .or_default()
            .insert(extension);
    }

    let rules = destinations
        .into_iter()
        .map(|(destination, extensions)| {
            let extensions = extensions.into_iter().collect::<Vec<_>>();
            let extension_pattern = match extensions.as_slice() {
                [extension] => extension.clone(),
                _ => format!("{{{}}}", extensions.join(",")),
            };
            let pattern = if recursive {
                format!("**/?*.(?i){extension_pattern}")
            } else {
                format!("?*.(?i){extension_pattern}")
            };

            Rule {
                name: format!("Sort into {destination}"),
                pattern,
                kind: RuleKind::File,
                exclude: Vec::new(),
                to: format!("template:{destination}/${{name}}"),
            }
        })
        .collect();

    preview_with_entry_limit(&quick_config(rules), target, maximum_entries)
}

/// Preview sorting files into a destination for each normalized extension.
pub fn sort_by_extension_preview(target: &Path, recursive: bool) -> Result<Preview, PreviewError> {
    let pattern = if recursive { "**/?*.*" } else { "?*.*" };
    let config = quick_config(vec![Rule {
        name: "Sort files by extension".into(),
        pattern: pattern.into(),
        kind: RuleKind::File,
        exclude: Vec::new(),
        to: "template:${extension|kebab-case}s/${name}".into(),
    }]);

    preview(&config, target)
}

fn normalize_extension(value: &str) -> Result<String, PreviewError> {
    let extension = value.strip_prefix('.').unwrap_or(value);
    if extension.is_empty() || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(PreviewError::Operation(format!(
            "invalid sort selector {value:?}: extensions must contain only ASCII letters and digits"
        )));
    }

    Ok(extension.to_ascii_lowercase())
}

fn assign_destination(
    destinations: &mut BTreeMap<String, String>,
    extension: &str,
    destination: &str,
) -> Result<(), PreviewError> {
    if let Some(existing) = destinations.get(extension) {
        if existing != destination {
            return Err(PreviewError::Operation(format!(
                "sort selectors overlap: extension {extension:?} maps to both {existing:?} and {destination:?}"
            )));
        }

        return Ok(());
    }

    destinations.insert(extension.into(), destination.into());

    Ok(())
}

fn quick_config(rules: Vec<Rule>) -> Config {
    Config {
        name: Some("Quick operation".into()),
        target: None,
        ignore: Vec::new(),
        acronyms: AcronymOptions::default(),
        rules,
    }
}
