use super::{
    MAX_PREVIEW_CHANGES, MAX_PREVIEW_RESULT_BYTES, empty_object_schema, encoded_size,
    parse_arguments,
};
use crate::{
    ApprovalRequirement, ChangeProposal, PreparedToolCall, Tool, ToolChange, ToolDefinition,
    ToolError,
};
use dirule_app::journal::{JournalRun, JournalService, JournalStatus, UndoAction};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_JOURNAL_RUNS: usize = 20;

pub(super) fn register(
    tools: &mut Vec<Box<dyn Tool>>,
    target: PathBuf,
    journal: Arc<JournalService>,
) {
    tools.push(Box::new(ListJournalRunsTool {
        target: target.clone(),
        journal: Arc::clone(&journal),
    }));
    tools.push(Box::new(InspectJournalRunTool {
        target: target.clone(),
        journal: Arc::clone(&journal),
    }));
    tools.push(Box::new(PreviewJournalUndoTool {
        target: target.clone(),
        journal: Arc::clone(&journal),
    }));
    tools.push(Box::new(UndoJournalRunTool { target, journal }));
}

struct ListJournalRunsTool {
    target: PathBuf,
    journal: Arc<JournalService>,
}

struct InspectJournalRunTool {
    target: PathBuf,
    journal: Arc<JournalService>,
}

struct PreviewJournalUndoTool {
    target: PathBuf,
    journal: Arc<JournalService>,
}

struct UndoJournalRunTool {
    target: PathBuf,
    journal: Arc<JournalService>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalRunArguments {
    run_id: String,
}

impl Tool for ListJournalRunsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_journal_runs".into(),
            description: "List up to 20 recent apply runs for the selected target only. Other project history is not exposed.".into(),
            parameters: empty_object_schema(),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        if !arguments.as_object().is_some_and(serde_json::Map::is_empty) {
            return Err(ToolError::new("`list_journal_runs` accepts no arguments"));
        }

        let target = canonical_target(&self.target)?;
        let mut runs = self
            .journal
            .list_runs()
            .map_err(|error| ToolError::new(error.to_string()))?
            .into_iter()
            .filter(|run| run.target == target)
            .take(MAX_JOURNAL_RUNS + 1)
            .collect::<Vec<_>>();
        let truncated = runs.len() > MAX_JOURNAL_RUNS;
        runs.truncate(MAX_JOURNAL_RUNS);

        let result = json!({
            "runs": runs.iter().map(run_summary).collect::<Vec<_>>(),
            "truncated": truncated,
        });
        ensure_result_size(&result)?;

        Ok(PreparedToolCall::ReadOnly(result))
    }
}

impl Tool for InspectJournalRunTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "inspect_journal_run".into(),
            description: "Inspect the recorded path changes for one apply run belonging to the selected target.".into(),
            parameters: run_id_schema(),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        let arguments: JournalRunArguments = parse_arguments(arguments)?;
        let target = canonical_target(&self.target)?;
        let details = self
            .journal
            .run_details(&arguments.run_id)
            .map_err(|error| ToolError::new(error.to_string()))?;
        ensure_target(&details.run, &target)?;
        if details.operations.len() > MAX_PREVIEW_CHANGES {
            return Err(ToolError::new(format!(
                "Ask can inspect at most {MAX_PREVIEW_CHANGES} journal operations at once"
            )));
        }

        let result = json!({
            "run": run_summary(&details.run),
            "changes": details.operations.iter().map(|operation| json!({
                "source": path_text(&operation.source),
                "destination": path_text(&operation.destination),
                "kind": entry_kind_name(operation.kind),
            })).collect::<Vec<_>>(),
        });
        ensure_result_size(&result)?;

        Ok(PreparedToolCall::ReadOnly(result))
    }
}

impl Tool for PreviewJournalUndoTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "preview_journal_undo".into(),
            description: "Preflight and show the exact path and directory actions for undoing one run belonging to the selected target. This does not undo it.".into(),
            parameters: run_id_schema(),
            approval: ApprovalRequirement::None,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        let arguments: JournalRunArguments = parse_arguments(arguments)?;
        let target = canonical_target(&self.target)?;
        let preview = self
            .journal
            .preview_undo(&arguments.run_id)
            .map_err(|error| ToolError::new(error.to_string()))?;
        ensure_target(&preview.run, &target)?;
        if preview.actions.len() > MAX_PREVIEW_CHANGES {
            return Err(ToolError::new(format!(
                "Ask can preview at most {MAX_PREVIEW_CHANGES} undo actions at once"
            )));
        }

        let result = json!({
            "run": run_summary(&preview.run),
            "actions": preview.actions.iter().map(action_value).collect::<Vec<_>>(),
        });
        ensure_result_size(&result)?;

        Ok(PreparedToolCall::ReadOnly(result))
    }
}

impl Tool for UndoJournalRunTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "undo_journal_run".into(),
            description: "Undo one preflighted apply run belonging to the selected target, only after explicit approval of every inverse move and directory action.".into(),
            parameters: run_id_schema(),
            approval: ApprovalRequirement::Required,
        }
    }

    fn prepare(&self, arguments: Value) -> Result<PreparedToolCall, ToolError> {
        let arguments: JournalRunArguments = parse_arguments(arguments)?;
        let target = canonical_target(&self.target)?;
        let preview = self
            .journal
            .preview_undo(&arguments.run_id)
            .map_err(|error| ToolError::new(error.to_string()))?;
        ensure_target(&preview.run, &target)?;
        if preview.actions.is_empty() {
            return Err(ToolError::new("journal run has no undo actions"));
        }
        if preview.actions.len() > MAX_PREVIEW_CHANGES {
            return Err(ToolError::new(format!(
                "Ask can undo at most {MAX_PREVIEW_CHANGES} journal actions at once; use `dirule undo` for larger runs"
            )));
        }

        let changes = preview.actions.iter().map(tool_change).collect::<Vec<_>>();
        let summary = format!(
            "Undo apply run {}: {} path or directory action(s) under the selected target",
            preview.run.id,
            changes.len()
        );
        let proposal = ChangeProposal { summary, changes };
        if encoded_size(&json!({
            "summary": &proposal.summary,
            "changes": &proposal.changes,
        }))? > MAX_PREVIEW_RESULT_BYTES
        {
            return Err(ToolError::new(
                "the exact undo action list exceeds the approval display limit; use `dirule undo`",
            ));
        }

        let journal = Arc::clone(&self.journal);
        let run_id = preview.run.id;
        let action = Box::new(move || {
            let report = journal
                .undo_authorized(&run_id)
                .map_err(|error| ToolError::new(error.to_string()))?;

            Ok(json!({
                "status": "undone",
                "run_id": run_id,
                "retained_directories": report.retained_directories.iter()
                    .map(|path| path_text(path))
                    .collect::<Vec<_>>(),
            }))
        });

        Ok(PreparedToolCall::Mutation { proposal, action })
    }
}

fn run_id_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "run_id": {
                "type": "string",
                "description": "An apply run ID returned by list_journal_runs for the selected target."
            }
        },
        "required": ["run_id"],
        "additionalProperties": false
    })
}

fn run_summary(run: &JournalRun) -> Value {
    json!({
        "id": run.id,
        "started_at": run.started_at,
        "status": status_name(run.status),
        "operations": run.operations,
    })
}

fn action_value(action: &UndoAction) -> Value {
    match action {
        UndoAction::Move {
            source,
            destination,
            kind,
        } => json!({
            "kind": "move",
            "source": path_text(source),
            "destination": path_text(destination),
            "entry_kind": entry_kind_name(*kind),
        }),
        UndoAction::CreateDirectory { path } => json!({
            "kind": "create_directory",
            "path": path_text(path),
        }),
        UndoAction::RemoveDirectoryIfEmpty { path } => json!({
            "kind": "remove_directory_if_empty",
            "path": path_text(path),
        }),
        UndoAction::RestoreFileContents { path } => json!({
            "kind": "restore_file_contents",
            "path": path_text(path),
        }),
    }
}

fn tool_change(action: &UndoAction) -> ToolChange {
    match action {
        UndoAction::Move {
            source,
            destination,
            ..
        } => ToolChange::Move {
            source: path_text(source),
            destination: path_text(destination),
        },
        UndoAction::CreateDirectory { path } => ToolChange::CreateDirectory {
            path: path_text(path),
        },
        UndoAction::RemoveDirectoryIfEmpty { path } => ToolChange::RemoveDirectoryIfEmpty {
            path: path_text(path),
        },
        UndoAction::RestoreFileContents { path } => ToolChange::RestoreFileContents {
            path: path_text(path),
        },
    }
}

fn ensure_target(run: &JournalRun, target: &Path) -> Result<(), ToolError> {
    if run.target == target {
        Ok(())
    } else {
        Err(ToolError::new(
            "journal run does not belong to the selected target",
        ))
    }
}

fn canonical_target(target: &Path) -> Result<PathBuf, ToolError> {
    target
        .canonicalize()
        .map_err(|_| ToolError::new("could not access the selected target"))
}

fn ensure_result_size(value: &Value) -> Result<(), ToolError> {
    if encoded_size(value)? > MAX_PREVIEW_RESULT_BYTES {
        return Err(ToolError::new(
            "journal result exceeds the Ask display limit; use Dirule's journal commands",
        ));
    }

    Ok(())
}

fn path_text(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn status_name(status: JournalStatus) -> &'static str {
    match status {
        JournalStatus::Applying => "applying",
        JournalStatus::Applied => "applied",
        JournalStatus::FailedRolledBack => "failed_rolled_back",
        JournalStatus::Interrupted => "interrupted",
        JournalStatus::Undoing => "undoing",
        JournalStatus::PartiallyRecovered => "partially_recovered",
        JournalStatus::Undone => "undone",
    }
}

fn entry_kind_name(kind: dirule_core::EntryKind) -> &'static str {
    match kind {
        dirule_core::EntryKind::File => "file",
        dirule_core::EntryKind::Directory => "directory",
        dirule_core::EntryKind::Symlink => "symlink",
    }
}
