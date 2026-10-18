use dirule_assistant::{AssistantConfig, CURRENT_ASSISTANT_CONFIG_VERSION, ProviderAdapter};
use std::path::Path;

const CONFIG: &str = r#"
version = "1.0"
default_profile = "gemini"

[profiles.gemini]
adapter = "openai-compatible"
endpoint = "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
model = "gemini-2.5-flash"
api_key_env = "GEMINI_API_KEY"

[profiles.nvidia]
adapter = "openai-compatible"
endpoint = "https://integrate.api.nvidia.com/v1/chat/completions"
model = "nvidia-model-id"
api_key_env = "NVIDIA_API_KEY"

[profiles.openai]
adapter = "openai-responses"
endpoint = "https://api.openai.com/v1/responses"
model = "gpt-5.6"
api_key_env = "OPENAI_API_KEY"
"#;

#[test]
fn loads_versioned_profiles_and_selects_default_or_named_profile() {
    let config = AssistantConfig::parse(Path::new("assistant.toml"), CONFIG)
        .expect("valid assistant config");

    assert_eq!(config.default_profile_name(), "gemini");
    assert_eq!(config.profile(None).unwrap().0, "gemini");
    assert_eq!(config.profile(Some("nvidia")).unwrap().0, "nvidia");
    assert_eq!(config.profile(Some("openai")).unwrap().0, "openai");
    assert_eq!(
        config.profile(Some("gemini")).unwrap().1.adapter,
        ProviderAdapter::OpenAiCompatible
    );
    assert_eq!(
        config.profile(Some("openai")).unwrap().1.adapter,
        ProviderAdapter::OpenAiResponses
    );
    assert_eq!(CURRENT_ASSISTANT_CONFIG_VERSION.to_string(), "1.0");
}

#[test]
fn rejects_unsupported_profile_version_before_using_the_config() {
    let config = CONFIG.replace("version = \"1.0\"", "version = \"2.0\"");

    let error = AssistantConfig::parse(Path::new("assistant.toml"), &config)
        .expect_err("unsupported version rejected");

    assert!(error.to_string().contains("version 2.0 is unsupported"));
}

#[test]
fn rejects_undefined_default_profile_and_invalid_environment_names() {
    let missing_default = CONFIG.replace(
        "default_profile = \"gemini\"",
        "default_profile = \"missing\"",
    );
    assert!(
        AssistantConfig::parse(Path::new("assistant.toml"), &missing_default)
            .expect_err("missing profile rejected")
            .to_string()
            .contains("is not defined")
    );

    let invalid_env = CONFIG.replace("GEMINI_API_KEY", "GEMINI-API-KEY");
    assert!(
        AssistantConfig::parse(Path::new("assistant.toml"), &invalid_env)
            .expect_err("invalid env name rejected")
            .to_string()
            .contains("invalid `api_key_env` variable name")
    );
}
