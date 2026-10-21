# Configuration

Project rules live in `dirule.toml` at the target's root. Start with `dirule init`, then edit the generated file. `dirule validate` checks and compiles the selected config without scanning the target.

## Example

```toml
version = "1.5"
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

`version` is required. `name`, `target`, `ignore`, and `acronyms` are optional. Rules run in declaration order; the first rule matching an entry's original path and `kind` owns it. A rule's `exclude` skips that path for that rule, allowing later rules to match. `kind` accepts `file`, `directory`, or `any` and defaults to `any`. `mode` accepts `apply` or `check` and defaults to `apply`.

A `check` mode rule reports the destination it expects but never renames that entry, including during `apply`. `check` fails while any check-mode rule has an unmet destination. Other matching `apply` rules can still run, and a check-mode directory does not virtually move its descendants.

```toml
[[rules]]
name = "Require kebab case in CI"
match = "src/**/*.rs"
mode = "check"
to = "builtin:kebab-case"
```

## Matching and destinations

Patterns match the full target-relative path, use `/` on every platform, and are case-sensitive by default. Dirule supports glob patterns and named captures such as `<year>`; it does not treat patterns as regular expressions. Common glob forms include `*` for characters within one path component, `**` across path components, `?` for one character, character classes, and brace alternatives.

`to` uses `builtin:` or `template:`. Built-in transformations are `snake-case`, `kebab-case`, `camel-case`, and `pascal-case`. Templates can combine captures and common values such as `${name}`, `${stem}`, `${extension}`, `${parent}`, and `${path}`. A filter such as `${vendor|kebab-case}` transforms a value before inserting it. Built-in transformations share a word splitter that recognizes common case boundaries and configured acronym spellings.

The `[acronyms]` table is project-specific and empty by default. Add spellings whose boundaries cannot be inferred, for example `iOS` in `iOSGuide`. There is no bundled acronym dictionary.

## Language-server reference updates

Configure language servers to update source references when a planned rename changes a filename. For example:

```toml
[lsp]
reference_updates = "required"

[[lsp.servers]]
command = "rust-analyzer"
args = []
root_markers = ["Cargo.toml"]

[lsp.servers.languages]
rs = "rust"
```

`reference_updates` accepts `required`, `best-effort`, or `off` and defaults to `required`. The policy is project-wide; individual servers do not override it. It applies to file extensions mapped by configured servers; other paths remain ordinary path-only renames. With `required`, planning fails if a matching server is missing, cannot start, or cannot return a supported edit. `best-effort` keeps the path plan and shows a warning when reference updates are unavailable. `off` skips LSP requests.

Before requesting edits, Dirule opens the scanned source files that are being renamed in the configured server session. This gives servers that build their project view from opened documents enough context to find references. The configured server receives those source texts through its local stdio process; choose a server command you trust.

`dirule plan` starts each matching server and requests `workspace/willRenameFiles`; its preview includes the returned source edits. `dirule apply` applies those edits with the path changes and journals both, so `dirule undo RUN_ID` restores the previous paths and source contents if they have not changed since the apply. The preview reads source documents selected by the server, but it does not send them to a remote service.

`root_markers` are searched upward from each renamed path to find the language-server workspace root. Dirule groups rename requests by the matching root, which supports several language projects under one target; if no marker is found, the operation target is used. This also works when the config comes from a global preset. Run `dirule lsp status` to check whether configured executables can be found. Executable names are looked up in `PATH`; an absolute path or a path relative to the config file also works. Arguments are passed directly without a shell. Status only checks executable availability; `plan` is the command that tests whether a server can handle a real rename request.

See the [configuration specification](specs/dirule-config.md) for the full pattern grammar, template rules, migrations, and edge-case behavior.

## Selecting a config

`plan`, `check`, and `apply` use the explicitly selected `--config FILE` or `--preset NAME` first. Otherwise, Dirule checks for `dirule.toml` directly in the selected target. If it is absent, Dirule may select one global preset whose absolute or `~/` target resolves to that same directory. Multiple matching presets are an error; no match means no config is available. Relative preset targets do not bind globally, and Dirule does not search parent directories. An explicit `--target PATH` selects the target and takes precedence over a config's target.

## Excluding paths

Dirule respects VCS ignore rules, including `.gitignore`, `.ignore`, and Git repository or global excludes. In a Git repository, ignored untracked paths are skipped while tracked paths remain eligible. Outside Git, `.ignore` files still filter scans.

Use `.diruleignore` for extra Dirule-specific exclusions. It applies to tracked and untracked paths and does not support negation. VCS-ignored untracked paths stay excluded. The top-level `ignore` list in `dirule.toml` is an absolute exclusion: matching files and directories (including their descendants) are never scanned or planned, even if tracked. It does not edit any ignore file. A rule's `exclude` only affects that rule and may let a later rule claim the path.

The exact precedence and path behavior are in the specification's [configuration shape](specs/dirule-config.md#configuration-shape) section.
