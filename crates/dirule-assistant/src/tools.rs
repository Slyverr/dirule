use crate::{ChangeProposal, ToolDefinition, ToolError};
use serde_json::Value;
use std::collections::BTreeMap;

pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError>;
}

pub enum PreparedToolCall {
    ReadOnly(Value),
    Mutation {
        proposal: ChangeProposal,
        action: Box<dyn FnOnce() -> Result<Value, ToolError> + Send>,
    },
}

pub trait ApprovalHandler {
    fn approve(&mut self, proposal: &ChangeProposal) -> Result<bool, ToolError>;
}

pub struct ToolCatalog {
    tools: BTreeMap<String, Box<dyn Tool>>,
    definitions: Vec<ToolDefinition>,
}

impl ToolCatalog {
    pub fn new(tools: impl IntoIterator<Item = Box<dyn Tool>>) -> Result<Self, ToolError> {
        let mut registered = BTreeMap::new();
        let mut definitions = Vec::new();

        for tool in tools {
            let definition = tool.definition();
            validate_definition(&definition)?;
            if registered.insert(definition.name.clone(), tool).is_some() {
                return Err(ToolError::new(format!(
                    "tool {:?} is registered more than once",
                    definition.name
                )));
            }

            definitions.push(definition);
        }

        definitions.sort_by(|left, right| left.name.cmp(&right.name));

        Ok(Self {
            tools: registered,
            definitions,
        })
    }

    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    pub(crate) fn prepare(
        &self,
        name: &str,
        arguments: Value,
    ) -> Result<PreparedToolCall, ToolError> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| ToolError::new(format!("unknown Dirule tool {name:?}")))?;
        if !arguments.is_object() {
            return Err(ToolError::new("tool arguments must be a JSON object"));
        }
        let definition = tool.definition();
        let prepared = tool.prepare(arguments)?;

        match (&definition.approval, &prepared) {
            (crate::ApprovalRequirement::Required, PreparedToolCall::ReadOnly(_)) => {
                Err(ToolError::new(format!(
                    "tool {name:?} requires approval but prepared no mutation"
                )))
            }
            (crate::ApprovalRequirement::None, PreparedToolCall::Mutation { .. }) => Err(
                ToolError::new(format!("tool {name:?} attempted an undeclared mutation")),
            ),
            _ => Ok(prepared),
        }
    }
}

fn validate_definition(definition: &ToolDefinition) -> Result<(), ToolError> {
    if definition.name == crate::assistant::END_SESSION_TOOL {
        return Err(ToolError::new(format!(
            "tool name {:?} is reserved for assistant session control",
            definition.name
        )));
    }

    if definition.name.trim().is_empty() {
        return Err(ToolError::new("tool names must not be empty"));
    }
    if !definition
        .name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(ToolError::new(format!(
            "tool name {:?} must contain only ASCII letters, digits, and underscores",
            definition.name
        )));
    }
    if definition.description.trim().is_empty() {
        return Err(ToolError::new(format!(
            "tool {:?} must have a description",
            definition.name
        )));
    }
    if !definition.parameters.is_object() {
        return Err(ToolError::new(format!(
            "tool {:?} parameters must be a JSON object schema",
            definition.name
        )));
    }

    Ok(())
}
