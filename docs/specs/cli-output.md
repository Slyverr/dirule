# CLI output 1.0

## Output formats

Commands with `--format` accept `human` or `json`. Human output is intended for interactive use. JSON output is versioned and intended for scripts and other tools.

Successful JSON results are written to stdout as one document with a `schema_version` field. When command execution fails after argument parsing, JSON mode writes one error document to stderr and leaves stdout available for any result already produced. Argument parsing errors use Clap's standard text output.

Commands without an output-format option use human error output. `presets export` writes TOML to stdout and reports failures as human text on stderr.

## Errors

An error document has this shape:

```json
{
  "schema_version": "1.0",
  "error": {
    "code": "command_error",
    "message": "invalid configuration ...",
    "exit_code": 2
  }
}
```

The stable error codes are:

- `check_failed`: `check` completed, but found planned changes or conflicts. The command exits with status `1`.
- `command_error`: the command could not complete. The command exits with status `2`.

The `message` is for display and may change between releases. Consumers should branch on `code` and `exit_code`, not parse the message. A successful command exits with status `0`.

When `check --format json` finds violations, stdout contains the versioned plan result and stderr contains the `check_failed` document. These are separate streams; Dirule never concatenates multiple JSON documents into one stream.
