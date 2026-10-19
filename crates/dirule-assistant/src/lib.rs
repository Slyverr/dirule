mod assistant;
mod builtins;
mod credentials;
mod error;
mod profile;
mod protocol;
mod provider;
mod tools;

pub use assistant::{Assistant, AssistantConversation, AssistantLimits, AssistantResult};
pub use builtins::{
    dirule_tool_catalog, dirule_tool_catalog_with_categories, dirule_tool_catalog_with_services,
    dirule_tool_catalog_with_source, dirule_tool_catalog_with_sources,
};
pub use error::{AssistantError, ProviderError, ToolError};
pub use profile::{
    AssistantConfig, AssistantConfigError, AssistantProfile, CURRENT_ASSISTANT_CONFIG_VERSION,
    ProviderAdapter,
};
pub use protocol::{
    ApprovalRequirement, AssistantEvent, AssistantObserver, ChangeProposal, Message, ModelProvider,
    ModelRequest, ModelResponse, ModelToolCall, ToolChange, ToolDefinition,
};
pub use provider::{
    OpenAiCompatibleConfig, OpenAiCompatibleProvider, OpenAiResponsesConfig,
    OpenAiResponsesProvider,
};
pub use tools::{ApprovalHandler, PreparedToolCall, Tool, ToolCatalog};
