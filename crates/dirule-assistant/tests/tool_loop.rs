use dirule_assistant::{
    ApprovalHandler, ApprovalRequirement, Assistant, AssistantLimits, ChangeProposal, Message,
    ModelProvider, ModelRequest, ModelResponse, ModelToolCall, PreparedToolCall, ProviderError,
    Tool, ToolCatalog, ToolChange, ToolDefinition, ToolError,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct FakeProvider {
    responses: Mutex<VecDeque<ModelResponse>>,
    requests: Mutex<Vec<ModelRequest>>,
}

struct SlowProvider;

impl ModelProvider for SlowProvider {
    fn complete_with_timeout(
        &self,
        _request: &ModelRequest,
        _timeout: Duration,
    ) -> Result<ModelResponse, ProviderError> {
        std::thread::sleep(Duration::from_millis(20));

        Ok(response_text("late response"))
    }
}

impl FakeProvider {
    fn new(responses: impl IntoIterator<Item = ModelResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

impl ModelProvider for FakeProvider {
    fn complete_with_timeout(
        &self,
        request: &ModelRequest,
        _timeout: Duration,
    ) -> Result<ModelResponse, ProviderError> {
        self.requests
            .lock()
            .expect("requests lock")
            .push(request.clone());
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .ok_or_else(|| ProviderError::new("no fake response remains"))
    }
}

struct FixedTool {
    approval: ApprovalRequirement,
    called: Arc<Mutex<bool>>,
}

impl Tool for FixedTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "inspect_target".into(),
            description: "Return a small summary of the target.".into(),
            parameters: json!({"type": "object", "properties": {}, "additionalProperties": false}),
            approval: self.approval,
        }
    }

    fn prepare(&self, _arguments: Value) -> Result<PreparedToolCall, ToolError> {
        match self.approval {
            ApprovalRequirement::None => Ok(PreparedToolCall::ReadOnly(json!({
                "entries": ["one.txt", "two.pdf"]
            }))),
            ApprovalRequirement::Required => {
                let called = Arc::clone(&self.called);
                Ok(PreparedToolCall::Mutation {
                    proposal: ChangeProposal {
                        summary: "Rename one file".into(),
                        changes: vec![ToolChange::Move {
                            source: "One.PDF".into(),
                            destination: "one.pdf".into(),
                        }],
                    },
                    action: Box::new(move || {
                        *called.lock().expect("called lock") = true;
                        Ok(json!({"status": "applied"}))
                    }),
                })
            }
        }
    }
}

struct FixedApproval(bool);

impl ApprovalHandler for FixedApproval {
    fn approve(&mut self, _proposal: &ChangeProposal) -> Result<bool, ToolError> {
        Ok(self.0)
    }
}

#[derive(Default)]
struct CountingApproval {
    calls: usize,
}

impl ApprovalHandler for CountingApproval {
    fn approve(&mut self, _proposal: &ChangeProposal) -> Result<bool, ToolError> {
        self.calls += 1;

        Ok(true)
    }
}

fn tool_call(name: &str, arguments: Value) -> ModelToolCall {
    ModelToolCall {
        id: "call-1".into(),
        name: name.into(),
        arguments,
    }
}

fn response_with_tool(call: ModelToolCall) -> ModelResponse {
    ModelResponse {
        content: None,
        tool_calls: vec![call],
    }
}

fn response_text(text: &str) -> ModelResponse {
    ModelResponse {
        content: Some(text.into()),
        tool_calls: Vec::new(),
    }
}

fn catalog(approval: ApprovalRequirement, called: Arc<Mutex<bool>>) -> ToolCatalog {
    ToolCatalog::new([Box::new(FixedTool { approval, called }) as Box<dyn Tool>])
        .expect("valid tool catalog")
}

#[test]
fn routes_model_tool_calls_to_registered_tools_and_returns_final_response() {
    let provider = FakeProvider::new([
        response_with_tool(tool_call("inspect_target", json!({}))),
        response_text("There are two files."),
    ]);
    let called = Arc::new(Mutex::new(false));
    let tools = catalog(ApprovalRequirement::None, called);
    let assistant = Assistant::new(&provider, &tools, AssistantLimits::default());

    let result = assistant
        .run(
            "test-model",
            "Use Dirule tools.",
            "What is here?",
            &mut FixedApproval(false),
        )
        .expect("assistant completes");

    assert_eq!(result.response, "There are two files.");
    assert_eq!(result.tool_calls, 1);
    assert_eq!(result.approvals_granted, 0);

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].tools[0].name, "inspect_target");
    assert!(
        matches!(requests[1].messages.last(), Some(Message::Tool { content, .. }) if content.contains("one.txt"))
    );
}

#[test]
fn denied_mutation_does_not_run_its_action() {
    let provider = FakeProvider::new([
        response_with_tool(tool_call("inspect_target", json!({}))),
        response_text("I left the files unchanged."),
    ]);
    let called = Arc::new(Mutex::new(false));
    let tools = catalog(ApprovalRequirement::Required, Arc::clone(&called));
    let assistant = Assistant::new(&provider, &tools, AssistantLimits::default());

    let result = assistant
        .run(
            "test-model",
            "Use Dirule tools.",
            "Rename this file.",
            &mut FixedApproval(false),
        )
        .expect("assistant completes after decline");

    assert_eq!(result.approvals_granted, 0);
    assert!(!*called.lock().expect("called lock"));
    assert!(
        matches!(provider.requests()[1].messages.last(), Some(Message::Tool { content, .. }) if content.contains("declined"))
    );
}

#[test]
fn approved_mutation_runs_only_after_approval_handler_accepts_proposal() {
    let provider = FakeProvider::new([
        response_with_tool(tool_call("inspect_target", json!({}))),
        response_text("Done."),
    ]);
    let called = Arc::new(Mutex::new(false));
    let tools = catalog(ApprovalRequirement::Required, Arc::clone(&called));
    let assistant = Assistant::new(&provider, &tools, AssistantLimits::default());

    let result = assistant
        .run(
            "test-model",
            "Use Dirule tools.",
            "Rename this file.",
            &mut FixedApproval(true),
        )
        .expect("assistant completes after approval");

    assert_eq!(result.approvals_granted, 1);
    assert!(*called.lock().expect("called lock"));
}

#[test]
fn unregistered_tool_is_reported_to_model_without_dispatching() {
    let provider = FakeProvider::new([
        response_with_tool(tool_call("run_shell", json!({"command": "rm -rf /"}))),
        response_text("I can only use Dirule tools."),
    ]);
    let called = Arc::new(Mutex::new(false));
    let tools = catalog(ApprovalRequirement::None, Arc::clone(&called));
    let assistant = Assistant::new(&provider, &tools, AssistantLimits::default());

    let result = assistant
        .run(
            "test-model",
            "Use Dirule tools.",
            "Organize this folder.",
            &mut FixedApproval(false),
        )
        .expect("assistant recovers from an unsupported tool call");

    assert_eq!(result.response, "I can only use Dirule tools.");
    assert!(!*called.lock().expect("called lock"));
    assert!(
        matches!(provider.requests()[1].messages.last(), Some(Message::Tool { content, .. }) if content.contains("unknown Dirule tool"))
    );
}

#[test]
fn tool_call_limit_rejects_a_batch_before_requesting_approval_or_mutating() {
    let provider = FakeProvider::new([ModelResponse {
        content: None,
        tool_calls: vec![
            tool_call("inspect_target", json!({})),
            ModelToolCall {
                id: "call-2".into(),
                name: "inspect_target".into(),
                arguments: json!({}),
            },
        ],
    }]);
    let called = Arc::new(Mutex::new(false));
    let tools = catalog(ApprovalRequirement::Required, Arc::clone(&called));
    let limits = AssistantLimits {
        maximum_tool_calls: 1,
        ..AssistantLimits::default()
    };
    let assistant = Assistant::new(&provider, &tools, limits);
    let mut approval = CountingApproval::default();

    let error = assistant
        .run(
            "test-model",
            "Use Dirule tools.",
            "Rename these files.",
            &mut approval,
        )
        .expect_err("tool-call batch over the limit must be rejected");

    assert!(error.to_string().contains("tool call limit (1)"));
    assert_eq!(approval.calls, 0);
    assert!(!*called.lock().expect("called lock"));
    assert_eq!(provider.requests().len(), 1);
}

#[test]
fn oversized_tool_results_are_replaced_before_the_next_provider_request() {
    let provider = FakeProvider::new([
        response_with_tool(tool_call("inspect_target", json!({}))),
        response_text("The result was too large to include."),
    ]);
    let called = Arc::new(Mutex::new(false));
    let tools = catalog(ApprovalRequirement::None, called);
    let limits = AssistantLimits {
        maximum_tool_result_bytes: 8,
        ..AssistantLimits::default()
    };
    let assistant = Assistant::new(&provider, &tools, limits);

    assistant
        .run(
            "test-model",
            "Use Dirule tools.",
            "What is here?",
            &mut FixedApproval(false),
        )
        .expect("assistant completes after reporting the bounded-result error");

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(matches!(
        requests[1].messages.last(),
        Some(Message::Tool { content, .. })
            if content.contains("tool result exceeds the 8-byte limit")
                && !content.contains("one.txt")
    ));
}

#[test]
fn prompt_limit_is_checked_before_calling_provider() {
    let provider = FakeProvider::new([]);
    let called = Arc::new(Mutex::new(false));
    let tools = catalog(ApprovalRequirement::None, called);
    let limits = AssistantLimits {
        maximum_prompt_bytes: 3,
        ..AssistantLimits::default()
    };
    let assistant = Assistant::new(&provider, &tools, limits);

    let error = assistant
        .run("test-model", "rules", "four", &mut FixedApproval(false))
        .expect_err("oversized prompt rejected");

    assert!(error.to_string().contains("3-byte limit"));
    assert!(provider.requests().is_empty());
}

#[test]
fn assistant_rejects_a_provider_response_after_the_overall_time_limit() {
    let called = Arc::new(Mutex::new(false));
    let tools = catalog(ApprovalRequirement::None, called);
    let limits = AssistantLimits {
        maximum_elapsed: Duration::from_millis(1),
        ..AssistantLimits::default()
    };
    let assistant = Assistant::new(&SlowProvider, &tools, limits);

    let error = assistant
        .run(
            "test-model",
            "Use Dirule tools.",
            "What is here?",
            &mut FixedApproval(false),
        )
        .expect_err("late provider response must be rejected");

    assert!(error.to_string().contains("time limit"));
}

#[test]
fn duplicate_tool_names_are_rejected() {
    let first = Box::new(FixedTool {
        approval: ApprovalRequirement::None,
        called: Arc::new(Mutex::new(false)),
    }) as Box<dyn Tool>;
    let second = Box::new(FixedTool {
        approval: ApprovalRequirement::None,
        called: Arc::new(Mutex::new(false)),
    }) as Box<dyn Tool>;

    let error = match ToolCatalog::new([first, second]) {
        Ok(_) => panic!("duplicate tool should be rejected"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("registered more than once"));
}
