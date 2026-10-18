use crate::{ProviderError, ToolError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequirement {
    None,
    Required,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub approval: ApprovalRequirement,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolChange {
    Move { source: String, destination: String },
    CreateDirectory { path: String },
    RemoveDirectoryIfEmpty { path: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChangeProposal {
    pub summary: String,
    pub changes: Vec<ToolChange>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        #[serde(default)]
        tool_calls: Vec<ModelToolCall>,
    },
    Tool {
        tool_call_id: String,
        name: String,
        content: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ModelRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ModelResponse {
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ModelToolCall>,
}

pub trait ModelProvider: Send + Sync {
    /// Complete one model request, returning no later than the supplied timeout.
    fn complete_with_timeout(
        &self,
        request: &ModelRequest,
        timeout: Duration,
    ) -> Result<ModelResponse, ProviderError>;
}

pub(crate) fn encode_tool_result(result: &Value, maximum: usize) -> Result<String, ToolError> {
    let encoded = serde_json::to_string(result)
        .map_err(|error| ToolError::new(format!("could not encode tool result: {error}")))?;
    if encoded.len() > maximum {
        return Err(ToolError::new(format!(
            "tool result exceeds the {maximum}-byte limit"
        )));
    }

    Ok(encoded)
}
