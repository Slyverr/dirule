use crate::credentials::resolve_api_key;
use crate::{
    Message, ModelProvider, ModelRequest, ModelResponse, ModelToolCall, ProviderError,
    ToolDefinition,
};
use serde_json::{Value, json};
use std::fmt;
use std::time::Duration;
use ureq::Agent;

mod responses;

pub use responses::{OpenAiResponsesConfig, OpenAiResponsesProvider};

pub(super) const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub(super) const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub(super) const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone, Debug)]
pub struct OpenAiCompatibleConfig {
    pub endpoint: String,
    pub model: String,
    pub api_key_env: String,
    pub request_timeout: Duration,
}

impl OpenAiCompatibleConfig {
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

pub struct OpenAiCompatibleProvider {
    agent: Agent,
    endpoint: String,
    model: String,
    api_key: String,
    request_timeout: Duration,
}

impl OpenAiCompatibleProvider {
    pub fn from_config(config: OpenAiCompatibleConfig) -> Result<Self, ProviderError> {
        let endpoint = validate_config(&config)?;
        let api_key = normalize_api_key(resolve_api_key(&config.api_key_env)?)?;

        Self::build(config, endpoint, api_key)
    }

    pub fn with_api_key(
        config: OpenAiCompatibleConfig,
        api_key: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let endpoint = validate_config(&config)?;
        let api_key = normalize_api_key(api_key.into())?;

        Self::build(config, endpoint, api_key)
    }

    fn build(
        config: OpenAiCompatibleConfig,
        endpoint: String,
        api_key: String,
    ) -> Result<Self, ProviderError> {
        let agent = build_agent(config.request_timeout);

        Ok(Self {
            agent,
            endpoint,
            model: config.model,
            api_key,
            request_timeout: config.request_timeout,
        })
    }
}

fn validate_config(config: &OpenAiCompatibleConfig) -> Result<String, ProviderError> {
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

pub(super) fn normalize_api_key(api_key: String) -> Result<String, ProviderError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(ProviderError::new("API key is empty"));
    }

    Ok(api_key.to_owned())
}

impl fmt::Debug for OpenAiCompatibleProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiCompatibleProvider")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

impl ModelProvider for OpenAiCompatibleProvider {
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
        let payload = chat_request(model, &request.messages, &request.tools)?;
        let response = post_json(
            &self.agent,
            &self.endpoint,
            &self.api_key,
            &payload,
            timeout,
        )?;

        parse_chat_response(&response)
    }
}

pub(super) fn build_agent(timeout: Duration) -> Agent {
    let config = Agent::config_builder()
        .max_redirects(0)
        .timeout_global(Some(timeout))
        .build();

    Agent::new_with_config(config)
}

pub(super) fn post_json(
    agent: &Agent,
    endpoint: &str,
    api_key: &str,
    payload: &Value,
    timeout: Duration,
) -> Result<Value, ProviderError> {
    let body = serde_json::to_vec(payload)
        .map_err(|_| ProviderError::new("could not encode the model request"))?;
    if body.len() > MAX_REQUEST_BYTES {
        return Err(ProviderError::new(format!(
            "model request exceeds the {MAX_REQUEST_BYTES}-byte limit"
        )));
    }

    let mut response = agent
        .post(endpoint.to_owned())
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .config()
        .timeout_global(Some(timeout))
        .build()
        .send(body)
        .map_err(provider_request_error)?;
    if response.status().is_redirection() {
        return Err(ProviderError::new(
            "model provider redirected the request; configure its final endpoint",
        ));
    }

    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES as u64)
        .read_to_vec()
        .map_err(|_| ProviderError::new("could not read provider response"))?;

    serde_json::from_slice(&body)
        .map_err(|_| ProviderError::new("provider returned an invalid JSON response"))
}

pub(super) fn validate_endpoint(endpoint: &str) -> Result<String, ProviderError> {
    let (scheme, rest) = endpoint
        .split_once("://")
        .ok_or_else(|| ProviderError::new("provider endpoint must be an absolute URL"))?;
    let authority = rest.split('/').next().unwrap_or_default();
    let normalized_authority = authority.to_ascii_lowercase();
    let local_http = scheme == "http"
        && ["localhost", "127.0.0.1", "[::1]"].iter().any(|host| {
            normalized_authority == *host
                || normalized_authority
                    .strip_prefix(host)
                    .is_some_and(|suffix| suffix.starts_with(':'))
        });
    if scheme != "https" && !local_http {
        return Err(ProviderError::new(
            "provider endpoint must use HTTPS (HTTP is allowed only for localhost tests)",
        ));
    }
    if authority.is_empty()
        || authority.contains('@')
        || endpoint.contains('?')
        || endpoint.contains('#')
        || endpoint.contains(char::is_whitespace)
    {
        return Err(ProviderError::new(
            "provider endpoint must not contain credentials, query parameters, or a fragment",
        ));
    }

    Ok(endpoint.to_owned())
}

fn chat_request(
    model: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
) -> Result<Value, ProviderError> {
    let messages = messages
        .iter()
        .map(chat_message)
        .collect::<Result<Vec<_>, _>>()?;
    let mut request = json!({
        "model": model,
        "messages": messages,
    });

    if !tools.is_empty() {
        request["tools"] = Value::Array(tools.iter().map(chat_tool).collect());
        request["tool_choice"] = Value::String("auto".into());
    }

    Ok(request)
}

fn chat_message(message: &Message) -> Result<Value, ProviderError> {
    match message {
        Message::System { content } => Ok(json!({ "role": "system", "content": content })),
        Message::User { content } => Ok(json!({ "role": "user", "content": content })),
        Message::Assistant {
            content,
            tool_calls,
        } => {
            let calls = tool_calls
                .iter()
                .map(|call| {
                    let arguments = serde_json::to_string(&call.arguments)
                        .map_err(|_| ProviderError::new("could not encode tool arguments"))?;
                    Ok(json!({
                        "id": call.id,
                        "type": "function",
                        "function": { "name": call.name, "arguments": arguments }
                    }))
                })
                .collect::<Result<Vec<_>, ProviderError>>()?;
            Ok(json!({
                "role": "assistant",
                "content": content,
                "tool_calls": calls,
            }))
        }
        Message::Tool {
            tool_call_id,
            name,
            content,
        } => Ok(json!({
            "role": "tool",
            "tool_call_id": tool_call_id,
            "name": name,
            "content": content,
        })),
    }
}

fn chat_tool(tool: &ToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

pub(super) fn provider_request_error(error: ureq::Error) -> ProviderError {
    match error {
        ureq::Error::StatusCode(status) => {
            ProviderError::new(format!("model provider returned HTTP status {status}"))
        }
        _ => ProviderError::new("request to the configured model provider failed"),
    }
}

fn parse_chat_response(response: &Value) -> Result<ModelResponse, ProviderError> {
    let message = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .ok_or_else(|| ProviderError::new("provider response is missing choices[0].message"))?;

    let content = match message.get("content") {
        None | Some(Value::Null) => None,
        Some(Value::String(content)) => Some(content.clone()),
        Some(_) => {
            return Err(ProviderError::new(
                "provider response message content must be a string or null",
            ));
        }
    };
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| calls.iter().map(parse_tool_call).collect())
        .transpose()?
        .unwrap_or_default();

    Ok(ModelResponse {
        content,
        tool_calls,
    })
}

fn parse_tool_call(call: &Value) -> Result<ModelToolCall, ProviderError> {
    let id = call
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ProviderError::new("provider tool call is missing its id"))?;
    let function = call
        .get("function")
        .ok_or_else(|| ProviderError::new("provider tool call is missing its function"))?;
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ProviderError::new("provider tool call is missing its function name"))?;
    let arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::new("provider tool call arguments must be a JSON string"))?;
    let arguments: Value = serde_json::from_str(arguments)
        .map_err(|_| ProviderError::new("provider tool call arguments contain invalid JSON"))?;
    if !arguments.is_object() {
        return Err(ProviderError::new(
            "provider tool call arguments must be a JSON object",
        ));
    }

    Ok(ModelToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments,
    })
}
