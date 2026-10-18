use crate::{
    ApprovalHandler, AssistantError, Message, ModelProvider, ModelRequest, ToolCatalog, ToolError,
    protocol::encode_tool_result, tools::PreparedToolCall,
};
use serde_json::json;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AssistantLimits {
    pub maximum_prompt_bytes: usize,
    pub maximum_turns: usize,
    pub maximum_tool_calls: usize,
    pub maximum_tool_result_bytes: usize,
    pub maximum_elapsed: Duration,
}

impl Default for AssistantLimits {
    fn default() -> Self {
        Self {
            maximum_prompt_bytes: 32 * 1024,
            maximum_turns: 8,
            maximum_tool_calls: 16,
            maximum_tool_result_bytes: 32 * 1024,
            maximum_elapsed: Duration::from_secs(180),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantResult {
    pub response: String,
    pub tool_calls: usize,
    pub approvals_granted: usize,
}

pub struct Assistant<'a> {
    provider: &'a dyn ModelProvider,
    tools: &'a ToolCatalog,
    limits: AssistantLimits,
}

impl<'a> Assistant<'a> {
    pub fn new(
        provider: &'a dyn ModelProvider,
        tools: &'a ToolCatalog,
        limits: AssistantLimits,
    ) -> Self {
        Self {
            provider,
            tools,
            limits,
        }
    }

    pub fn run(
        &self,
        model: &str,
        instructions: &str,
        prompt: &str,
        approvals: &mut dyn ApprovalHandler,
    ) -> Result<AssistantResult, AssistantError> {
        if prompt.len() > self.limits.maximum_prompt_bytes {
            return Err(AssistantError::PromptTooLarge {
                limit: self.limits.maximum_prompt_bytes,
            });
        }

        let mut messages = vec![
            Message::System {
                content: instructions.to_owned(),
            },
            Message::User {
                content: prompt.to_owned(),
            },
        ];
        let mut tool_calls = 0;
        let mut approvals_granted = 0;
        let started = Instant::now();

        for _ in 0..self.limits.maximum_turns {
            let remaining = self.remaining_time(started)?;
            let request = ModelRequest {
                model: model.to_owned(),
                messages: messages.clone(),
                tools: self.tools.definitions().to_vec(),
            };
            let response = self.provider.complete_with_timeout(&request, remaining)?;
            self.remaining_time(started)?;
            if response.tool_calls.is_empty() {
                return Ok(AssistantResult {
                    response: response.content.unwrap_or_default(),
                    tool_calls,
                    approvals_granted,
                });
            }

            tool_calls = tool_calls.saturating_add(response.tool_calls.len());
            if tool_calls > self.limits.maximum_tool_calls {
                return Err(AssistantError::LimitExceeded {
                    limit: "tool call",
                    maximum: self.limits.maximum_tool_calls,
                });
            }

            messages.push(Message::Assistant {
                content: response.content,
                tool_calls: response.tool_calls.clone(),
            });

            for call in response.tool_calls {
                self.remaining_time(started)?;
                let result = self.dispatch(
                    &call.name,
                    call.arguments,
                    approvals,
                    &mut approvals_granted,
                );
                self.remaining_time(started)?;
                let content = match result {
                    Ok(value) => encode_tool_result(&value, self.limits.maximum_tool_result_bytes)
                        .unwrap_or_else(error_result),
                    Err(error) => error_result(error),
                };
                messages.push(Message::Tool {
                    tool_call_id: call.id,
                    name: call.name,
                    content,
                });
            }
        }

        Err(AssistantError::MissingFinalResponse)
    }

    fn remaining_time(&self, started: Instant) -> Result<Duration, AssistantError> {
        self.limits
            .maximum_elapsed
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(AssistantError::TimeLimitExceeded {
                maximum: self.limits.maximum_elapsed,
            })
    }

    fn dispatch(
        &self,
        name: &str,
        arguments: serde_json::Value,
        approvals: &mut dyn ApprovalHandler,
        approvals_granted: &mut usize,
    ) -> Result<serde_json::Value, ToolError> {
        match self.tools.prepare(name, arguments)? {
            PreparedToolCall::ReadOnly(result) => Ok(result),
            PreparedToolCall::Mutation { proposal, action } => {
                if !approvals.approve(&proposal)? {
                    return Ok(json!({
                        "status": "declined",
                        "summary": proposal.summary,
                        "changes": proposal.changes,
                    }));
                }

                *approvals_granted = approvals_granted.saturating_add(1);
                action()
            }
        }
    }
}

fn error_result(error: ToolError) -> String {
    serde_json::to_string(&json!({ "error": error.to_string() }))
        .unwrap_or_else(|_| "{\"error\":\"tool failed\"}".to_owned())
}
