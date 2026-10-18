use crate::{ApprovalRequirement, PreparedToolCall, Tool, ToolCatalog, ToolDefinition, ToolError};
use dirule_app::configuration::{ConfigurationLoadError, ConfigurationSource};
use dirule_app::presets::PresetStore;
use dirule_app::{
    CategoryCatalog, Preview, PreviewSourceError, inspect_target, rename_preview_with_entry_limit,
    sort_selectors_preview_with_entry_limit,
};
use dirule_core::{EntryKind, Transform};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

const MAX_PREVIEW_CHANGES: usize = 40;
const MAX_PREVIEW_RESULT_BYTES: usize = 24 * 1024;
const MAX_SCANNED_ENTRIES: usize = 10_000;
const MAX_TARGET_LISTING_ENTRIES: usize = 100;
const MAX_PRESET_LISTING_ENTRIES: usize = 64;

pub fn dirule_tool_catalog(target: PathBuf) -> Result<ToolCatalog, ToolError> {
    let categories =
        CategoryCatalog::user_if_available().map_err(|error| ToolError::new(error.to_string()))?;
    dirule_tool_catalog_with_categories(target, categories)
}

pub fn dirule_tool_catalog_with_categories(
    target: PathBuf,
    categories: CategoryCatalog,
) -> Result<ToolCatalog, ToolError> {
    let presets = user_presets()?;

    build_tool_catalog(target, categories, None, presets)
}

pub fn dirule_tool_catalog_with_source(
    target: PathBuf,
    base: PathBuf,
    categories: CategoryCatalog,
    source: Option<ConfigurationSource>,
) -> Result<ToolCatalog, ToolError> {
    let selected_source = source.map(|source| (source, base));
    let presets = user_presets()?;

    build_tool_catalog(target, categories, selected_source, presets)
}

/// Build a tool catalog with explicit preset storage for callers and tests.
pub fn dirule_tool_catalog_with_sources(
    target: PathBuf,
    base: PathBuf,
    categories: CategoryCatalog,
    source: Option<ConfigurationSource>,
    presets: Option<PresetStore>,
) -> Result<ToolCatalog, ToolError> {
    let selected_source = source.map(|source| (source, base));

    build_tool_catalog(target, categories, selected_source, presets)
}

fn build_tool_catalog(
    target: PathBuf,
    categories: CategoryCatalog,
    source: Option<(ConfigurationSource, PathBuf)>,
    presets: Option<PresetStore>,
) -> Result<ToolCatalog, ToolError> {
    let selected_source = source.as_ref().map(|(source, _)| source.clone());
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(InspectTargetTool {
            target: target.clone(),
        }),
        Box::new(ListConfigurationSourcesTool {
            target: target.clone(),
            selected_source,
            presets,
        }),
        Box::new(ListCategoriesTool {
            categories: categories.clone(),
        }),
        Box::new(PreviewSortTool {
            target: target.clone(),
            categories,
        }),
        Box::new(PreviewRenameTool {
            target: target.clone(),
        }),
    ];
    let mut tools = tools;

    if let Some((source, base)) = source {
        tools.push(Box::new(InspectRulesTool {
            source: source.clone(),
        }));
        tools.push(Box::new(PreviewRulesTool {
            source,
            base,
            target,
        }));
    }

    ToolCatalog::new(tools)
}

fn user_presets() -> Result<Option<PresetStore>, ToolError> {
    PresetStore::user_if_available().map_err(|_| ToolError::new("could not access global presets"))
}

struct InspectTargetTool {
    target: PathBuf,
}

struct ListCategoriesTool {
    categories: CategoryCatalog,
}

struct ListConfigurationSourcesTool {
    target: PathBuf,
    selected_source: Option<ConfigurationSource>,
    presets: Option<PresetStore>,
}

struct PreviewSortTool {
    target: PathBuf,
    categories: CategoryCatalog,
}

struct PreviewRenameTool {
    target: PathBuf,
}

struct InspectRulesTool {
    source: ConfigurationSource,
}

struct PreviewRulesTool {
    source: ConfigurationSource,
    base: PathBuf,
    target: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SortArguments {
    selectors: Vec<String>,
    #[serde(default)]
    recursive: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameArguments {
    transform: NameTransform,
    #[serde(default)]
    recursive: bool,
}

#[derive(Deserialize)]
enum NameTransform {
    #[serde(rename = "snake-case")]
    Snake,
    #[serde(rename = "kebab-case")]
    Kebab,
    #[serde(rename = "camel-case")]
    Camel,
    #[serde(rename = "pascal-case")]
    Pascal,
}

impl From<NameTransform> for Transform {
    fn from(transform: NameTransform) -> Self {
        match transform {
            NameTransform::Snake => Self::SnakeCase,
            NameTransform::Kebab => Self::KebabCase,
            NameTransform::Camel => Self::CamelCase,
            NameTransform::Pascal => Self::PascalCase,
        }
    }
}

impl Tool for ListCategoriesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_categories".into(),
            description: "List Dirule's effective file-sorting categories, their extensions, and destinations.".into(),
            parameters: empty_object_schema(),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        if !arguments.as_object().is_some_and(serde_json::Map::is_empty) {
            return Err(ToolError::new("`list_categories` accepts no arguments"));
        }

        let categories = self
            .categories
            .iter()
            .map(|category| {
                json!({
                    "name": category.name,
                    "extensions": category.extensions,
                    "destination": category.destination,
                })
            })
            .collect::<Vec<_>>();

        Ok(PreparedToolCall::ReadOnly(
            json!({ "categories": categories }),
        ))
    }
}

impl Tool for ListConfigurationSourcesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_configuration_sources".into(),
            description: "List whether the selected target has a local dirule.toml and the names of available global presets. This returns no configuration contents or storage paths.".into(),
            parameters: empty_object_schema(),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        require_no_arguments("list_configuration_sources", arguments)?;

        let project_config = ConfigurationSource::project_default(&self.target);
        let project_config_available = project_config.path().is_file();
        let project_config_selected = self
            .selected_source
            .as_ref()
            .is_some_and(|source| source.path() == project_config.path());
        let preset_names = self
            .presets
            .as_ref()
            .map(PresetStore::list)
            .transpose()
            .map_err(|_| ToolError::new("could not list global preset names"))?
            .unwrap_or_default();
        let listed_presets = preset_names
            .iter()
            .take(MAX_PRESET_LISTING_ENTRIES)
            .map(|name| {
                json!({
                    "name": name,
                    "selected": self.selected_source.as_ref().is_some_and(|source| {
                        source.kind() == dirule_app::configuration::ConfigurationSourceKind::Preset
                            && source.name() == name
                    }),
                })
            })
            .collect::<Vec<_>>();

        Ok(PreparedToolCall::ReadOnly(json!({
            "project_config": {
                "name": "dirule.toml",
                "available": project_config_available,
                "selected": project_config_selected,
            },
            "global_presets": {
                "count": preset_names.len(),
                "truncated": preset_names.len() > listed_presets.len(),
                "items": listed_presets,
            },
        })))
    }
}

impl Tool for InspectTargetTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "inspect_target".into(),
            description: "List a bounded sample of visible file and directory paths under the selected target. This does not read file contents.".into(),
            parameters: empty_object_schema(),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        require_no_arguments("inspect_target", arguments)?;

        let inspection = inspect_target(&self.target, MAX_SCANNED_ENTRIES)
            .map_err(|error| ToolError::new(error.to_string()))?;
        let entries = inspection
            .entries
            .iter()
            .take(MAX_TARGET_LISTING_ENTRIES)
            .map(|entry| {
                json!({
                    "path": entry.path,
                    "kind": entry_kind_name(entry.kind),
                })
            })
            .collect::<Vec<_>>();
        let listing_truncated =
            inspection.truncated || inspection.entries.len() > MAX_TARGET_LISTING_ENTRIES;

        let mut result = json!({
            "target": inspection.target,
            "entries_seen": inspection.entries.len(),
            "scan_truncated": inspection.truncated,
            "listing_truncated": listing_truncated,
            "entries": entries,
        });
        while encoded_size(&result)? > MAX_PREVIEW_RESULT_BYTES {
            let current = result["entries"].as_array().map_or(0, Vec::len);
            if current == 0 {
                return Err(ToolError::new(
                    "target inspection summary exceeds the tool output limit",
                ));
            }

            result["entries"]
                .as_array_mut()
                .expect("target inspection entries are an array")
                .truncate(current / 2);
            result["listing_truncated"] = json!(true);
        }

        Ok(PreparedToolCall::ReadOnly(result))
    }
}

impl Tool for PreviewSortTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "preview_sort".into(),
            description: "Preview sorting files in the selected target by extension or Dirule category. This never moves files.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "selectors": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "maxItems": 32,
                        "description": "Category names or file extensions, such as images, videos, pdf, or .tar.gz."
                    },
                    "recursive": {
                        "type": "boolean",
                        "default": false,
                        "description": "Include entries below the target's immediate children."
                    }
                },
                "required": ["selectors"],
                "additionalProperties": false
            }),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        let arguments: SortArguments = parse_arguments(arguments)?;
        if arguments.selectors.len() > 32 {
            return Err(ToolError::new(
                "a sort preview accepts at most 32 selectors",
            ));
        }

        let preview = sort_selectors_preview_with_entry_limit(
            &self.target,
            &arguments.selectors,
            &self.categories,
            arguments.recursive,
            Some(MAX_SCANNED_ENTRIES),
        )
        .map_err(|error| ToolError::new(error.to_string()))?;

        Ok(PreparedToolCall::ReadOnly(preview_result(&preview)?))
    }
}

impl Tool for PreviewRenameTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "preview_rename".into(),
            description: "Preview a built-in naming transformation over files and directories in the selected target. This never renames entries.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "transform": {
                        "type": "string",
                        "enum": ["snake-case", "kebab-case", "camel-case", "pascal-case"]
                    },
                    "recursive": {
                        "type": "boolean",
                        "default": false
                    }
                },
                "required": ["transform"],
                "additionalProperties": false
            }),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        let arguments: RenameArguments = parse_arguments(arguments)?;
        let preview = rename_preview_with_entry_limit(
            &self.target,
            arguments.transform.into(),
            arguments.recursive,
            Some(MAX_SCANNED_ENTRIES),
        )
        .map_err(|error| ToolError::new(error.to_string()))?;

        Ok(PreparedToolCall::ReadOnly(preview_result(&preview)?))
    }
}

impl Tool for InspectRulesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "inspect_rules".into(),
            description: "Read the selected Dirule config's naming rules and exclusions. This sends rule patterns and templates to the model but does not read target file contents.".into(),
            parameters: empty_object_schema(),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        require_no_arguments("inspect_rules", arguments)?;

        let loaded = dirule_app::configuration::load_source(&self.source)
            .map_err(|error| ToolError::new(error.to_string()))?;
        let config = loaded.config();
        let result = json!({
            "source": self.source.name(),
            "name": &config.name,
            "target": config.target.as_ref().map(|path| path.display().to_string()),
            "ignore": &config.ignore,
            "rules": &config.rules,
        });

        Ok(PreparedToolCall::ReadOnly(result))
    }
}

impl Tool for PreviewRulesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "preview_configured_rules".into(),
            description: "Preview the selected Dirule config's rules on the current target. This never changes paths.".into(),
            parameters: empty_object_schema(),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        require_no_arguments("preview_configured_rules", arguments)?;

        let preview = dirule_app::preview_source_with_entry_limit(
            &self.source,
            &self.base,
            Some(self.target.clone()),
            Some(MAX_SCANNED_ENTRIES),
        )
        .map_err(preview_source_error)?;

        Ok(PreparedToolCall::ReadOnly(preview_result(&preview)?))
    }
}

fn empty_object_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

fn entry_kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::File => "file",
        EntryKind::Directory => "directory",
        EntryKind::Symlink => "symlink",
    }
}

fn require_no_arguments(name: &str, arguments: Value) -> Result<(), ToolError> {
    if !arguments.as_object().is_some_and(serde_json::Map::is_empty) {
        return Err(ToolError::new(format!("`{name}` accepts no arguments")));
    }

    Ok(())
}

fn preview_source_error(error: PreviewSourceError) -> ToolError {
    match error {
        PreviewSourceError::Configuration(ConfigurationLoadError::Missing { source }) => {
            ToolError::new(format!(
                "configuration not found at {}",
                source.path().display()
            ))
        }
        error => ToolError::new(error.to_string()),
    }
}

fn parse_arguments<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T, ToolError> {
    serde_json::from_value(arguments)
        .map_err(|error| ToolError::new(format!("invalid tool arguments: {error}")))
}

fn preview_result(preview: &Preview) -> Result<Value, ToolError> {
    let plan = preview.plan();
    let changes = plan
        .entries
        .iter()
        .take(MAX_PREVIEW_CHANGES)
        .map(|entry| {
            json!({
                "from": entry.source,
                "to": entry.destination,
                "rule": entry.rule,
            })
        })
        .collect::<Vec<_>>();
    let conflicts = plan
        .conflicts
        .iter()
        .take(MAX_PREVIEW_CHANGES)
        .collect::<Vec<_>>();
    let findings = plan
        .findings
        .iter()
        .take(MAX_PREVIEW_CHANGES)
        .collect::<Vec<_>>();
    let mut result = json!({
        "target": preview.target(),
        "change_count": plan.entries.len(),
        "conflict_count": plan.conflicts.len(),
        "finding_count": plan.findings.len(),
        "changes_truncated": plan.entries.len() > changes.len(),
        "conflicts_truncated": plan.conflicts.len() > conflicts.len(),
        "findings_truncated": plan.findings.len() > findings.len(),
        "changes": changes,
        "conflicts": conflicts,
        "findings": findings,
    });

    while encoded_size(&result)? > MAX_PREVIEW_RESULT_BYTES {
        let mut reduced = false;
        for (field, truncated_field) in [
            ("changes", "changes_truncated"),
            ("conflicts", "conflicts_truncated"),
            ("findings", "findings_truncated"),
        ] {
            let Some(values) = result[field].as_array() else {
                continue;
            };
            let length = values.len();
            if length == 0 {
                continue;
            }

            let next_length = length / 2;
            result[field]
                .as_array_mut()
                .expect("preview field remains an array")
                .truncate(next_length);
            result[truncated_field] = json!(true);
            reduced = true;
        }
        if !reduced {
            break;
        }
    }

    let size = encoded_size(&result)?;
    if size > MAX_PREVIEW_RESULT_BYTES {
        return Err(ToolError::new(
            "preview summary exceeds the tool output limit",
        ));
    }

    Ok(result)
}

fn encoded_size(value: &Value) -> Result<usize, ToolError> {
    serde_json::to_vec(value)
        .map(|encoded| encoded.len())
        .map_err(|error| ToolError::new(format!("could not encode tool result: {error}")))
}
