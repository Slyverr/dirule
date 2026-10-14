# Changelog

## Unreleased

### Fixed

- Reject category extensions with multiple leading dots instead of silently normalizing them.

## Version 0.2.1

### Fixed

- Preserve comments and formatting when editing an existing TOML configuration.

### Improved

- Defer `.diruleignore` matcher setup until a Git-tracked path needs recovery.

## Version 0.2.0

### Added

- Preview-first `sort` and `rename` commands that reuse the existing planner, apply flow, and undo journal.
- Multi-selector sorting with built-in and user-configurable file categories.
- Target-aware configuration discovery from a local `dirule.toml` or one global preset bound to the exact target.
- Target-wide `ignore` patterns that exclude matching paths even when Git tracks them.
- Project-level `.diruleignore` exclusions for tracked and untracked paths, while VCS-ignored untracked paths remain excluded.
- Structured plan findings, conflict details, and command errors in JSON output.

### Fixed

- Valid rename chains and cycles no longer conflict with destinations vacated by other planned moves.
- Scans skip Git metadata paths and return findings in stable order.

### Improved

- Name the starter rule accurately for both files and directories.
- Reduce scan and conflict-planning overhead on large targets and configurations with many path patterns.

## Version 0.1.0

### Added

- CLI commands to initialize and validate configs, preview and check plans, apply changes, manage presets, and inspect or undo journaled runs.
- Ordered, versioned TOML rules with glob matching, named captures, destination templates, built-in case transforms, custom acronyms, entry-kind filters, and exclusions.
- Ignore-aware scanning and conflict-safe file and directory renames. Dirule changes paths without rewriting file contents.
- Human-readable and JSON output, terminal color controls, and shell completion generation.
- Common short options for targets, configs, presets, output formats, and non-interactive confirmation.
