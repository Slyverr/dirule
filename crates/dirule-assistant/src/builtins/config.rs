use super::{MAX_PREVIEW_RESULT_BYTES, encoded_size};
use crate::{
    ApprovalRequirement, ChangeProposal, PreparedToolCall, Tool, ToolChange, ToolDefinition,
    ToolError,
};
use dirule_app::configuration::create_config;
use dirule_core::{Config, parse_config};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_CONFIG_BYTES: usize = 16 * 1024;

pub(super) fn register(tools: &mut Vec<Box<dyn Tool>>, target: PathBuf) {
    tools.push(Box::new(DraftProjectConfigTool));
    tools.push(Box::new(SaveProjectConfigTool { target }));
}

struct DraftProjectConfigTool;

struct SaveProjectConfigTool {
    target: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigArguments {
    contents: String,
}

impl Tool for DraftProjectConfigTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "draft_project_config".into(),
            description: "Validate a proposed dirule.toml with Dirule's config reader and return the exact TOML. This does not create or modify a file.".into(),
            parameters: config_schema(),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        let arguments: ConfigArguments = super::parse_arguments(arguments)?;
        let config = validate_config(&arguments.contents)?;
        let result = config_result(&config, arguments.contents);
        ensure_size(&result)?;

        Ok(PreparedToolCall::ReadOnly(result))
    }
}

impl Tool for SaveProjectConfigTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "save_project_config".into(),
            description: "Create a validated project dirule.toml only after the user approves its exact contents. It never overwrites an existing file.".into(),
            parameters: config_schema(),
            approval: ApprovalRequirement::Required,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        let arguments: ConfigArguments = super::parse_arguments(arguments)?;
        let config = validate_config(&arguments.contents)?;
        let target = canonical_target(&self.target)?;
        let path = target.join("dirule.toml");
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err(ToolError::new(
                    "dirule.toml already exists in the selected target; Ask will not overwrite it",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ToolError::new(format!(
                    "could not check the selected target's dirule.toml: {error}"
                )));
            }
        }

        let changes = vec![ToolChange::WriteFile {
            path: path_text(&path),
            contents: arguments.contents.clone(),
        }];
        let proposal = ChangeProposal {
            summary: format!(
                "Create validated dirule.toml in the selected target ({} rule(s))",
                config.rules.len()
            ),
            changes,
        };
        if encoded_size(&json!({
            "summary": &proposal.summary,
            "changes": &proposal.changes,
        }))? > MAX_PREVIEW_RESULT_BYTES
        {
            return Err(ToolError::new(
                "the configuration exceeds the approval display limit; shorten the config and try again",
            ));
        }

        let rule_count = config.rules.len();
        let contents = arguments.contents;
        let action = Box::new(move || {
            create_config(&path, &contents).map_err(|error| ToolError::new(error.to_string()))?;

            Ok(json!({
                "status": "created",
                "path": path_text(&path),
                "rules": rule_count,
            }))
        });

        Ok(PreparedToolCall::Mutation { proposal, action })
    }
}

fn config_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "contents": {
                "type": "string",
                "maxLength": MAX_CONFIG_BYTES,
                "description": "A complete TOML document for dirule.toml. Use target = \".\" unless the user explicitly requested another target."
            }
        },
        "required": ["contents"],
        "additionalProperties": false
    })
}

fn validate_config(contents: &str) -> Result<Config, ToolError> {
    if contents.len() > MAX_CONFIG_BYTES {
        return Err(ToolError::new(format!(
            "configuration exceeds the {MAX_CONFIG_BYTES}-byte limit"
        )));
    }

    parse_config(contents)
        .map_err(|error| ToolError::new(format!("configuration is invalid: {error}")))
}

fn config_result(config: &Config, contents: String) -> Value {
    json!({
        "valid": true,
        "name": config.name.as_deref(),
        "target": config.target.as_deref().map(path_text),
        "rule_count": config.rules.len(),
        "contents": contents,
    })
}

fn canonical_target(target: &Path) -> Result<PathBuf, ToolError> {
    target
        .canonicalize()
        .map_err(|_| ToolError::new("could not access the selected target"))
}

fn ensure_size(value: &Value) -> Result<(), ToolError> {
    if encoded_size(value)? > MAX_PREVIEW_RESULT_BYTES {
        return Err(ToolError::new(
            "the validated configuration exceeds the Ask result limit",
        ));
    }

    Ok(())
}

fn path_text(path: &Path) -> String {
    let path = path.to_string_lossy();

    #[cfg(windows)]
    let path = path
        .strip_prefix(r"\\?\UNC\")
        .map(|unc| format!("//{unc}"))
        .or_else(|| path.strip_prefix(r"\\?\").map(str::to_owned))
        .unwrap_or_else(|| path.into_owned())
        .replace('\\', "/");

    #[cfg(not(windows))]
    let path = path.into_owned();

    path
}
