use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum AssistantError {
    Provider(ProviderError),
    Tool(ToolError),
    PromptTooLarge { limit: usize },
    LimitExceeded { limit: &'static str, maximum: usize },
    MissingFinalResponse,
}

#[derive(Debug)]
pub struct ProviderError {
    message: String,
}

#[derive(Debug)]
pub struct ToolError {
    message: String,
}

impl ProviderError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AssistantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provider(error) => fmt::Display::fmt(error, formatter),
            Self::Tool(error) => fmt::Display::fmt(error, formatter),
            Self::PromptTooLarge { limit } => {
                write!(formatter, "prompt exceeds the {limit}-byte limit")
            }
            Self::LimitExceeded { limit, maximum } => {
                write!(
                    formatter,
                    "assistant exceeded the {limit} limit ({maximum})"
                )
            }
            Self::MissingFinalResponse => {
                formatter.write_str("model did not return a final response within the turn limit")
            }
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AssistantError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Provider(error) => Some(error),
            Self::Tool(error) => Some(error),
            Self::PromptTooLarge { .. }
            | Self::LimitExceeded { .. }
            | Self::MissingFinalResponse => None,
        }
    }
}

impl Error for ProviderError {}
impl Error for ToolError {}

impl From<ProviderError> for AssistantError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(error)
    }
}

impl From<ToolError> for AssistantError {
    fn from(error: ToolError) -> Self {
        Self::Tool(error)
    }
}
