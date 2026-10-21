use super::args;
use dirule_app::configuration::{self, ConfigurationSource, ConfigurationSourceKind};
use dirule_app::presets::PresetStore;
use dirule_assistant::{
    ApprovalHandler, Assistant, AssistantConfig, AssistantConversation, AssistantError,
    AssistantLimits, ChangeProposal, ModelProvider, OpenAiCompatibleConfig,
    OpenAiCompatibleProvider, OpenAiResponsesConfig, OpenAiResponsesProvider, ProviderAdapter,
    ToolError, dirule_tool_catalog_with_source,
};
use std::error::Error;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

struct InteractiveApproval {
    progress: super::output::ProgressHandle,
    color: args::ColorChoice,
}

impl ApprovalHandler for InteractiveApproval {
    fn approve(&mut self, proposal: &ChangeProposal) -> Result<bool, ToolError> {
        self.progress.pause();
        let styles = super::output::Colorizer::stderr(self.color);
        let mut output = io::stderr().lock();
        writeln!(
            output,
            "\n{} {}",
            styles.style("Approval required", "33"),
            styles.style(&proposal.summary, "1")
        )
        .map_err(|error| ToolError::new(error.to_string()))?;
        for change in &proposal.changes {
            let line = match change {
                dirule_assistant::ToolChange::Move {
                    source,
                    destination,
                } => format!("  {source:?} -> {destination:?}"),
                dirule_assistant::ToolChange::CreateDirectory { path } => {
                    format!("  Create directory {path:?}")
                }
                dirule_assistant::ToolChange::RemoveDirectoryIfEmpty { path } => {
                    format!("  Remove directory if empty {path:?}")
                }
                dirule_assistant::ToolChange::WriteFile { path, contents } => {
                    writeln!(
                        output,
                        "  Create validated file {path:?} ({} bytes):",
                        contents.len()
                    )
                    .map_err(|error| ToolError::new(error.to_string()))?;
                    for line in contents.split_inclusive('\n') {
                        writeln!(output, "    {line:?}")
                            .map_err(|error| ToolError::new(error.to_string()))?;
                    }
                    continue;
                }
                dirule_assistant::ToolChange::RestoreFileContents { path } => {
                    format!("  Restore file contents {path:?}")
                }
            };
            writeln!(output, "{line}").map_err(|error| ToolError::new(error.to_string()))?;
        }
        drop(output);

        if !io::stdin().is_terminal() {
            return Err(ToolError::new(
                "Dirule Ask needs an interactive terminal to approve path changes; no changes were applied",
            ));
        }

        super::prompt::confirm(
            "Apply this exact proposal?",
            "Dirule Ask needs an interactive terminal to approve path changes",
            false,
        )
        .map_err(|error| ToolError::new(error.to_string()))
    }
}

pub(super) fn ask(options: args::AskArgs, color: args::ColorChoice) -> Result<(), Box<dyn Error>> {
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
    let provider: Box<dyn ModelProvider> = match &profile.adapter {
        ProviderAdapter::OpenAiCompatible => Box::new(OpenAiCompatibleProvider::from_config(
            OpenAiCompatibleConfig::new(
                profile.endpoint.clone(),
                profile.model.clone(),
                profile.api_key_env.clone(),
            ),
        )?),
        ProviderAdapter::OpenAiResponses => Box::new(OpenAiResponsesProvider::from_config(
            OpenAiResponsesConfig::new(
                profile.endpoint.clone(),
                profile.model.clone(),
                profile.api_key_env.clone(),
            ),
        )?),
    };
    let input = read_prompt(prompt_arguments)?;
    let base = std::env::current_dir()?;
    let target = dirule_app::paths::resolve_target(
        &base,
        target_argument.unwrap_or_else(|| PathBuf::from(".")),
    )?;
    let source = select_rule_source(&base, &target, rules)?;
    let categories = dirule_app::CategoryCatalog::user_if_available()?;
    let tools =
        dirule_tool_catalog_with_source(target.clone(), base.clone(), categories, source.clone())?;
    let mutating_tools_available = tools
        .definitions()
        .iter()
        .any(|tool| tool.approval == dirule_assistant::ApprovalRequirement::Required);
    let operation_policy = if mutating_tools_available {
        "Filesystem and configuration changes require explicit terminal approval of the exact displayed proposal. Save a configuration only when the user asked to create one. Never claim a change was applied unless the matching tool succeeded."
    } else {
        "This invocation is read-only and can inspect or preview but cannot change paths."
    };
    let selected_config = source.as_ref().map_or_else(
        || "No project rule config or preset is selected.".to_owned(),
        |source| {
            format!(
                "The selected rule source is {:?}. You may inspect its rules, preview them, or request an apply through an approval-gated tool.",
                source.name()
            )
        },
    );
    let instructions = format!(
        "You are Dirule Ask, a specialist assistant for Dirule's file and directory naming and sorting capabilities. Interpret the user's goal, use the provided Dirule tools to list available configuration sources, inspect categories or selected rules, create previews, and inspect the apply journal for the selected target. Explain results plainly. You may inspect or preview a listed global preset by passing its name to the rules tools. The selected target is {}. Keep operations within that target. {} {} Do not request or infer file contents.",
        target.display(),
        selected_config,
        operation_policy
    );
    let assistant = Assistant::new(provider.as_ref(), &tools, AssistantLimits::default());

    super::output::assistant_notice(profile_name, &profile.model, &target, color)?;

    match input {
        AskInput::OneShot(prompt) => {
            let mut conversation = assistant.conversation(&instructions);
            let result = run_turn(
                &assistant,
                &profile.model,
                &mut conversation,
                &prompt,
                color,
            )?;
            super::output::assistant_response(&result.response, color).map_err(Into::into)
        }
        AskInput::Interactive => {
            let session_instructions = format!(
                "{instructions} In interactive chat, call end_session only when the user clearly asks to end this conversation. Do not use it when the user asks to stop or cancel a filesystem task."
            );
            let mut conversation = assistant.conversation_with_session_end(&session_instructions);
            run_chat(
                &assistant,
                profile_name,
                &profile.model,
                &target,
                &mut conversation,
                color,
            )
        }
    }
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

enum AskInput {
    OneShot(String),
    Interactive,
}

fn read_prompt(arguments: Vec<String>) -> Result<AskInput, Box<dyn Error>> {
    let maximum_bytes = AssistantLimits::default().maximum_prompt_bytes;
    if !arguments.is_empty() {
        let prompt = arguments.join(" ");
        return validate_prompt(prompt, maximum_bytes).map(AskInput::OneShot);
    }

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return Ok(AskInput::Interactive);
    }

    if io::stdin().is_terminal() {
        return Err("interactive Ask needs both stdin and stdout attached to a terminal; pass a prompt or pipe one on stdin".into());
    }

    let mut bytes = Vec::new();
    io::stdin()
        .take(maximum_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum_bytes {
        return Err(format!("prompt exceeds the {maximum_bytes}-byte limit").into());
    }

    validate_prompt(String::from_utf8(bytes)?, maximum_bytes).map(AskInput::OneShot)
}

fn validate_prompt(prompt: String, maximum_bytes: usize) -> Result<String, Box<dyn Error>> {
    if prompt.len() > maximum_bytes {
        return Err(format!("prompt exceeds the {maximum_bytes}-byte limit").into());
    }
    if prompt.trim().is_empty() {
        return Err("provide a prompt, pipe one on stdin, or run `dirule ask` in a terminal to start a conversation".into());
    }

    Ok(prompt.trim().to_owned())
}

fn run_turn(
    assistant: &Assistant<'_>,
    model: &str,
    conversation: &mut AssistantConversation,
    prompt: &str,
    color: args::ColorChoice,
) -> Result<dirule_assistant::AssistantResult, AssistantError> {
    let mut progress = super::output::AssistantProgress::new(color);
    let mut approval = InteractiveApproval {
        progress: progress.handle(),
        color,
    };
    let result = assistant.run_conversation_turn_with_observer(
        model,
        conversation,
        prompt,
        &mut approval,
        &mut progress,
    );
    drop(progress);

    result
}

fn run_chat(
    assistant: &Assistant<'_>,
    profile: &str,
    model: &str,
    target: &Path,
    conversation: &mut AssistantConversation,
    color: args::ColorChoice,
) -> Result<(), Box<dyn Error>> {
    let styles = super::output::Colorizer::stdout(color);
    writeln!(
        io::stdout().lock(),
        "Type a request and press Enter. Use {} for commands.",
        styles.style("/help", "36")
    )?;

    let stdin = io::stdin();
    let mut line = String::new();
    loop {
        write!(io::stdout().lock(), "{} ", styles.style("you>", "36"))?;
        io::stdout().flush()?;
        line.clear();
        if stdin.lock().read_line(&mut line)? == 0 {
            writeln!(io::stdout().lock())?;
            return Ok(());
        }

        let prompt = line.trim();
        if prompt.is_empty() {
            continue;
        }

        match prompt {
            "/help" => {
                write_chat_help(color)?;
                continue;
            }
            "/reset" => {
                conversation.reset();
                writeln!(
                    io::stdout().lock(),
                    "{} Conversation context cleared; target and profile stay the same.",
                    styles.style("✓", "32")
                )?;
                continue;
            }
            "/status" => {
                let mut stdout = io::stdout().lock();
                writeln!(stdout, "{} {profile}", styles.style("Profile:", "90"))?;
                writeln!(stdout, "{} {model}", styles.style("Model:", "90"))?;
                writeln!(
                    stdout,
                    "{} {}\n",
                    styles.style("Target:", "90"),
                    target.display()
                )?;
                continue;
            }
            "/quit" | "/exit" => {
                writeln!(io::stdout().lock())?;
                return Ok(());
            }
            _ => {}
        }

        match run_turn(assistant, model, conversation, prompt, color) {
            Ok(result) => {
                if result.response.trim().is_empty() && result.session_ended {
                    writeln!(
                        io::stdout().lock(),
                        "{} Session ended.",
                        styles.style("✓", "32")
                    )?;
                } else {
                    super::output::assistant_response(&result.response, color)?;
                }
                if result.session_ended {
                    return Ok(());
                }
            }
            Err(error) => report_turn_error(&error, color)?,
        }
    }
}

fn write_chat_help(color: args::ColorChoice) -> io::Result<()> {
    let styles = super::output::Colorizer::stdout(color);
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "\n{}", styles.style("Conversation commands", "1"))?;
    writeln!(stdout, "  {}  Show this help", styles.style("/help", "36"))?;
    writeln!(
        stdout,
        "  {}  Clear conversation context",
        styles.style("/reset", "36")
    )?;
    writeln!(
        stdout,
        "  {}  Show the active profile, model, and target",
        styles.style("/status", "36")
    )?;
    writeln!(
        stdout,
        "  {}  End the conversation (`/exit` also works)",
        styles.style("/quit", "36")
    )?;
    writeln!(stdout)
}

fn report_turn_error(error: &dyn Error, color: args::ColorChoice) -> io::Result<()> {
    let styles = super::output::Colorizer::stderr(color);
    writeln!(
        io::stderr().lock(),
        "\n{} {error}\n",
        styles.style("Ask error:", "31")
    )
}
