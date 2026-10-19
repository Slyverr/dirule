use crate::{
    ApprovalHandler, AssistantError, AssistantEvent, AssistantObserver, Message, ModelProvider,
    ModelRequest, ToolCatalog, ToolError, protocol::encode_tool_result, tools::PreparedToolCall,
};
use serde_json::json;
use std::time::{Duration, Instant};

pub(crate) const END_SESSION_TOOL: &str = "end_session";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AssistantLimits {
    pub maximum_prompt_bytes: usize,
    pub maximum_conversation_bytes: usize,
    pub maximum_turns: usize,
    pub maximum_tool_calls: usize,
    pub maximum_tool_result_bytes: usize,
    pub maximum_elapsed: Duration,
}

impl Default for AssistantLimits {
    fn default() -> Self {
        Self {
            maximum_prompt_bytes: Self::DEFAULT_MAXIMUM_PROMPT_BYTES,
            maximum_conversation_bytes: 256 * 1024,
            maximum_turns: 8,
            maximum_tool_calls: 16,
            maximum_tool_result_bytes: 32 * 1024,
            maximum_elapsed: Duration::from_secs(180),
        }
    }
}

impl AssistantLimits {
    pub const DEFAULT_MAXIMUM_PROMPT_BYTES: usize = 32 * 1024;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantResult {
    pub response: String,
    pub tool_calls: usize,
    pub approvals_granted: usize,
    pub session_ended: bool,
}

pub struct AssistantConversation {
    instructions: String,
    messages: Vec<Message>,
    maximum_bytes: usize,
    allow_session_end: bool,
}

impl AssistantConversation {
    pub fn reset(&mut self) {
        self.messages.clear();
        self.messages.push(Message::System {
            content: self.instructions.clone(),
        });
    }
}

struct SilentObserver;

impl AssistantObserver for SilentObserver {
    fn on_event(&mut self, _event: AssistantEvent) {}
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
        self.run_with_observer(model, instructions, prompt, approvals, &mut SilentObserver)
    }

    pub fn run_with_observer(
        &self,
        model: &str,
        instructions: &str,
        prompt: &str,
        approvals: &mut dyn ApprovalHandler,
        observer: &mut dyn AssistantObserver,
    ) -> Result<AssistantResult, AssistantError> {
        let mut conversation = self.conversation(instructions);

        self.run_conversation_turn_with_observer(
            model,
            &mut conversation,
            prompt,
            approvals,
            observer,
        )
    }

    pub fn conversation(&self, instructions: &str) -> AssistantConversation {
        self.new_conversation(instructions, false)
    }

    pub fn conversation_with_session_end(&self, instructions: &str) -> AssistantConversation {
        self.new_conversation(instructions, true)
    }

    fn new_conversation(
        &self,
        instructions: &str,
        allow_session_end: bool,
    ) -> AssistantConversation {
        AssistantConversation {
            instructions: instructions.to_owned(),
            messages: vec![Message::System {
                content: instructions.to_owned(),
            }],
            maximum_bytes: self.limits.maximum_conversation_bytes,
            allow_session_end,
        }
    }

    pub fn run_conversation_turn(
        &self,
        model: &str,
        conversation: &mut AssistantConversation,
        prompt: &str,
        approvals: &mut dyn ApprovalHandler,
    ) -> Result<AssistantResult, AssistantError> {
        self.run_conversation_turn_with_observer(
            model,
            conversation,
            prompt,
            approvals,
            &mut SilentObserver,
        )
    }

    pub fn run_conversation_turn_with_observer(
        &self,
        model: &str,
        conversation: &mut AssistantConversation,
        prompt: &str,
        approvals: &mut dyn ApprovalHandler,
        observer: &mut dyn AssistantObserver,
    ) -> Result<AssistantResult, AssistantError> {
        if prompt.len() > self.limits.maximum_prompt_bytes {
            return Err(AssistantError::PromptTooLarge {
                limit: self.limits.maximum_prompt_bytes,
            });
        }

        let previous_messages = conversation.messages.len();
        conversation.messages.push(Message::User {
            content: prompt.to_owned(),
        });
        if let Err(error) = conversation.ensure_size() {
            conversation.messages.truncate(previous_messages);
            return Err(error);
        }

        let mut tool_calls = 0;
        let mut approvals_granted = 0;
        let started = Instant::now();

        for turn in 1..=self.limits.maximum_turns {
            let remaining = self.remaining_time(started)?;
            let request = ModelRequest {
                model: model.to_owned(),
                messages: conversation.messages.clone(),
                tools: self.request_tools(conversation.allow_session_end),
            };
            observer.on_event(AssistantEvent::ModelRequestStarted {
                turn,
                turn_limit: self.limits.maximum_turns,
            });
            let response = self.provider.complete_with_timeout(&request, remaining);
            observer.on_event(AssistantEvent::ModelRequestFinished {
                turn,
                succeeded: response.is_ok(),
            });
            let response = response?;
            self.remaining_time(started)?;
            if response.tool_calls.is_empty() {
                conversation.messages.push(Message::Assistant {
                    content: response.content.clone(),
                    tool_calls: Vec::new(),
                });
                conversation.ensure_size()?;

                return Ok(AssistantResult {
                    response: response.content.unwrap_or_default(),
                    tool_calls,
                    approvals_granted,
                    session_ended: false,
                });
            }

            tool_calls = tool_calls.saturating_add(response.tool_calls.len());
            if tool_calls > self.limits.maximum_tool_calls {
                return Err(AssistantError::LimitExceeded {
                    limit: "tool call",
                    maximum: self.limits.maximum_tool_calls,
                });
            }

            if conversation.allow_session_end
                && response
                    .tool_calls
                    .iter()
                    .any(|call| call.name == END_SESSION_TOOL)
            {
                conversation.messages.push(Message::Assistant {
                    content: response.content.clone(),
                    tool_calls: response.tool_calls.clone(),
                });

                let has_no_arguments = response.tool_calls.len() == 1
                    && response.tool_calls[0]
                        .arguments
                        .as_object()
                        .is_some_and(serde_json::Map::is_empty);
                if !has_no_arguments {
                    for call in response.tool_calls {
                        conversation.messages.push(Message::Tool {
                            tool_call_id: call.id,
                            name: call.name,
                            content: error_result(ToolError::new(
                                "end_session must be the only tool call and take no arguments; no tools were executed",
                            )),
                        });
                    }
                    conversation.ensure_size()?;

                    continue;
                }

                let call = response
                    .tool_calls
                    .into_iter()
                    .next()
                    .expect("one end call");
                observer.on_event(AssistantEvent::ToolCallStarted {
                    name: call.name.clone(),
                });
                conversation.messages.push(Message::Tool {
                    tool_call_id: call.id,
                    name: call.name.clone(),
                    content: "{\"status\":\"session_ended\"}".into(),
                });
                observer.on_event(AssistantEvent::ToolCallFinished {
                    name: call.name,
                    succeeded: true,
                });
                conversation.ensure_size()?;

                return Ok(AssistantResult {
                    response: response.content.unwrap_or_default(),
                    tool_calls,
                    approvals_granted,
                    session_ended: true,
                });
            }

            conversation.messages.push(Message::Assistant {
                content: response.content,
                tool_calls: response.tool_calls.clone(),
            });

            for call in response.tool_calls {
                self.remaining_time(started)?;
                observer.on_event(AssistantEvent::ToolCallStarted {
                    name: call.name.clone(),
                });
                let result = self.dispatch(
                    &call.name,
                    call.arguments,
                    approvals,
                    &mut approvals_granted,
                    observer,
                );
                let (content, succeeded) = match result {
                    Ok(value) => {
                        match encode_tool_result(&value, self.limits.maximum_tool_result_bytes) {
                            Ok(content) => (content, true),
                            Err(error) => (error_result(error), false),
                        }
                    }
                    Err(error) => (error_result(error), false),
                };
                observer.on_event(AssistantEvent::ToolCallFinished {
                    name: call.name.clone(),
                    succeeded,
                });
                self.remaining_time(started)?;
                conversation.messages.push(Message::Tool {
                    tool_call_id: call.id,
                    name: call.name,
                    content,
                });
            }

            conversation.ensure_size()?;
        }

        Err(AssistantError::MissingFinalResponse)
    }

    fn request_tools(&self, allow_session_end: bool) -> Vec<crate::ToolDefinition> {
        let mut tools = self.tools.definitions().to_vec();
        if allow_session_end {
            tools.push(crate::ToolDefinition {
                name: END_SESSION_TOOL.into(),
                description: "End the interactive Dirule Ask session when the user clearly asks to leave the conversation. Do not use this to stop or cancel a filesystem task.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                approval: crate::ApprovalRequirement::None,
            });
            tools.sort_by(|left, right| left.name.cmp(&right.name));
        }

        tools
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
        observer: &mut dyn AssistantObserver,
    ) -> Result<serde_json::Value, ToolError> {
        match self.tools.prepare(name, arguments)? {
            PreparedToolCall::ReadOnly(result) => Ok(result),
            PreparedToolCall::Mutation { proposal, action } => {
                observer.on_event(AssistantEvent::ApprovalRequired {
                    name: name.to_owned(),
                });
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

impl AssistantConversation {
    fn ensure_size(&self) -> Result<(), AssistantError> {
        let size = serde_json::to_vec(&self.messages).map_or(usize::MAX, |messages| messages.len());
        if size > self.maximum_bytes {
            return Err(AssistantError::ConversationTooLarge {
                limit: self.maximum_bytes,
            });
        }

        Ok(())
    }
}

fn error_result(error: ToolError) -> String {
    serde_json::to_string(&json!({ "error": error.to_string() }))
        .unwrap_or_else(|_| "{\"error\":\"tool failed\"}".to_owned())
}
