use crate::{ApprovalRequirement, PreparedToolCall, Tool, ToolCatalog, ToolDefinition, ToolError};
use dirule_app::configuration::{ConfigurationLoadError, ConfigurationSource};
use dirule_app::{
    CategoryCatalog, Preview, PreviewSourceError, rename_preview, sort_selectors_preview,
};
use dirule_core::Transform;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

const MAX_PREVIEW_CHANGES: usize = 40;
const MAX_PREVIEW_RESULT_BYTES: usize = 24 * 1024;

pub fn dirule_tool_catalog(target: PathBuf) -> Result<ToolCatalog, ToolError> {
    let categories =
        CategoryCatalog::user_if_available().map_err(|error| ToolError::new(error.to_string()))?;
    dirule_tool_catalog_with_categories(target, categories)
}

pub fn dirule_tool_catalog_with_categories(
    target: PathBuf,
    categories: CategoryCatalog,
) -> Result<ToolCatalog, ToolError> {
    build_tool_catalog(target, categories, None)
}

pub fn dirule_tool_catalog_with_source(
    target: PathBuf,
    base: PathBuf,
    categories: CategoryCatalog,
    source: Option<ConfigurationSource>,
) -> Result<ToolCatalog, ToolError> {
    let selected_source = source.map(|source| (source, base));

    build_tool_catalog(target, categories, selected_source)
}

fn build_tool_catalog(
    target: PathBuf,
    categories: CategoryCatalog,
    source: Option<(ConfigurationSource, PathBuf)>,
) -> Result<ToolCatalog, ToolError> {
    let tools: Vec<Box<dyn Tool>> = vec![
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

struct ListCategoriesTool {
    categories: CategoryCatalog,
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

        let preview = sort_selectors_preview(
            &self.target,
            &arguments.selectors,
            &self.categories,
            arguments.recursive,
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
        let preview = rename_preview(
            &self.target,
            arguments.transform.into(),
            arguments.recursive,
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

        let preview =
            dirule_app::preview_source(&self.source, &self.base, Some(self.target.clone()))
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
    let mut changes = plan
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
    let mut result = json!({
        "target": preview.target(),
        "change_count": plan.entries.len(),
        "conflicts": plan.conflicts,
        "findings": plan.findings,
        "changes_truncated": plan.entries.len() > changes.len(),
        "changes": changes,
    });

    while serde_json::to_vec(&result)
        .map_err(|error| ToolError::new(format!("could not encode preview: {error}")))?
        .len()
        > MAX_PREVIEW_RESULT_BYTES
        && changes.len() > 1
    {
        changes.truncate(changes.len() / 2);
        result["changes"] = json!(changes);
        result["changes_truncated"] = json!(true);
    }
    let size = serde_json::to_vec(&result)
        .map_err(|error| ToolError::new(format!("could not encode preview: {error}")))?
        .len();
    if size > MAX_PREVIEW_RESULT_BYTES {
        return Err(ToolError::new(
            "preview summary exceeds the tool output limit",
        ));
    }

    Ok(result)
}
