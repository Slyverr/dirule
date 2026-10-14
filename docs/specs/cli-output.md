# CLI output 1.3

## Output formats

Commands with `--format` accept `human` or `json`. Human output is intended for interactive use. JSON output is versioned and intended for scripts and other tools.

Successful JSON results are written to stdout as one document with a `schema_version` field. When command execution fails after argument parsing, JSON mode writes one error document to stderr and leaves stdout available for any result already produced. Argument parsing errors use Clap's standard text output.

Commands without an output-format option use human error output. `presets export` writes TOML to stdout and reports failures as human text on stderr.

## Errors

An error document has this shape:

```json
{
  "schema_version": "1.3",
  "error": {
    "code": "command_error",
    "kind": "command",
    "message": "command failed",
    "exit_code": 2
  }
}
```

The stable error codes are:

- `check_failed`: `check` completed, but found planned changes or conflicts. The command exits with status `1`.
- `command_error`: the command could not complete. The command exits with status `2`.

The `message` is for display and may change between releases. Consumers should branch on `code`, `kind`, and `exit_code`, not parse the message. A successful command exits with status `0`.

In output 1.2, error documents also include a stable `kind` that classifies the failure without changing the existing `code` or exit status. Kinds are `check_failure`, `configuration`, `configuration_selection`, `target`, `scan`, `apply`, `journal`, `preset`, and `command`. The `command` kind is the fallback for failures without a more specific category.

## Conflicts

Plan conflicts include a stable `kind`, the affected `path`, sorted and deduplicated `related_paths`, the display-oriented `reason`, and a human-readable `suggestion`. `related_paths` lists the source and destination paths of the moves involved; it lets a consumer connect a conflict to the corresponding entries in the plan. Paths are target-relative and use `/` separators on every platform.

Human plan output shows the conflict reason, related paths, and suggestion together so an operator can identify the affected moves and a useful next step.

Stable conflict kinds are:

- `duplicate_destination`: multiple planned entries share the same destination.
- `destination_occupied`: a destination already exists and is not vacated by the plan.
- `file_destination_parent`: a planned file destination is also a parent of another destination.
- `unsafe_parent`: a destination has a parent that Dirule cannot safely traverse or create.
- `dependent_move`: a move depends on another move that will be skipped because of a conflict.

Consumers should branch on `kind`; `reason` and `suggestion` are for display and may change between releases. Output 1.1 added these fields to each conflict while retaining the existing conflict fields and plan structure.

```json
{
  "kind": "duplicate_destination",
  "path": "archive/report.txt",
  "related_paths": ["Archive/report.txt", "Report.txt", "archive/report.txt"],
  "reason": "multiple entries have the same planned destination",
  "suggestion": "Adjust the rules so each entry maps to a unique destination."
}
```

When `check --format json` finds violations, stdout contains the versioned plan result and stderr contains the `check_failed` document. These are separate streams; Dirule never concatenates multiple JSON documents into one stream.

For `apply --format json`, the apply result is written to stdout. If changes need confirmation and `--yes` was not supplied, Dirule writes a human-readable plan preview to stderr before prompting. In a non-interactive session, the confirmation error is also written to stderr as a JSON error document. Therefore, stderr is not JSON-only in this confirmation-required case. Automation should preview with `plan --format json` and apply with `--yes --format json`.

In output 1.3, plan results add a `diagnostics` array. Each diagnostic has a stable `kind`, an optional target-relative `path`, the display-oriented `message`, and a human-readable `suggestion`. `path` is `null` when Dirule cannot faithfully represent it as Unicode. The existing `findings` array remains a list of message strings for compatibility. Consumers should branch on `kind`; `message` and `suggestion` may change between releases.

Stable diagnostic kinds are:

- `skipped_git_non_unicode_path`: a Git-tracked path contains non-Unicode bytes.
- `skipped_non_regular_entry`: a filesystem entry is neither a regular file nor directory.
- `skipped_non_unicode_path`: a path cannot be represented as Unicode.
- `skipped_symlink`: the path is a symbolic link, which Dirule skips.

Plan arrays have deterministic ordering for the same target and configuration. Entries follow planner order, conflicts follow planner order, and findings preserve planner order followed by scanner findings sorted lexically by message.
