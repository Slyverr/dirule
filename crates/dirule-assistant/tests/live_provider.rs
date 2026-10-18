use dirule_app::CategoryCatalog;
use dirule_assistant::{
    ApprovalHandler, Assistant, AssistantConfig, AssistantLimits, ChangeProposal,
    OpenAiCompatibleConfig, OpenAiCompatibleProvider, ProviderAdapter, ToolError,
    dirule_tool_catalog_with_categories,
};
use std::fs;
use tempfile::tempdir;

struct DeclineMutations;

impl ApprovalHandler for DeclineMutations {
    fn approve(&mut self, _proposal: &ChangeProposal) -> Result<bool, ToolError> {
        Ok(false)
    }
}

#[test]
#[ignore = "requires a configured provider profile and its API key"]
fn configured_provider_calls_a_read_only_dirule_tool() {
    let config = match std::env::var_os("DIRULE_ASSISTANT_CONFIG") {
        Some(path) => AssistantConfig::load(path).expect("load selected assistant config"),
        None => AssistantConfig::load_user().expect("load user assistant.toml"),
    };
    let profile_name = std::env::var("DIRULE_LIVE_PROFILE").ok();
    let (selected_name, profile) = config
        .profile(profile_name.as_deref())
        .expect("select live test profile");
    match profile.adapter {
        ProviderAdapter::OpenAiCompatible => {}
    }

    let provider = OpenAiCompatibleProvider::from_config(OpenAiCompatibleConfig::new(
        profile.endpoint.clone(),
        profile.model.clone(),
        profile.api_key_env.clone(),
    ))
    .expect("construct configured provider");
    let target = tempdir().expect("create synthetic target");
    let source = target.path().join("Summer Photo.JPG");
    fs::write(&source, "synthetic image placeholder").expect("create synthetic file");
    let tools = dirule_tool_catalog_with_categories(
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
    )
    .expect("create read-only Dirule tool catalog");
    let assistant = Assistant::new(&provider, &tools, AssistantLimits::default());

    let result = assistant
        .run(
            &profile.model,
            "You are testing Dirule's read-only tools on a synthetic temporary folder. You must call preview_sort with selectors [\"images\"] before answering. Never claim that files were changed.",
            "Preview sorting the image in this folder.",
            &mut DeclineMutations,
        )
        .expect("live provider response");

    assert!(
        result.tool_calls > 0,
        "profile {selected_name:?} answered without calling a Dirule tool"
    );
    assert!(!result.response.trim().is_empty());
    assert!(source.is_file(), "preview changed the source file");
    assert!(
        !target.path().join("images/Summer Photo.JPG").exists(),
        "preview moved the source file"
    );

    println!(
        "Profile {selected_name:?} called {} Dirule tool(s): {}",
        result.tool_calls, result.response
    );
}
