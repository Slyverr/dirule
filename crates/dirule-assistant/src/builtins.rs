use crate::{
    ApprovalRequirement, ChangeProposal, PreparedToolCall, Tool, ToolCatalog, ToolChange,
    ToolDefinition, ToolError,
};
use dirule_app::configuration::{
    ConfigurationLoadError, ConfigurationSource, ConfigurationSourceKind,
};
use dirule_app::journal::JournalService;
use dirule_app::presets::PresetStore;
use dirule_app::{
    CategoryCatalog, ConflictPolicy, Preview, PreviewSourceError, apply_authorized, inspect_target,
    rename_preview_with_entry_limit, sort_selectors_preview_with_entry_limit,
};
use dirule_core::{EntryKind, Transform};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

const MAX_PREVIEW_CHANGES: usize = 40;
const MAX_PREVIEW_RESULT_BYTES: usize = 24 * 1024;
const MAX_SCANNED_ENTRIES: usize = 10_000;
const MAX_TARGET_LISTING_ENTRIES: usize = 100;
const MAX_PRESET_LISTING_ENTRIES: usize = 64;
const MAX_APPROVED_CHANGES: usize = MAX_PREVIEW_CHANGES;

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

    build_tool_catalog(target.clone(), target, categories, None, presets, None)
}

pub fn dirule_tool_catalog_with_source(
    target: PathBuf,
    base: PathBuf,
    categories: CategoryCatalog,
    source: Option<ConfigurationSource>,
) -> Result<ToolCatalog, ToolError> {
    let presets = user_presets()?;
    let journal = JournalService::user().ok().map(Arc::new);

    build_tool_catalog(target, base, categories, source, presets, journal)
}

/// Build a tool catalog with explicit preset storage for callers and tests.
pub fn dirule_tool_catalog_with_sources(
    target: PathBuf,
    base: PathBuf,
    categories: CategoryCatalog,
    source: Option<ConfigurationSource>,
    presets: Option<PresetStore>,
) -> Result<ToolCatalog, ToolError> {
    build_tool_catalog(target, base, categories, source, presets, None)
}

/// Build a tool catalog with injected preset and journal services.
pub fn dirule_tool_catalog_with_services(
    target: PathBuf,
    base: PathBuf,
    categories: CategoryCatalog,
    source: Option<ConfigurationSource>,
    presets: Option<PresetStore>,
    journal: Option<Arc<JournalService>>,
) -> Result<ToolCatalog, ToolError> {
    build_tool_catalog(target, base, categories, source, presets, journal)
}

fn build_tool_catalog(
    target: PathBuf,
    base: PathBuf,
    categories: CategoryCatalog,
    selected_source: Option<ConfigurationSource>,
    presets: Option<PresetStore>,
    journal: Option<Arc<JournalService>>,
) -> Result<ToolCatalog, ToolError> {
    let apply_services = journal.map(|journal| {
        (
            target.clone(),
            base.clone(),
            categories.clone(),
            selected_source.clone(),
            presets.clone(),
            journal,
        )
    });
    let mut tools: Vec<Box<dyn Tool>> = vec![
        Box::new(InspectTargetTool {
            target: target.clone(),
        }),
        Box::new(ListConfigurationSourcesTool {
            target: target.clone(),
            selected_source: selected_source.clone(),
            presets: presets.clone(),
        }),
        Box::new(ListCategoriesTool {
            categories: categories.clone(),
        }),
        Box::new(PreviewSortTool {
            target: target.clone(),
            categories: categories.clone(),
        }),
        Box::new(PreviewRenameTool {
            target: target.clone(),
        }),
        Box::new(InspectRulesTool {
            selected_source: selected_source.clone(),
            presets: presets.clone(),
        }),
        Box::new(PreviewRulesTool {
            selected_source,
            base,
            target,
            presets,
        }),
    ];

    if let Some((target, base, categories, selected_source, presets, journal)) = apply_services {
        tools.push(Box::new(ApplySortTool {
            target: target.clone(),
            categories: categories.clone(),
            journal: Arc::clone(&journal),
        }));
        tools.push(Box::new(ApplyRenameTool {
            target: target.clone(),
            journal: Arc::clone(&journal),
        }));
        tools.push(Box::new(ApplyRulesTool {
            selected_source,
            base,
            target,
            presets,
            journal,
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
    selected_source: Option<ConfigurationSource>,
    presets: Option<PresetStore>,
}

struct PreviewRulesTool {
    selected_source: Option<ConfigurationSource>,
    base: PathBuf,
    target: PathBuf,
    presets: Option<PresetStore>,
}

struct ApplySortTool {
    target: PathBuf,
    categories: CategoryCatalog,
    journal: Arc<JournalService>,
}

struct ApplyRenameTool {
    target: PathBuf,
    journal: Arc<JournalService>,
}

struct ApplyRulesTool {
    selected_source: Option<ConfigurationSource>,
    base: PathBuf,
    target: PathBuf,
    presets: Option<PresetStore>,
    journal: Arc<JournalService>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleSourceArguments {
    #[serde(default)]
    preset: Option<String>,
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
                        source.kind() == ConfigurationSourceKind::Preset
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
            parameters: sort_arguments_schema(),
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
            parameters: rename_arguments_schema(),
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
            description: "Read naming rules and exclusions from the selected config, or from a named global preset. This sends rule patterns and templates to the model but does not read target file contents.".into(),
            parameters: rule_source_arguments_schema(),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        let arguments: RuleSourceArguments = parse_arguments(arguments)?;
        let source = resolve_rule_source(
            self.selected_source.as_ref(),
            self.presets.as_ref(),
            arguments.preset.as_deref(),
        )?;

        let loaded = dirule_app::configuration::load_source(&source)
            .map_err(|error| ToolError::new(error.to_string()))?;
        let config = loaded.config();
        let result = json!({
            "source": source.name(),
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
            description: "Preview rules from the selected config, or from a named global preset, on the current target. This never changes paths.".into(),
            parameters: rule_source_arguments_schema(),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        let arguments: RuleSourceArguments = parse_arguments(arguments)?;
        let source = resolve_rule_source(
            self.selected_source.as_ref(),
            self.presets.as_ref(),
            arguments.preset.as_deref(),
        )?;

        let preview = dirule_app::preview_source_with_entry_limit(
            &source,
            &self.base,
            Some(self.target.clone()),
            Some(MAX_SCANNED_ENTRIES),
        )
        .map_err(preview_source_error)?;

        Ok(PreparedToolCall::ReadOnly(preview_result(&preview)?))
    }
}

impl Tool for ApplySortTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "apply_sort".into(),
            description: "Plan sorting by file extension or category, then apply only after the user approves the exact path changes in the terminal.".into(),
            parameters: sort_arguments_schema(),
            approval: ApprovalRequirement::Required,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        let arguments: SortArguments = parse_arguments(arguments)?;
        let preview = sort_selectors_preview_with_entry_limit(
            &self.target,
            &arguments.selectors,
            &self.categories,
            arguments.recursive,
            Some(MAX_SCANNED_ENTRIES),
        )
        .map_err(|error| ToolError::new(error.to_string()))?;

        prepare_apply(preview, Arc::clone(&self.journal), "Apply sorting")
    }
}

impl Tool for ApplyRenameTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "apply_rename".into(),
            description: "Plan a built-in naming transformation, then apply only after the user approves the exact path changes in the terminal.".into(),
            parameters: rename_arguments_schema(),
            approval: ApprovalRequirement::Required,
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

        prepare_apply(preview, Arc::clone(&self.journal), "Apply renaming")
    }
}

impl Tool for ApplyRulesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "apply_configured_rules".into(),
            description: "Preview configured rules, then apply only after the user approves the exact path changes in the terminal. An optional preset selects a named global preset.".into(),
            parameters: rule_source_arguments_schema(),
            approval: ApprovalRequirement::Required,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        let arguments: RuleSourceArguments = parse_arguments(arguments)?;
        let source = resolve_rule_source(
            self.selected_source.as_ref(),
            self.presets.as_ref(),
            arguments.preset.as_deref(),
        )?;
        let preview = dirule_app::preview_source_with_entry_limit(
            &source,
            &self.base,
            Some(self.target.clone()),
            Some(MAX_SCANNED_ENTRIES),
        )
        .map_err(preview_source_error)?;

        prepare_apply(preview, Arc::clone(&self.journal), "Apply configured rules")
    }
}

fn prepare_apply(
    preview: Preview,
    journal: Arc<JournalService>,
    operation: &str,
) -> Result<PreparedToolCall, ToolError> {
    let plan = preview.plan();
    if !plan.conflicts.is_empty() {
        return Err(ToolError::new(format!(
            "plan has {} conflict(s); no changes were applied",
            plan.conflicts.len()
        )));
    }
    if !plan.has_changes() {
        return Ok(PreparedToolCall::ReadOnly(
            json!({ "status": "no_changes" }),
        ));
    }
    if plan.entries.len() > MAX_APPROVED_CHANGES {
        return Err(ToolError::new(format!(
            "Ask can approve at most {MAX_APPROVED_CHANGES} changes at once; use Dirule's normal apply command for larger plans"
        )));
    }

    let changes = plan
        .entries
        .iter()
        .map(|entry| ToolChange {
            source: entry.source.display().to_string(),
            destination: entry.destination.display().to_string(),
        })
        .collect::<Vec<_>>();
    let finding_count = plan.findings.len();
    let summary = format!(
        "{operation}: {} path change(s) under target {:?}; {finding_count} finding(s)",
        changes.len(),
        preview.target()
    );
    let proposal = ChangeProposal { summary, changes };
    if encoded_size(&json!({
        "summary": &proposal.summary,
        "changes": &proposal.changes,
    }))? > MAX_PREVIEW_RESULT_BYTES
    {
        return Err(ToolError::new(
            "the exact change list exceeds the approval display limit; use Dirule's normal apply command",
        ));
    }

    let action = Box::new(move || {
        let applied = apply_authorized(preview, ConflictPolicy::Abort, &journal)
            .map_err(|error| ToolError::new(error.to_string()))?;

        Ok(json!({
            "status": "applied",
            "moved": applied.report.moved.len(),
            "finding_count": finding_count,
            "run_id": applied.run_id,
        }))
    });

    Ok(PreparedToolCall::Mutation { proposal, action })
}

fn rule_source_arguments_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "preset": {
                "type": "string",
                "description": "Optional global preset name. Omit to use the configuration source selected by the CLI."
            }
        },
        "additionalProperties": false
    })
}

fn sort_arguments_schema() -> Value {
    json!({
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
    })
}

fn rename_arguments_schema() -> Value {
    json!({
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
    })
}

fn resolve_rule_source(
    selected_source: Option<&ConfigurationSource>,
    presets: Option<&PresetStore>,
    preset_name: Option<&str>,
) -> Result<ConfigurationSource, ToolError> {
    if let Some(name) = preset_name {
        let store =
            presets.ok_or_else(|| ToolError::new("global preset storage is unavailable"))?;

        return ConfigurationSource::preset(store, name)
            .map_err(|_| ToolError::new("invalid global preset name"));
    }

    selected_source.cloned().ok_or_else(|| {
        ToolError::new(
            "no rule source is selected; choose a listed global preset or rerun with --config or --preset",
        )
    })
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
