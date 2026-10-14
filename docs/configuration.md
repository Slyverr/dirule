# Configuration

Project rules live in `dirule.toml` at the target's root. Start with `dirule init`, then edit the generated file. `dirule validate` checks and compiles the selected config without scanning the target.

## Example

```toml
version = "1.3"
name = "Downloads"
target = "."
ignore = ["generated/api/**", "vendor/**"]

[acronyms]
entries = ["iOS", "eBay"]

[[rules]]
name = "Archive invoices"
match = "Invoices/<year>-<vendor>-<title>.pdf"
kind = "file"
to = "template:Archive/${year}/${vendor|kebab-case}/${title|kebab-case}.pdf"

[[rules]]
name = "Normalize directories"
match = "**/*"
kind = "directory"
to = "builtin:kebab-case"

[[rules]]
name = "Normalize remaining files"
match = "**/*"
kind = "file"
exclude = ["**/*.md"]
to = "builtin:kebab-case"
```

`version` is required. `name`, `target`, `ignore`, and `acronyms` are optional. Rules run in declaration order; the first rule matching an entry's original path and `kind` owns it. A rule's `exclude` skips that path for that rule, allowing later rules to match. `kind` accepts `file`, `directory`, or `any` and defaults to `any`.

## Matching and destinations

Patterns match the full target-relative path, use `/` on every platform, and are case-sensitive by default. Dirule supports glob patterns and named captures such as `<year>`; it does not treat patterns as regular expressions. Common glob forms include `*` for characters within one path component, `**` across path components, `?` for one character, character classes, and brace alternatives.

`to` uses `builtin:` or `template:`. Built-in transformations are `snake-case`, `kebab-case`, `camel-case`, and `pascal-case`. Templates can combine captures and common values such as `${name}`, `${stem}`, `${extension}`, `${parent}`, and `${path}`. A filter such as `${vendor|kebab-case}` transforms a value before inserting it. Built-in transformations share a word splitter that recognizes common case boundaries and configured acronym spellings.

The `[acronyms]` table is project-specific and empty by default. Add spellings whose boundaries cannot be inferred, for example `iOS` in `iOSGuide`. There is no bundled acronym dictionary.

See the [configuration specification](specs/dirule-config.md) for the full pattern grammar, template rules, migrations, and edge-case behavior.

## Selecting a config

`plan`, `check`, and `apply` use the explicitly selected `--config FILE` or `--preset NAME` first. Otherwise, Dirule checks for `dirule.toml` directly in the selected target. If it is absent, Dirule may select one global preset whose absolute or `~/` target resolves to that same directory. Multiple matching presets are an error; no match means no config is available. Relative preset targets do not bind globally, and Dirule does not search parent directories. An explicit `--target PATH` selects the target and takes precedence over a config's target.

## Excluding paths

Dirule respects VCS ignore rules, including `.gitignore`, `.ignore`, and Git repository or global excludes. In a Git repository, ignored untracked paths are skipped while tracked paths remain eligible. Outside Git, `.ignore` files still filter scans.

Use `.diruleignore` for extra Dirule-specific exclusions. It applies to tracked and untracked paths and does not support negation. VCS-ignored untracked paths stay excluded. The top-level `ignore` list in `dirule.toml` is an absolute exclusion: matching files and directories (including their descendants) are never scanned or planned, even if tracked. It does not edit any ignore file. A rule's `exclude` only affects that rule and may let a later rule claim the path.

The exact precedence and path behavior are in the specification's [configuration shape](specs/dirule-config.md#configuration-shape) section.
