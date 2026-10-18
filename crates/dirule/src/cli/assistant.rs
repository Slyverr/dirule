use super::args;
use dirule_app::configuration::{self, ConfigurationSource, ConfigurationSourceKind};
use dirule_app::presets::PresetStore;
use dirule_assistant::{
    ApprovalHandler, Assistant, AssistantConfig, AssistantLimits, ChangeProposal,
    OpenAiCompatibleConfig, OpenAiCompatibleProvider, ProviderAdapter, ToolError,
    dirule_tool_catalog_with_source,
};
use std::error::Error;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

const MAX_PROMPT_BYTES: usize = 32 * 1024;

struct DeclineMutations;

impl ApprovalHandler for DeclineMutations {
    fn approve(&mut self, _proposal: &ChangeProposal) -> Result<bool, ToolError> {
        Ok(false)
    }
}

pub(super) fn ask(options: args::AskArgs) -> Result<(), Box<dyn Error>> {
    let args::AskArgs {
        prompt: prompt_arguments,
        target: target_argument,
        profile: profile_name_argument,
        assistant_config,
        rules,
    } = options;
    let config = match assistant_config {
        Some(path) => AssistantConfig::load(path)?,
        None => AssistantConfig::load_user()?,
    };
    let (profile_name, profile) = config.profile(profile_name_argument.as_deref())?;
    match &profile.adapter {
        ProviderAdapter::OpenAiCompatible => {}
    }

    let provider = OpenAiCompatibleProvider::from_config(OpenAiCompatibleConfig::new(
        profile.endpoint.clone(),
        profile.model.clone(),
        profile.api_key_env.clone(),
    ))?;
    let prompt = read_prompt(prompt_arguments)?;
    let base = std::env::current_dir()?;
    let target = dirule_app::paths::resolve_target(
        &base,
        target_argument.unwrap_or_else(|| PathBuf::from(".")),
    )?;
    let source = select_rule_source(&base, &target, rules)?;
    let categories = dirule_app::CategoryCatalog::user_if_available()?;
    let tools =
        dirule_tool_catalog_with_source(target.clone(), base.clone(), categories, source.clone())?;
    let selected_config = source.as_ref().map_or_else(
        || "No project rule config or preset is selected.".to_owned(),
        |source| {
            format!(
                "The selected rule source is {:?}. You may inspect its rules or preview them with the provided read-only tools.",
                source.name()
            )
        },
    );
    let instructions = format!(
        "You are Dirule Ask, a specialist assistant for Dirule's file and directory naming and sorting capabilities. Interpret the user's goal, use the provided Dirule tools to list available configuration sources, inspect categories or selected rules, and create read-only previews, then explain the result plainly. Never claim that files were changed: this version can only inspect and preview. The selected target is {}. Keep operations within that target. {} Do not request or infer file contents. Listing another preset does not select it; tell the user to rerun with --preset NAME to inspect a different one.",
        target.display(),
        selected_config
    );
    let assistant = Assistant::new(&provider, &tools, AssistantLimits::default());

    eprintln!(
        "Using profile {profile_name:?} with model {:?} at {}. The prompt, target path, selected rule excerpts, available preset names, requested names, and preview data may be sent to that provider; file contents are not read by these tools.",
        profile.model, profile.endpoint
    );
    eprintln!("Review the provider's data policy before sending project names or paths.");

    let result = assistant.run(
        &profile.model,
        &instructions,
        &prompt,
        &mut DeclineMutations,
    )?;
    println!("{}", result.response);

    Ok(())
}

fn select_rule_source(
    base: &Path,
    target: &Path,
    rules: args::RuleSourceArgs,
) -> Result<Option<ConfigurationSource>, Box<dyn Error>> {
    let explicit_source = if let Some(path) = rules.config {
        Some(ConfigurationSource::explicit_file(base, path))
    } else if let Some(name) = rules.preset {
        Some(ConfigurationSource::preset(&PresetStore::user()?, &name)?)
    } else {
        None
    };

    if let Some(source) = explicit_source {
        configuration::load_source(&source)?;

        return Ok(Some(source));
    }

    let presets = PresetStore::user_if_available()?;
    let source =
        configuration::discover_source(base, Some(target.to_path_buf()), presets.as_ref())?;
    if source.kind() == ConfigurationSourceKind::ProjectDefault && !source.path().is_file() {
        return Ok(None);
    }

    configuration::load_source(&source)?;

    Ok(Some(source))
}

fn read_prompt(arguments: Vec<String>) -> Result<String, Box<dyn Error>> {
    if !arguments.is_empty() {
        let prompt = arguments.join(" ");
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(format!("prompt exceeds the {MAX_PROMPT_BYTES}-byte limit").into());
        }

        return Ok(prompt);
    }

    if io::stdin().is_terminal() {
        eprint!("What would you like Dirule to do? ");
        io::stderr().flush()?;
    }

    let mut bytes = Vec::new();
    io::stdin()
        .take((MAX_PROMPT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PROMPT_BYTES {
        return Err(format!("prompt exceeds the {MAX_PROMPT_BYTES}-byte limit").into());
    }

    Ok(String::from_utf8(bytes)?)
}
