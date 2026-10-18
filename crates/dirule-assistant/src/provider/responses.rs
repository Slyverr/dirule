use super::{
    DEFAULT_REQUEST_TIMEOUT, build_agent, normalize_api_key, post_json, validate_endpoint,
};
use crate::credentials::resolve_api_key;
use crate::{
    Message, ModelProvider, ModelRequest, ModelResponse, ModelToolCall, ProviderError,
    ToolDefinition,
};
use serde_json::{Value, json};
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;
use ureq::Agent;

#[derive(Clone, Debug)]
pub struct OpenAiResponsesConfig {
    pub endpoint: String,
    pub model: String,
    pub api_key_env: String,
    pub request_timeout: Duration,
}

impl OpenAiResponsesConfig {
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key_env: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            api_key_env: api_key_env.into(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }
}

pub struct OpenAiResponsesProvider {
    agent: Agent,
    endpoint: String,
    model: String,
    api_key: String,
    request_timeout: Duration,
    conversation: Mutex<ResponsesConversation>,
}

#[derive(Default)]
struct ResponsesConversation {
    input: Vec<Value>,
    messages: Vec<Message>,
}

impl OpenAiResponsesProvider {
    pub fn from_config(config: OpenAiResponsesConfig) -> Result<Self, ProviderError> {
        let endpoint = validate_config(&config)?;
        let api_key = normalize_api_key(resolve_api_key(&config.api_key_env)?)?;

        Self::build(config, endpoint, api_key)
    }

    pub fn with_api_key(
        config: OpenAiResponsesConfig,
        api_key: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let endpoint = validate_config(&config)?;
        let api_key = normalize_api_key(api_key.into())?;

        Self::build(config, endpoint, api_key)
    }

    fn build(
        config: OpenAiResponsesConfig,
        endpoint: String,
        api_key: String,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            agent: build_agent(config.request_timeout),
            endpoint,
            model: config.model,
            api_key,
            request_timeout: config.request_timeout,
            conversation: Mutex::new(ResponsesConversation::default()),
        })
    }
}

fn validate_config(config: &OpenAiResponsesConfig) -> Result<String, ProviderError> {
    let endpoint = validate_endpoint(&config.endpoint)?;
    if config.model.trim().is_empty() {
        return Err(ProviderError::new("model name must not be empty"));
    }
    if config.request_timeout.is_zero() {
        return Err(ProviderError::new(
            "request timeout must be greater than zero",
        ));
    }

    Ok(endpoint)
}

impl fmt::Debug for OpenAiResponsesProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiResponsesProvider")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

impl ModelProvider for OpenAiResponsesProvider {
    fn complete_with_timeout(
        &self,
        request: &ModelRequest,
        timeout: Duration,
    ) -> Result<ModelResponse, ProviderError> {
        let timeout = timeout.min(self.request_timeout);
        if timeout.is_zero() {
            return Err(ProviderError::new(
                "assistant request time limit was exhausted before contacting the provider",
            ));
        }

        let model = if request.model.trim().is_empty() {
            &self.model
        } else {
            &request.model
        };
        let mut conversation = self
            .conversation
            .lock()
            .map_err(|_| ProviderError::new("Responses conversation state is unavailable"))?;
        if !request.messages.starts_with(&conversation.messages)
            || (!conversation.messages.is_empty()
                && request.messages.len() == conversation.messages.len())
        {
            *conversation = ResponsesConversation::default();
        }

        let mut input = conversation.input.clone();
        let prior_response_was_added = !conversation.messages.is_empty();
        for message in &request.messages[conversation.messages.len()..] {
            append_message(&mut input, message, prior_response_was_added)?;
        }

        let payload = responses_request(model, &input, &request.tools);
        let response = post_json(
            &self.agent,
            &self.endpoint,
            &self.api_key,
            &payload,
            timeout,
        )?;
        let (response, output) = parse_response(&response)?;

        input.extend(output);
        conversation.input = input;
        conversation.messages = request.messages.clone();

        Ok(response)
    }
}

fn append_message(
    input: &mut Vec<Value>,
    message: &Message,
    prior_response_was_added: bool,
) -> Result<(), ProviderError> {
    match message {
        Message::System { content } => input.push(json!({ "role": "system", "content": content })),
        Message::User { content } => input.push(json!({ "role": "user", "content": content })),
        Message::Assistant { .. } if prior_response_was_added => {}
        Message::Assistant {
            content,
            tool_calls,
        } => {
            if let Some(content) = content {
                input.push(json!({ "role": "assistant", "content": content }));
            }
            for call in tool_calls {
                let arguments = serde_json::to_string(&call.arguments)
                    .map_err(|_| ProviderError::new("could not encode tool arguments"))?;
                input.push(json!({
                    "type": "function_call",
                    "call_id": call.id,
                    "name": call.name,
                    "arguments": arguments,
                }));
            }
        }
        Message::Tool {
            tool_call_id,
            content,
            ..
        } => input.push(json!({
            "type": "function_call_output",
            "call_id": tool_call_id,
            "output": content,
        })),
    }

    Ok(())
}

fn responses_request(model: &str, input: &[Value], tools: &[ToolDefinition]) -> Value {
    let mut request = json!({
        "model": model,
        "input": input,
        "store": false,
    });
    if !tools.is_empty() {
        request["tools"] = Value::Array(tools.iter().map(responses_tool).collect());
        request["tool_choice"] = Value::String("auto".into());
    }

    request
}

fn responses_tool(tool: &ToolDefinition) -> Value {
    json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
        "strict": false,
    })
}

fn parse_response(response: &Value) -> Result<(ModelResponse, Vec<Value>), ProviderError> {
    match response.get("status").and_then(Value::as_str) {
        Some("incomplete") => {
            return Err(ProviderError::new(
                "provider returned an incomplete response",
            ));
        }
        Some("failed") => return Err(ProviderError::new("provider returned a failed response")),
        _ => {}
    }

    let output = response
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::new("provider response is missing its output array"))?;
    let mut content = Vec::new();
    let mut tool_calls = Vec::new();
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => append_output_text(item, &mut content),
            Some("function_call") => tool_calls.push(parse_tool_call(item)?),
            _ => {}
        }
    }

    let content = (!content.is_empty()).then(|| content.join("\n"));
    if content.is_none() && tool_calls.is_empty() {
        return Err(ProviderError::new(
            "provider response contained no assistant text or function calls",
        ));
    }

    Ok((
        ModelResponse {
            content,
            tool_calls,
        },
        output.clone(),
    ))
}

fn append_output_text(item: &Value, content: &mut Vec<String>) {
    let Some(parts) = item.get("content").and_then(Value::as_array) else {
        return;
    };

    for part in parts {
        match part.get("type").and_then(Value::as_str) {
            Some("output_text") => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    content.push(text.to_owned());
                }
            }
            Some("refusal") => {
                if let Some(text) = part.get("refusal").and_then(Value::as_str) {
                    content.push(text.to_owned());
                }
            }
            _ => {}
        }
    }
}

fn parse_tool_call(call: &Value) -> Result<ModelToolCall, ProviderError> {
    let id = call
        .get("call_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ProviderError::new("provider function call is missing its call ID"))?;
    let name = call
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ProviderError::new("provider function call is missing its name"))?;
    let arguments = call
        .get("arguments")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::new("provider function call arguments must be a string"))?;
    let arguments: Value = serde_json::from_str(arguments)
        .map_err(|_| ProviderError::new("provider function call arguments contain invalid JSON"))?;
    if !arguments.is_object() {
        return Err(ProviderError::new(
            "provider function call arguments must be a JSON object",
        ));
    }

    Ok(ModelToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments,
    })
}
