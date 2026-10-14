# Quick operations

Quick operations provide a preview-first path for common sorting and naming tasks without requiring a local configuration or preset. They compile into ordinary in-memory rules and use the same scan, planner, destination validation, apply, and journal flow as configured rules.

## Commands

```text
dirule sort EXTENSION [--target PATH] [--recursive] [--apply] [--yes]
dirule sort --by extension [--target PATH] [--recursive] [--apply] [--yes]
dirule rename TRANSFORM [--target PATH] [--recursive] [--apply] [--yes]
```

`TRANSFORM` is one of `snake-case`, `kebab-case`, `camel-case`, or `pascal-case`. `EXTENSION` is an alphanumeric suffix, with an optional leading dot. `--target` defaults to the current directory. Quick operations do not load or merge `dirule.toml` or a preset.

## Scope and preview

- Preview is the default. Quick operations do not mutate files unless `--apply` is present.
- Without `--recursive`, match direct files for `sort` and direct files and directories for `rename`. A renamed directory carries its descendants through the existing virtual-path planner.
- With `--recursive`, `sort` also includes nested files and collects them into target-level bucket directories. If different directories contain files with the same name and extension, the planner reports a conflict instead of overwriting either file.
- `sort EXTENSION` matches that final extension case-insensitively and moves matches into a target-level `<lowercase-extension>s/` bucket. For example, `.PDF` files go to `pdfs/`.
- `sort --by extension` creates one sibling bucket per normalized final extension. Files with no extension, including a single-component dotfile such as `.profile`, remain unchanged. Multiple-extension files are bucketed by their final suffix.
- `rename TRANSFORM` applies the selected built-in transformation to matching entry names. It preserves file extensions and leading dots on hidden file and directory names.
- `--apply` asks for confirmation unless `--yes` is also supplied. `--yes` only affects an explicitly requested apply.
- `--on-conflict` retains the configured apply behavior: abort by default or skip conflicting operations and their dependents when `skip` is selected.
- Existing paths are never overwritten. Apply uses the normal identity checks and journal, so successful quick-operation runs can be listed and undone.
- Symlinks, ignored paths, and non-Unicode paths follow the standard scanner behavior.
- Dirule changes paths only; it does not rewrite file contents or references.

## Output

Quick operations accept `--format human|json` and the global `--color` option. Preview output uses the same plan schema and exit behavior as `plan`; apply output uses the same apply result schema and confirmation rules as `apply`. JSON output stays on stdout, except a JSON preview displayed before apply confirmation, which goes to stderr.

## Validation

CLI tests cover preview non-mutation, confirmation requirements, `--yes`, direct and recursive scope, case-insensitive extensions, extensionless names and dotfiles, existing-destination conflicts, duplicate recursive destinations, repeat-run idempotence, and undo through the normal journal. Ignore handling, symlink skipping, non-Unicode paths, identity checks, rollback, and conflict policies are shared scanner/apply behavior with separate filesystem and journal tests.
