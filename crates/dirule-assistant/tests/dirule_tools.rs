use dirule_app::configuration::ConfigurationSource;
use dirule_app::journal::JournalService;
use dirule_app::presets::PresetStore;
use dirule_app::{CategoryCatalog, ConflictPolicy, apply_authorized, sort_selectors_preview};
use dirule_assistant::{
    ApprovalHandler, Assistant, AssistantLimits, ChangeProposal, ModelProvider, ModelRequest,
    ModelResponse, ModelToolCall, ProviderError, ToolChange, ToolError,
    dirule_tool_catalog_with_categories, dirule_tool_catalog_with_services,
    dirule_tool_catalog_with_source, dirule_tool_catalog_with_sources,
};
use serde_json::json;
use std::collections::VecDeque;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;

struct FakeProvider(Mutex<VecDeque<ModelResponse>>);

impl ModelProvider for FakeProvider {
    fn complete_with_timeout(
        &self,
        _request: &ModelRequest,
        _timeout: Duration,
    ) -> Result<ModelResponse, ProviderError> {
        self.0
            .lock()
            .expect("fake response lock")
            .pop_front()
            .ok_or_else(|| ProviderError::new("no fake response remains"))
    }
}

struct InspectingProvider {
    requests: Mutex<Vec<ModelRequest>>,
}

struct TargetInspectingProvider {
    requests: Mutex<Vec<ModelRequest>>,
}

struct SourceListingProvider {
    requests: Mutex<Vec<ModelRequest>>,
}

struct PresetPreviewProvider {
    requests: Mutex<Vec<ModelRequest>>,
}

struct ApplySortProvider {
    requests: Mutex<Vec<ModelRequest>>,
}

struct ToolCallProvider {
    name: String,
    arguments: serde_json::Value,
    requests: Mutex<Vec<ModelRequest>>,
}

struct RecordedApproval {
    approved: bool,
    proposals: Mutex<Vec<ChangeProposal>>,
}

impl ModelProvider for TargetInspectingProvider {
    fn complete_with_timeout(
        &self,
        request: &ModelRequest,
        _timeout: Duration,
    ) -> Result<ModelResponse, ProviderError> {
        let mut requests = self.requests.lock().expect("request lock");
        requests.push(request.clone());
        let response = if requests.len() == 1 {
            ModelResponse {
                content: None,
                tool_calls: vec![ModelToolCall {
                    id: "inspect-target".into(),
                    name: "inspect_target".into(),
                    arguments: json!({}),
                }],
            }
        } else {
            ModelResponse {
                content: Some("I listed a bounded sample of paths.".into()),
                tool_calls: Vec::new(),
            }
        };

        Ok(response)
    }
}

impl ModelProvider for SourceListingProvider {
    fn complete_with_timeout(
        &self,
        request: &ModelRequest,
        _timeout: Duration,
    ) -> Result<ModelResponse, ProviderError> {
        let mut requests = self.requests.lock().expect("request lock");
        requests.push(request.clone());
        let response = if requests.len() == 1 {
            ModelResponse {
                content: None,
                tool_calls: vec![ModelToolCall {
                    id: "list-sources".into(),
                    name: "list_configuration_sources".into(),
                    arguments: json!({}),
                }],
            }
        } else {
            ModelResponse {
                content: Some("I listed the available configuration sources.".into()),
                tool_calls: Vec::new(),
            }
        };

        Ok(response)
    }
}

impl ModelProvider for PresetPreviewProvider {
    fn complete_with_timeout(
        &self,
        request: &ModelRequest,
        _timeout: Duration,
    ) -> Result<ModelResponse, ProviderError> {
        let mut requests = self.requests.lock().expect("request lock");
        requests.push(request.clone());
        let tool_calls = match requests.len() {
            1 => vec![ModelToolCall {
                id: "list-sources".into(),
                name: "list_configuration_sources".into(),
                arguments: json!({}),
            }],
            2 => vec![
                ModelToolCall {
                    id: "inspect-preset".into(),
                    name: "inspect_rules".into(),
                    arguments: json!({ "preset": "downloads" }),
                },
                ModelToolCall {
                    id: "preview-preset".into(),
                    name: "preview_configured_rules".into(),
                    arguments: json!({ "preset": "downloads" }),
                },
            ],
            _ => {
                return Ok(ModelResponse {
                    content: Some("The downloads preset would normalize the filename.".into()),
                    tool_calls: Vec::new(),
                });
            }
        };

        Ok(ModelResponse {
            content: None,
            tool_calls,
        })
    }
}

impl ModelProvider for ApplySortProvider {
    fn complete_with_timeout(
        &self,
        request: &ModelRequest,
        _timeout: Duration,
    ) -> Result<ModelResponse, ProviderError> {
        let mut requests = self.requests.lock().expect("request lock");
        requests.push(request.clone());
        let response = if requests.len() == 1 {
            ModelResponse {
                content: None,
                tool_calls: vec![ModelToolCall {
                    id: "apply-sort".into(),
                    name: "apply_sort".into(),
                    arguments: json!({ "selectors": ["images"] }),
                }],
            }
        } else {
            ModelResponse {
                content: Some("The requested sort is complete.".into()),
                tool_calls: Vec::new(),
            }
        };

        Ok(response)
    }
}

impl ModelProvider for ToolCallProvider {
    fn complete_with_timeout(
        &self,
        request: &ModelRequest,
        _timeout: Duration,
    ) -> Result<ModelResponse, ProviderError> {
        let mut requests = self.requests.lock().expect("request lock");
        requests.push(request.clone());
        let response = if requests.len() == 1 {
            ModelResponse {
                content: None,
                tool_calls: vec![ModelToolCall {
                    id: "requested-tool".into(),
                    name: self.name.clone(),
                    arguments: self.arguments.clone(),
                }],
            }
        } else {
            ModelResponse {
                content: Some("I checked the journal.".into()),
                tool_calls: Vec::new(),
            }
        };

        Ok(response)
    }
}

impl ApprovalHandler for RecordedApproval {
    fn approve(&mut self, proposal: &ChangeProposal) -> Result<bool, ToolError> {
        self.proposals
            .lock()
            .expect("proposal lock")
            .push(proposal.clone());

        Ok(self.approved)
    }
}

impl ModelProvider for InspectingProvider {
    fn complete_with_timeout(
        &self,
        request: &ModelRequest,
        _timeout: Duration,
    ) -> Result<ModelResponse, ProviderError> {
        let mut requests = self.requests.lock().expect("request lock");
        requests.push(request.clone());

        if requests.len() == 1 {
            return Ok(ModelResponse {
                content: None,
                tool_calls: vec![
                    ModelToolCall {
                        id: "inspect-config".into(),
                        name: "inspect_rules".into(),
                        arguments: json!({}),
                    },
                    ModelToolCall {
                        id: "preview-config".into(),
                        name: "preview_configured_rules".into(),
                        arguments: json!({}),
                    },
                ],
            });
        }

        Ok(ModelResponse {
            content: Some("The configured photo rule would normalize the filename.".into()),
            tool_calls: Vec::new(),
        })
    }
}

struct NoApproval;

impl ApprovalHandler for NoApproval {
    fn approve(&mut self, _proposal: &ChangeProposal) -> Result<bool, ToolError> {
        Ok(false)
    }
}

#[test]
fn preview_sort_uses_existing_app_service_and_does_not_mutate_target() {
    let target = tempdir().expect("temporary target");
    fs::write(target.path().join("Summer Photo.JPG"), "image bytes").expect("create image");
    let catalog = dirule_tool_catalog_with_categories(
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
    )
    .expect("tool catalog");
    let provider = FakeProvider(Mutex::new(VecDeque::from([
        ModelResponse {
            content: None,
            tool_calls: vec![ModelToolCall {
                id: "call-1".into(),
                name: "preview_sort".into(),
                arguments: json!({ "selectors": [".jpg"] }),
            }],
        },
        ModelResponse {
            content: Some("The image would move into `jpgs/`. No files were changed.".into()),
            tool_calls: Vec::new(),
        },
    ])));
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());

    let result = assistant
        .run(
            "test-model",
            "Use Dirule previews.",
            "Sort JPG files.",
            &mut NoApproval,
        )
        .expect("preview assistant response");

    assert_eq!(result.tool_calls, 1);
    assert!(result.response.contains("No files were changed"));
    assert!(target.path().join("Summer Photo.JPG").exists());
    assert!(!target.path().join("jpgs/Summer Photo.JPG").exists());
}

#[test]
fn selected_config_can_be_inspected_and_previewed_without_mutation() {
    let target = tempdir().expect("temporary target");
    let config_path = target.path().join("dirule.toml");
    let config = format!(
        "version = {:?}\nname = \"Photo cleanup\"\ntarget = \".\"\n\n[[rules]]\nname = \"Normalize photos\"\nmatch = \"<title>.JPG\"\nto = \"template:${{title|kebab-case}}.jpg\"\n",
        dirule_core::CURRENT_CONFIG_VERSION.to_string()
    );
    fs::write(&config_path, &config).expect("write project config");
    fs::write(target.path().join("Summer Photo.JPG"), "image bytes").expect("create photo");
    let source =
        ConfigurationSource::explicit_file(target.path(), std::path::PathBuf::from("dirule.toml"));
    let catalog = dirule_tool_catalog_with_source(
        target.path().to_path_buf(),
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
        Some(source),
    )
    .expect("tool catalog");
    let provider = InspectingProvider {
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());

    let result = assistant
        .run(
            "test-model",
            "Inspect and preview the selected config.",
            "How would this rule rename the photo?",
            &mut NoApproval,
        )
        .expect("assistant response");

    assert_eq!(result.tool_calls, 2);
    assert!(result.response.contains("would normalize the filename"));
    assert_eq!(
        fs::read_to_string(config_path).expect("read unchanged config"),
        config
    );
    assert_eq!(
        fs::read_to_string(target.path().join("Summer Photo.JPG")).expect("read photo"),
        "image bytes"
    );
    assert!(!target.path().join("summer-photo.jpg").exists());

    let requests = provider.requests.lock().expect("request lock");
    let tool_results = requests[1]
        .messages
        .iter()
        .filter_map(|message| match message {
            dirule_assistant::Message::Tool { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(tool_results.len(), 2);
    let encoded_results = tool_results.join("\n");
    assert!(encoded_results.contains("Normalize photos"));
    assert!(encoded_results.contains("Summer Photo.JPG"));
    assert!(encoded_results.contains("summer-photo.jpg"));
    assert!(!encoded_results.contains("image bytes"));
}

#[test]
fn tool_catalog_exposes_validated_config_authoring_without_unrestricted_access() {
    let target = tempdir().expect("temporary target");
    let catalog = dirule_tool_catalog_with_categories(
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
    )
    .expect("tool catalog");
    let names = catalog
        .definitions()
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        [
            "draft_project_config",
            "inspect_rules",
            "inspect_target",
            "list_categories",
            "list_configuration_sources",
            "preview_configured_rules",
            "preview_rename",
            "preview_sort",
            "save_project_config"
        ]
    );
    let save = catalog
        .definitions()
        .iter()
        .find(|tool| tool.name == "save_project_config")
        .expect("save tool definition");
    assert_eq!(
        save.approval,
        dirule_assistant::ApprovalRequirement::Required
    );
    assert!(
        !names
            .iter()
            .any(|name| name.contains("shell") || name.contains("read_file"))
    );
}

#[test]
fn assistant_can_select_a_listed_preset_for_read_only_inspection_and_preview() {
    let target = tempdir().expect("temporary target");
    let original_name = target.path().join("ReadMe.TXT");
    fs::write(&original_name, "private file bytes").expect("create target file");
    let preset_directory = tempdir().expect("temporary preset directory");
    let presets = PresetStore::new(preset_directory.path()).expect("preset store");
    let config = format!(
        "version = {:?}\nname = \"Text cleanup\"\ntarget = \".\"\n\n[[rules]]\nname = \"Normalize text names\"\nmatch = \"*.TXT\"\nto = \"builtin:kebab-case\"\n",
        dirule_core::CURRENT_CONFIG_VERSION.to_string()
    );
    fs::write(presets.path("downloads").expect("preset path"), &config).expect("write preset");
    let catalog = dirule_tool_catalog_with_sources(
        target.path().to_path_buf(),
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
        None,
        Some(presets),
    )
    .expect("tool catalog");
    let provider = PresetPreviewProvider {
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());

    let result = assistant
        .run(
            "test-model",
            "Use available read-only tools.",
            "Inspect and preview the downloads preset.",
            &mut NoApproval,
        )
        .expect("assistant response");

    assert!(result.response.contains("normalize the filename"));
    assert!(original_name.exists());
    assert!(!target.path().join("read-me.txt").exists());
    let requests = provider.requests.lock().expect("request lock");
    let tool_results = requests[2]
        .messages
        .iter()
        .filter_map(|message| match message {
            dirule_assistant::Message::Tool { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let encoded_results = tool_results.join("\n");

    assert_eq!(tool_results.len(), 3);
    assert!(encoded_results.contains("Normalize text names"));
    assert!(encoded_results.contains("ReadMe.TXT"));
    assert!(encoded_results.contains("read-me.TXT"), "{encoded_results}");
    assert!(!encoded_results.contains("private file bytes"));
}

#[test]
fn apply_sort_waits_for_exact_approval_and_records_the_change() {
    let target = tempdir().expect("temporary target");
    let original_name = target.path().join("Portrait.JPG");
    fs::write(&original_name, "private image bytes").expect("create image");
    let journal_directory = tempdir().expect("temporary journal directory");
    let journal = Arc::new(JournalService::at(journal_directory.path()));
    let catalog = dirule_tool_catalog_with_services(
        target.path().to_path_buf(),
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
        None,
        None,
        Some(Arc::clone(&journal)),
    )
    .expect("tool catalog");
    let provider = ApplySortProvider {
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());
    let mut approval = RecordedApproval {
        approved: true,
        proposals: Mutex::new(Vec::new()),
    };

    let result = assistant
        .run(
            "test-model",
            "Apply tools require an exact user approval.",
            "Sort images into their category folders.",
            &mut approval,
        )
        .expect("assistant response");

    assert_eq!(result.approvals_granted, 1);
    assert!(!original_name.exists());
    assert!(target.path().join("media/images/Portrait.JPG").is_file());
    let proposals = approval.proposals.lock().expect("proposal lock");
    assert_eq!(proposals.len(), 1);
    assert_eq!(
        proposals[0].changes,
        [ToolChange::Move {
            source: "Portrait.JPG".into(),
            destination: "media/images/Portrait.JPG".into(),
        }]
    );
    let runs = journal.list_runs().expect("list journaled apply");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].operations, 1);
    let requests = provider.requests.lock().expect("request lock");
    let apply_result = requests[1]
        .messages
        .iter()
        .find_map(|message| match message {
            dirule_assistant::Message::Tool { content, .. } => Some(content),
            _ => None,
        })
        .expect("apply tool result");
    assert!(apply_result.contains("\"status\":\"applied\""));
    assert!(!apply_result.contains("private image bytes"));
}

#[test]
fn journal_tools_list_only_runs_for_the_selected_target() {
    let target = tempdir().expect("temporary selected target");
    let other_target = tempdir().expect("temporary unrelated target");
    let journal_directory = tempdir().expect("temporary journal directory");
    let journal = Arc::new(JournalService::at(journal_directory.path()));
    let selected_run = record_image_sort(target.path(), &journal);
    let unrelated_run = record_image_sort(other_target.path(), &journal);
    let catalog = dirule_tool_catalog_with_services(
        target.path().to_path_buf(),
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
        None,
        None,
        Some(Arc::clone(&journal)),
    )
    .expect("tool catalog");
    let provider = ToolCallProvider {
        name: "list_journal_runs".into(),
        arguments: json!({}),
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());

    assistant
        .run(
            "test-model",
            "List only selected-target history.",
            "Show recent runs.",
            &mut NoApproval,
        )
        .expect("assistant response");

    let requests = provider.requests.lock().expect("request lock");
    let result = requests[1]
        .messages
        .iter()
        .find_map(|message| match message {
            dirule_assistant::Message::Tool { content, .. } => Some(content),
            _ => None,
        })
        .expect("journal tool result");
    let result: serde_json::Value = serde_json::from_str(result).expect("journal result JSON");
    let runs = result["runs"].as_array().expect("run list");

    assert!(runs.iter().any(|run| run["id"] == selected_run));
    assert!(runs.iter().all(|run| run["id"] != unrelated_run));
    assert!(
        !result
            .to_string()
            .contains(other_target.path().to_str().unwrap())
    );
}

#[test]
fn undo_journal_run_shows_move_and_directory_actions_before_applying() {
    let target = tempdir().expect("temporary target");
    let original_name = target.path().join("Portrait.JPG");
    let journal_directory = tempdir().expect("temporary journal directory");
    let journal = Arc::new(JournalService::at(journal_directory.path()));
    let run_id = record_image_sort(target.path(), &journal);
    let catalog = dirule_tool_catalog_with_services(
        target.path().to_path_buf(),
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
        None,
        None,
        Some(Arc::clone(&journal)),
    )
    .expect("tool catalog");
    let provider = ToolCallProvider {
        name: "undo_journal_run".into(),
        arguments: json!({ "run_id": run_id }),
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());
    let mut approval = RecordedApproval {
        approved: true,
        proposals: Mutex::new(Vec::new()),
    };

    let result = assistant
        .run(
            "test-model",
            "Undo requires approval of the displayed actions.",
            "Undo this apply run.",
            &mut approval,
        )
        .expect("assistant response");

    assert_eq!(result.approvals_granted, 1);
    assert!(original_name.is_file());
    assert!(!target.path().join("media/images/Portrait.JPG").exists());
    assert_eq!(
        journal.list_runs().expect("journal runs")[0].status,
        dirule_app::journal::JournalStatus::Undone
    );
    let proposals = approval.proposals.lock().expect("proposal lock");
    assert_eq!(proposals.len(), 1);
    assert!(proposals[0].changes.contains(&ToolChange::Move {
        source: "media/images/Portrait.JPG".into(),
        destination: "Portrait.JPG".into(),
    }));
    assert!(
        proposals[0]
            .changes
            .contains(&ToolChange::RemoveDirectoryIfEmpty {
                path: "media/images".into(),
            })
    );
}

#[test]
fn declined_journal_undo_preserves_the_applied_run() {
    let target = tempdir().expect("temporary target");
    let journal_directory = tempdir().expect("temporary journal directory");
    let journal = Arc::new(JournalService::at(journal_directory.path()));
    let run_id = record_image_sort(target.path(), &journal);
    let catalog = dirule_tool_catalog_with_services(
        target.path().to_path_buf(),
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
        None,
        None,
        Some(Arc::clone(&journal)),
    )
    .expect("tool catalog");
    let provider = ToolCallProvider {
        name: "undo_journal_run".into(),
        arguments: json!({ "run_id": run_id }),
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());
    let mut approval = RecordedApproval {
        approved: false,
        proposals: Mutex::new(Vec::new()),
    };

    assistant
        .run(
            "test-model",
            "Undo requires approval of the displayed actions.",
            "Undo this apply run.",
            &mut approval,
        )
        .expect("assistant response");

    assert!(!target.path().join("Portrait.JPG").exists());
    assert!(target.path().join("media/images/Portrait.JPG").is_file());
    assert_eq!(
        journal.list_runs().expect("journal runs")[0].status,
        dirule_app::journal::JournalStatus::Applied
    );
    assert_eq!(approval.proposals.lock().expect("proposal lock").len(), 1);
}

#[test]
fn preview_journal_undo_preflights_without_changing_the_target() {
    let target = tempdir().expect("temporary target");
    let journal_directory = tempdir().expect("temporary journal directory");
    let journal = Arc::new(JournalService::at(journal_directory.path()));
    let run_id = record_image_sort(target.path(), &journal);
    let catalog = dirule_tool_catalog_with_services(
        target.path().to_path_buf(),
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
        None,
        None,
        Some(Arc::clone(&journal)),
    )
    .expect("tool catalog");
    let provider = ToolCallProvider {
        name: "preview_journal_undo".into(),
        arguments: json!({ "run_id": run_id }),
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());

    assistant
        .run(
            "test-model",
            "Use read-only journal previews.",
            "Can this run be undone?",
            &mut NoApproval,
        )
        .expect("assistant response");

    assert!(!target.path().join("Portrait.JPG").exists());
    assert!(target.path().join("media/images/Portrait.JPG").is_file());
    assert_eq!(
        journal.list_runs().expect("journal runs")[0].status,
        dirule_app::journal::JournalStatus::Applied
    );
    let requests = provider.requests.lock().expect("request lock");
    let result = requests[1]
        .messages
        .iter()
        .find_map(|message| match message {
            dirule_assistant::Message::Tool { content, .. } => Some(content),
            _ => None,
        })
        .expect("undo preview result");
    assert!(result.contains("remove_directory_if_empty"));
    assert!(result.contains("Portrait.JPG"));
}

#[test]
fn draft_project_config_validates_without_writing_a_file() {
    let target = tempdir().expect("temporary target");
    let contents = test_config();
    let catalog = dirule_tool_catalog_with_categories(
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
    )
    .expect("tool catalog");
    let provider = ToolCallProvider {
        name: "draft_project_config".into(),
        arguments: json!({ "contents": contents }),
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());

    assistant
        .run(
            "test-model",
            "Validate a proposed project config.",
            "Draft one rule for text files.",
            &mut NoApproval,
        )
        .expect("assistant response");

    assert!(!target.path().join("dirule.toml").exists());
    let result = tool_result(&provider);
    assert_eq!(result["valid"], true);
    assert_eq!(result["rule_count"], 1);
    assert_eq!(result["contents"], test_config());
}

#[test]
fn save_project_config_requires_approval_of_exact_contents() {
    let target = tempdir().expect("temporary target");
    let contents = test_config();
    let catalog = dirule_tool_catalog_with_categories(
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
    )
    .expect("tool catalog");
    let provider = ToolCallProvider {
        name: "save_project_config".into(),
        arguments: json!({ "contents": contents }),
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());
    let mut approval = RecordedApproval {
        approved: true,
        proposals: Mutex::new(Vec::new()),
    };

    let result = assistant
        .run(
            "test-model",
            "Save only after exact approval.",
            "Create this project config.",
            &mut approval,
        )
        .expect("assistant response");

    let path = target
        .path()
        .canonicalize()
        .expect("canonical target")
        .join("dirule.toml");
    assert_eq!(result.approvals_granted, 1);
    assert_eq!(
        fs::read_to_string(&path).expect("saved config"),
        test_config()
    );
    let proposals = approval.proposals.lock().expect("proposal lock");
    assert_eq!(proposals[0].changes.len(), 1);
    match &proposals[0].changes[0] {
        ToolChange::WriteFile {
            path: proposed_path,
            contents,
        } => {
            assert_eq!(std::path::PathBuf::from(proposed_path), path);
            assert_eq!(contents, &test_config());
        }
        change => panic!("expected file creation proposal, got {change:?}"),
    }
    assert_eq!(tool_result(&provider)["status"], "created");
}

#[test]
fn declined_project_config_save_does_not_create_the_file() {
    let target = tempdir().expect("temporary target");
    let catalog = dirule_tool_catalog_with_categories(
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
    )
    .expect("tool catalog");
    let provider = ToolCallProvider {
        name: "save_project_config".into(),
        arguments: json!({ "contents": test_config() }),
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());
    let mut approval = RecordedApproval {
        approved: false,
        proposals: Mutex::new(Vec::new()),
    };

    assistant
        .run(
            "test-model",
            "Save only after exact approval.",
            "Create this project config.",
            &mut approval,
        )
        .expect("assistant response");

    assert!(!target.path().join("dirule.toml").exists());
    assert_eq!(approval.proposals.lock().expect("proposal lock").len(), 1);
}

fn record_image_sort(target: &std::path::Path, journal: &JournalService) -> String {
    fs::write(target.join("Portrait.JPG"), "image bytes").expect("create image");
    let preview = sort_selectors_preview(
        target,
        &["images".into()],
        &CategoryCatalog::built_in(),
        false,
    )
    .expect("sort preview");
    apply_authorized(preview, ConflictPolicy::Abort, journal)
        .expect("apply and journal sort")
        .run_id
        .expect("run ID")
}

fn test_config() -> String {
    format!(
        "version = {:?}\nname = \"Text names\"\ntarget = \".\"\n\n[[rules]]\nname = \"Normalize text names\"\nmatch = \"*.txt\"\nto = \"builtin:kebab-case\"\n",
        dirule_core::CURRENT_CONFIG_VERSION.to_string()
    )
}

fn tool_result(provider: &ToolCallProvider) -> serde_json::Value {
    let requests = provider.requests.lock().expect("request lock");
    let result = requests[1]
        .messages
        .iter()
        .find_map(|message| match message {
            dirule_assistant::Message::Tool { content, .. } => Some(content),
            _ => None,
        })
        .expect("tool result");
    serde_json::from_str(result).expect("tool result JSON")
}

#[test]
fn declined_apply_sort_leaves_the_target_and_journal_unchanged() {
    let target = tempdir().expect("temporary target");
    let original_name = target.path().join("Portrait.JPG");
    fs::write(&original_name, "image bytes").expect("create image");
    let journal_directory = tempdir().expect("temporary journal directory");
    let journal = Arc::new(JournalService::at(journal_directory.path()));
    let catalog = dirule_tool_catalog_with_services(
        target.path().to_path_buf(),
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
        None,
        None,
        Some(Arc::clone(&journal)),
    )
    .expect("tool catalog");
    let provider = ApplySortProvider {
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());
    let mut approval = RecordedApproval {
        approved: false,
        proposals: Mutex::new(Vec::new()),
    };

    let result = assistant
        .run(
            "test-model",
            "Apply tools require an exact user approval.",
            "Sort images into their category folders.",
            &mut approval,
        )
        .expect("assistant response");

    assert_eq!(result.approvals_granted, 0);
    assert!(original_name.is_file());
    assert!(!target.path().join("media/images/Portrait.JPG").exists());
    assert!(journal.list_runs().expect("list journal runs").is_empty());
}

#[test]
fn source_listing_exposes_only_local_availability_and_global_preset_names() {
    let target = tempdir().expect("temporary target");
    let local_config = target.path().join("dirule.toml");
    fs::write(&local_config, "LOCAL_CONFIG_CONTENT_MUST_NOT_BE_SENT").expect("write local config");
    let preset_directory = tempdir().expect("temporary preset directory");
    let presets = PresetStore::new(preset_directory.path()).expect("preset store");
    fs::write(
        presets.path("downloads").expect("preset path"),
        "PRESET_CONTENT_MUST_NOT_BE_SENT",
    )
    .expect("write preset");
    fs::write(
        presets.path("broken").expect("preset path"),
        "PRIVATE_BROKEN_CONTENT",
    )
    .expect("write malformed preset");
    for index in 0..65 {
        fs::write(
            presets
                .path(&format!("extra-{index:02}"))
                .expect("preset path"),
            "",
        )
        .expect("write additional preset");
    }
    let selected_source =
        ConfigurationSource::preset(&presets, "downloads").expect("select preset source");
    let catalog = dirule_tool_catalog_with_sources(
        target.path().to_path_buf(),
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
        Some(selected_source),
        Some(presets),
    )
    .expect("tool catalog");
    let provider = SourceListingProvider {
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());

    assistant
        .run(
            "test-model",
            "List available source names only.",
            "What rule sources are available?",
            &mut NoApproval,
        )
        .expect("assistant response");

    let requests = provider.requests.lock().expect("request lock");
    let tool_result = requests[1]
        .messages
        .iter()
        .find_map(|message| match message {
            dirule_assistant::Message::Tool { content, .. } => Some(content),
            _ => None,
        })
        .expect("source listing tool result");
    let value: serde_json::Value = serde_json::from_str(tool_result).expect("tool result JSON");
    let items = value["global_presets"]["items"]
        .as_array()
        .expect("global preset items");

    assert_eq!(value["project_config"]["available"], true);
    assert_eq!(value["project_config"]["selected"], false);
    assert_eq!(value["global_presets"]["count"], 67);
    assert_eq!(value["global_presets"]["truncated"], true);
    assert_eq!(items.len(), 64);
    assert!(tool_result.len() <= 24 * 1024);
    assert_eq!(items[0]["name"], "broken");
    assert_eq!(items[1]["name"], "downloads");
    assert_eq!(items[1]["selected"], true);
    assert!(!tool_result.contains("LOCAL_CONFIG_CONTENT_MUST_NOT_BE_SENT"));
    assert!(!tool_result.contains("PRESET_CONTENT_MUST_NOT_BE_SENT"));
    assert!(!tool_result.contains("PRIVATE_BROKEN_CONTENT"));
    assert!(!tool_result.contains(preset_directory.path().to_str().expect("UTF-8 path")));
}

#[test]
fn inspect_target_returns_a_truncated_names_only_listing() {
    let target = tempdir().expect("temporary target");
    fs::write(target.path().join(".diruleignore"), "excluded.txt\n")
        .expect("write Dirule exclusions");
    fs::write(target.path().join("excluded.txt"), "private file bytes")
        .expect("write excluded file");
    for index in 0..105 {
        fs::write(
            target
                .path()
                .join(format!("entry-{index:03}-{}.txt", "x".repeat(230))),
            "private file bytes",
        )
        .expect("write target file");
    }

    let catalog = dirule_tool_catalog_with_categories(
        target.path().to_path_buf(),
        CategoryCatalog::built_in(),
    )
    .expect("tool catalog");
    let provider = TargetInspectingProvider {
        requests: Mutex::new(Vec::new()),
    };
    let assistant = Assistant::new(&provider, &catalog, AssistantLimits::default());
    let result = assistant
        .run(
            "test-model",
            "Inspect the selected target.",
            "What files are here?",
            &mut NoApproval,
        )
        .expect("inspection assistant response");
    let requests = provider.requests.lock().expect("request lock");
    let tool_result = requests[1]
        .messages
        .iter()
        .find_map(|message| match message {
            dirule_assistant::Message::Tool { content, .. } => Some(content),
            _ => None,
        })
        .expect("target inspection tool result");
    let value: serde_json::Value = serde_json::from_str(tool_result).expect("tool result JSON");

    assert!(result.response.contains("bounded sample"));
    let returned_entries = value["entries"].as_array().expect("entries");
    assert!(!returned_entries.is_empty());
    assert!(returned_entries.len() < 100);
    assert_eq!(value["listing_truncated"], true);
    assert!(tool_result.len() <= 24 * 1024);
    assert!(tool_result.contains("entry-"));
    assert!(!tool_result.contains("excluded.txt"));
    assert!(!tool_result.contains("private file bytes"));
}
