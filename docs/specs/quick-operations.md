# Quick operations

Quick operations provide a preview-first path for common sorting and naming tasks without requiring a local configuration or preset. They compile into ordinary in-memory rules and use the same scan, planner, destination validation, apply, and journal flow as configured rules.

## Commands

```text
dirule sort SELECTOR... [--target PATH] [--recursive] [--apply] [--yes]
dirule sort --by extension [--target PATH] [--recursive] [--apply] [--yes]
dirule rename TRANSFORM [--target PATH] [--recursive] [--apply] [--yes]
```

`SELECTOR` is an alphanumeric extension, with an optional leading dot, or a category name. Multiple selectors are accepted. A category name takes precedence over an extension with the same spelling; prefix that extension with `.` to select it literally. `--by extension` sorts every file with a final extension into its own bucket. `TRANSFORM` is one of `snake-case`, `kebab-case`, `camel-case`, or `pascal-case`. `--target` defaults to the current directory. Quick operations do not load or merge `dirule.toml` or a preset.

## Sort categories

Built-in categories are `images`, `videos`, `audio`, and `documents`. They match the final filename extension case-insensitively and move matching files to `media/images/`, `media/videos/`, `media/audio/`, and `documents/`, respectively. Category lists use explicit extensions and do not inspect file contents. The documents category is a curated extension group; media types do not define a general document family.

Users may add categories or replace built-in definitions in `categories.toml` under Dirule's platform-specific user configuration directory: `$XDG_CONFIG_HOME/dirule/categories.toml` on Linux and non-macOS Unix targets, falling back to `$HOME/.config/dirule/categories.toml` when `XDG_CONFIG_HOME` is unset; `~/Library/Application Support/dirule/categories.toml` on macOS; or `%APPDATA%/dirule/categories.toml` on Windows. For example:

```toml
version = "1.0"

[[categories]]
name = "images"
extensions = ["png", "jpg", "jpeg", "raw"]
destination = "media/images"

[[categories]]
name = "ebooks"
extensions = ["epub", "mobi"]
destination = "reading/books"
```

Category names start with a lowercase letter and contain lowercase letters, digits, or hyphens. Extensions are ASCII letters or digits; an optional leading dot is accepted. Destinations are relative paths under the selected target and use `/`. Defining a category with a built-in name replaces that built-in's extensions and destination. Other definitions add categories. The optional catalog is versioned independently from project `dirule.toml` files. An absent file uses built-in categories; malformed, unknown-version, or unsafe definitions are errors. If the selected categories or extensions overlap but route the same extension to different destinations, Dirule reports an error before scanning.

## Scope and preview

- Preview is the default. Quick operations do not mutate files unless `--apply` is present.
- Without `--recursive`, match direct files for `sort` and direct files and directories for `rename`. A renamed directory carries its descendants through the existing virtual-path planner.
- With `--recursive`, `sort` also includes nested files and collects them into target-level bucket directories. If different directories contain files with the same name and extension, the planner reports a conflict instead of overwriting either file.
- `sort SELECTOR...` accepts multiple extensions or categories. Extension selectors match their final extension case-insensitively and move matches into a target-level `<lowercase-extension>s/` bucket. For example, `sort pdf png` uses `pdfs/` and `pngs/`.
- Category selectors match their configured extensions and route files to the category destination. For example, `sort images videos` routes image and video files to their separate media folders. A leading dot forces an extension selector when it has the same name as a category.
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

CLI tests cover preview non-mutation, confirmation requirements, `--yes`, direct and recursive scope, multiple extensions and categories, case-insensitive extensions, custom category overrides, extensionless names and dotfiles, overlapping selectors, existing-destination conflicts, duplicate recursive destinations, repeat-run idempotence, and undo through the normal journal. Ignore handling, symlink skipping, non-Unicode paths, identity checks, rollback, and conflict policies are shared scanner/apply behavior with separate filesystem and journal tests.
