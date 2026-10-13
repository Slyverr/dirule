# Apply journal and undo 1.0

## Purpose

Applying a plan must leave enough local state to inspect the run and safely undo completed renames. Undo is best-effort recovery, not a content backup or a promise of whole-run atomicity. A changed destination must never be overwritten or silently moved back.

This specification covers the CLI apply path. It does not add a TUI, MCP integration, file-content operations, deletion, copy, or cross-device moves.

## State location

Run data is stored outside the target tree:

- Linux and other Unix systems: `$XDG_STATE_HOME/dirule/runs/`, falling back to `$HOME/.local/state/dirule/runs/`.
- macOS: `$HOME/Library/Application Support/dirule/runs/`.
- Windows: `%LOCALAPPDATA%/dirule/runs/`.

If the state location cannot be determined or written, apply fails before changing target paths. `plan` and `check` never create journal state. An apply with no operations, a rejected plan, or a declined confirmation creates no run.

Each run has a unique, user-visible identifier and its own append-only journal. A journal contains the canonical target path, start time, plan fingerprint, format version, and ordered filesystem mutation events. Use JSON Lines so each intent and completion can be appended and synced independently. Restrict permissions on Unix state directories and journal files to the owning user.

Journal readers migrate older formats through registered one-step migrations before interpreting events. Reject ambiguous or missing migration routes and unknown future versions; preserve the original journal when migration fails.

## Recording apply

Before the first mutation, create and sync the run header. Before each filesystem mutation, append and sync an intent containing the operation, relative paths, entry kind, and expected object identity. After the mutation, append and sync its completion and observed postcondition.

Record each actual no-replace rename separately, including both legs of case-only renames through a temporary sibling. Record directories created for destination parents. A recovery reader may reconcile an intent without a completion only when the source/destination state and stored identity prove whether the operation happened. Ambiguous states are reported and left untouched.

The journal records metadata, not file contents. File identity uses the strongest portable metadata available, including file ID where the platform exposes one, plus type, size, and modification time. Directory identity uses type and file ID where available; directory timestamps can change when contents change. Unsupported or unverifiable identities reduce undo eligibility rather than weakening checks.

The existing immediate rollback after an apply error remains. Rollback steps are journaled too. A process interruption can leave an incomplete run; it must remain inspectable and eligible for verified recovery when its operations can be reconciled.

## Run listing

`dirule runs list` lists run identifiers, targets, timestamps, and status. Status is derived from the valid journal events and includes applying, applied, interrupted, partially recovered, and undone. A truncated final JSONL record may be ignored and reconciled from its preceding intent; malformed complete records or an unsupported format version are errors. Listing never changes the target or journal.

## Undo

`dirule undo RUN_ID` restores completed renames in reverse execution order. It requires interactive confirmation; `--yes` allows a non-interactive run without relaxing any safety checks.

Before undoing, verify that each moved object is still at its recorded destination with the recorded postcondition, and that every original path is available when its reversal runs. Reject the undo before mutating if a destination was replaced or modified, a required source is missing, the target identity changed, or a reversal would overwrite an existing path. Never follow symlinks or escape the canonical target root.

Journal every reverse rename and directory cleanup with the same intent/completion protocol, so an interrupted undo can be retried safely. Remove only empty destination-parent directories that the journal proves Dirule created. If an external file now occupies one, preserve it and report that cleanup was skipped. Repeating a completed undo reports that the run is already undone.

An interrupted apply may be undone when the journal and filesystem state prove which moves completed. An unresolved operation blocks recovery for that run; do not guess. A failed immediate rollback remains visible as a partially recovered run.

## CLI output

Human apply output includes the run identifier and the command to undo it. `runs list` provides identifiers needed after an interrupted command. JSON apply output returns one JSON document containing the plan, execution result, and run identifier; it must not concatenate multiple JSON documents. JSON run listing and undo output use the same explicit schema version field as other machine-readable results.

## Acceptance tests

- `check`, `plan`, rejected apply, declined apply, and no-op apply create no run state.
- A successful apply records the target, fingerprint, each rename, and the returned run identifier.
- Undo restores files and directories, preserves file contents, and handles case-only renames.
- Replaced or modified destinations, occupied original paths, and a changed target are rejected without mutation.
- Interruption after a recorded intent but before completion can be reconciled safely; ambiguous state is refused.
- Interruption during undo is recoverable by retrying the same run identifier.
- A newly created parent is removed only when empty and known to belong to the run.
- Apply-time rollback failures remain visible in the run list and do not discard recovery data.
- Human and JSON apply/run outputs expose enough information to locate and undo a run.
