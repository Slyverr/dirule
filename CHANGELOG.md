# Changelog

## Unreleased

### Added

- Structured findings with stable kinds, affected paths, and suggestions in JSON plan output.
- Structured conflict details and stable error categories in JSON output.

### Improved

- Speed up large directory scans by reducing path-indexing overhead.

## Version 0.1.0

### Added

- CLI commands to initialize and validate configs, preview and check plans, apply changes, manage presets, and inspect or undo journaled runs.
- Ordered, versioned TOML rules with glob matching, named captures, destination templates, built-in case transforms, custom acronyms, entry-kind filters, and exclusions.
- Ignore-aware scanning and conflict-safe file and directory renames. Dirule changes paths without rewriting file contents.
- Human-readable and JSON output, terminal color controls, and shell completion generation.
- Common short options for targets, configs, presets, output formats, and non-interactive confirmation.
