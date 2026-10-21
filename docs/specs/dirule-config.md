# Dirule configuration 1.5

## Purpose and scope

This specification defines the project configuration file, rule format, and behavior shared by `plan`, `check`, and `apply`. The file is named `dirule.toml` and uses TOML. A configuration can combine built-in transformations with custom path matching and destination templates.

The format must be able to grow through explicit migrations. New matchers, transformations, and filesystem actions may be added without silently changing the meaning of existing configurations. Configurations without a rule `mode` migrate as `mode = "apply"`.

## Format evolution

Every configuration declares `version` as a quoted `major.minor` string. The reader parses that field first, migrates older documents through a complete sequence of registered one-step migrations, then validates and loads the current schema. Each source version must have at most one registered migration; ambiguous routes are configuration errors. The runtime configuration does not carry format-version branches. Documents without `kind` migrate with `kind = "any"`; documents without `exclude` migrate with an empty list; documents without top-level `ignore` migrate with an empty list, preserving their prior behavior.

A migration must preserve the document's meaning, update its version, and produce a document accepted by the next migration. Readers reject unknown future versions and any older version without a complete migration path; they never skip versions or guess how to interpret fields. Changes that preserve existing meaning may advance the minor component. Changes that alter existing meaning require a major component change and an explicit migration policy.

## Configuration shape

`name` is optional and labels the configuration. `target` is optional and defaults to `.`. `ignore` is an optional list of target-relative path patterns that excludes matching source paths before rule matching. `rules` is an ordered, non-empty array. Each rule may set `kind` to `any`, `file`, or `directory`; it defaults to `any`. `mode` accepts `apply` or `check` and defaults to `apply`. The optional per-rule `exclude` list skips entries from that rule while allowing later rules to match. The optional `acronyms` table lists project-specific spellings for the word splitter used by built-in case transformations and template filters. The optional `lsp` table sets a project-wide reference-update policy and configures language-server executables.

```toml
version = "1.5"
name = "Downloads"
target = "."
ignore = ["generated/api/**", "vendor/**"]

[acronyms]
entries = ["iOS", "eBay"]

[lsp]
reference_updates = "required"

[[lsp.servers]]
command = "rust-analyzer"
args = []
root_markers = ["Cargo.toml"]

[lsp.servers.languages]
rs = "rust"

[[rules]]
name = "Sort invoices by year and vendor"
match = "Invoices/<year>-<vendor>-<title>.pdf"
kind = "file"
to = "template:Archive/${year}/${vendor}/${title|kebab-case}.pdf"

[[rules]]
name = "Normalize remaining filenames"
match = "**/*"
kind = "file"
exclude = ["**/*.md"]
to = "builtin:kebab-case"
```

Top-level `ignore` patterns are evaluated against original target-relative source paths before rule matching. A matching directory and all of its descendants are excluded from scanning and planning, including when Git tracks them; no rule can claim an excluded source. This setting does not prevent a rule from choosing a destination under a matching path. The patterns add to VCS ignore handling and do not edit `.gitignore`, `.ignore`, or any other project file. Quick operations do not load a configuration and therefore do not use its `ignore` list.

A target may also contain `.diruleignore`, which uses ignore-file patterns to add Dirule-specific exclusions. It applies to tracked and untracked paths, and negation patterns are invalid. VCS-ignored untracked paths stay excluded; tracked paths remain eligible unless excluded by `.diruleignore` or top-level `ignore`. Dirule reads this file without changing it.

Rules are evaluated in declaration order. The first rule matching both the entry's kind and path owns it. A kind mismatch or matching per-rule exclusion does not claim the entry, so later rules can still match it. Per-rule exclusions use the same path-relative glob syntax as `match`, inspect original paths, and apply only to their containing rule; excluding a directory from one rule does not automatically exclude its descendants. The parser splits `to` at its first colon, so colons later in a value remain part of that value. Unknown fields, kinds, namespaces, transformations, malformed patterns, exclusions, and invalid templates are configuration errors with the relevant field or rule identified.

A rule with `mode = "check"` claims matching entries just like an apply-mode rule, so later rules do not take ownership. Dirule reports the destination that the rule expects, but does not schedule that rule's rename. `check` fails while the expected destination differs from the path after any apply-mode parent-directory moves. `apply` can execute other apply-mode operations and leaves check-mode names unchanged. A check-mode directory does not supply a virtual rename to its descendants; children continue from the directory path that will actually remain.

```toml
[[rules]]
name = "Require kebab case in CI"
match = "src/**/*.rs"
mode = "check"
to = "builtin:kebab-case"
```

## Matching

Patterns match the complete path relative to the selected target. `/` is the separator in configuration patterns on every platform. Matching is case-sensitive by default. `(?i)` switches the following part of the current component to case-insensitive matching; `(?-i)` switches it back. The supported glob syntax is:

| Syntax | Meaning |
| --- | --- |
| `*` | Zero or more characters within one path component. |
| `?` | One character within one path component. |
| `[abc]`, `[a-z]` | One character from a set or range. |
| `[!abc]` | One character outside a set. |
| `{jpg,png}` | One alternative; nested alternatives are unsupported. |
| `**` | Zero or more path components. |
| `(?i)`, `(?-i)` | Switch case-insensitive matching on or off for the remainder of the component. |
| `<name>` | Capture one or more characters within a component, ending at the next literal delimiter or `/`. |
| `\*` | Literal `*`; in TOML, use a literal string such as `match = '\*'`. |

Additional matching rules:

- `*`, `?`, character classes, and captures do not cross `/`; `**` does.
- `**/` can match zero directories, so `**/*.pdf` matches both `invoice.pdf` and `archive/invoice.pdf`.
- Capture names are unique per pattern and use `[A-Za-z_][A-Za-z0-9_]*`. A template may reuse a capture.
- Adjacent captures, and captures adjacent to `*` or `?`, are invalid because their boundaries are ambiguous. `<year>-<title>.pdf` is valid; `<first><last>.txt` is not.
- Captures inside alternatives are unsupported. Alternatives expose the selected branch as a single value.
- Reject unknown or malformed syntax; do not interpret patterns as regular expressions.

Use the maintained `wax` matcher through a small adapter for named captures. Use `ignore` for recursive traversal and ignore-file handling rather than implementing glob or ignore syntax from scratch.

## Destinations

The `to` value has the form `namespace:value`. The initial namespaces are `builtin` and `template`.

A built-in transformation changes the entry name and preserves its planned parent. For files, it transforms the stem and preserves the extension. For directories, it transforms the directory name and preserves a leading dot as its hidden marker; descendants inherit the resulting virtual parent path.

The common template values are `${path}`, `${parent}`, `${name}`, `${stem}`, and `${extension}`. Named captures such as `<year>` provide `${year}`. `${path}` and captures refer to the original source path; `${parent}` refers to the planned parent path. `${name}`, `${stem}`, and `${extension}` refer to the original entry name. `${extension}` omits the dot and is empty when there is no extension. For dotfiles and multiple extensions, the final extension is the suffix after the last dot, except a leading dot alone is not an extension.

Template grammar:

- `${value}` inserts a common path value or named capture.
- `${value|operation}` applies one built-in transformation before insertion.
- `$$` inserts a literal `$`; no other escaping, nesting, or arbitrary code is supported.
- `${path}` and `${parent}` may contain separators. `${name}`, `${stem}`, `${extension}`, and named captures must be a single component; reject values containing `/` or `\\`.
- Template paths use `/` and must be relative. Reject absolute paths and any expanded `.` or `..` component. Literal `/` in the template creates a destination subdirectory.

Reject unknown values, missing captures, multiple filters, and unknown operations. Do not treat malformed placeholders as literal text.

```toml
[[rules]]
name = "Sort dated invoices"
match = "Invoices/<year>-<vendor>-<title>.pdf"
to = "template:Archive/${year}/${vendor|kebab-case}/${title}.pdf"
```

The template filter calls the same operation as `to = "builtin:kebab-case"`; both forms must produce identical output for the same input value.

## Built-in transformations and word splitting

The built-in transformations are `snake-case`, `kebab-case`, `camel-case`, and `pascal-case`. They operate on tokens produced by one shared word splitter:

- Split between a lowercase letter and a following uppercase letter: `MyConfig` becomes `My` + `Config`.
- Split an uppercase run before its final capital when that capital starts a lowercase word: `HTTPServer` becomes `HTTP` + `Server`; `MyXMLParser` becomes `My` + `XML` + `Parser`.
- Split at letter/digit transitions. Treat punctuation as a boundary and normalize it to the selected output style.
- Recognize exact acronym spellings from the configured lexicon at token boundaries before applying generic case boundaries.

The `[acronyms]` table has an optional `entries` list, empty by default. Add exact, non-empty alphabetic spellings when a project needs a mixed-case boundary that generic case rules cannot infer, such as `iOS` in `iOSGuide`. Entries apply to every built-in transformation and template filter. Duplicate entries are rejected; longer entries are matched first and only at token boundaries. They do not define match patterns or destinations.

There is no bundled acronym dictionary. Generic case rules already recognize uppercase runs such as `HTTP` and `XML`; project entries cover ambiguous mixed-case spellings. The lexicon does not infer product-specific capitalization or perform dictionary-based word segmentation.

## Language-server reference updates

`lsp.reference_updates` is a project-wide policy with values `required`, `best-effort`, and `off`; it defaults to `required`. It applies to file extensions mapped by configured servers; other paths remain ordinary path-only renames. `required` blocks planning when a matching server cannot start, does not advertise rename support, or returns an unsupported or invalid edit. `best-effort` keeps the path plan and reports the failure as a warning. `off` skips server requests. The policy applies to all configured servers; per-server overrides are not supported.

The optional `lsp.servers` list configures one executable and maps its file extensions to LSP document language identifiers. For example, one TypeScript server can map `ts` to `typescript` and `tsx` to `typescriptreact`. `command` may be an executable name resolved through `PATH`, an absolute path, or a path relative to the configuration file. `args` are passed as separate process arguments; Dirule does not invoke a shell. `root_markers` are relative paths searched upward from each renamed path to identify a language-server workspace root. Dirule groups rename requests by that root, which supports several language projects under one target. When no marker is found, the operation target is used. A file extension may be assigned to only one configured server.

`dirule lsp status` loads the selected project config or preset and reports whether each executable can be found. It does not start the server or send LSP requests. During `plan`, Dirule starts each matching server, opens the scanned source files that are being renamed, sends `workspace/willRenameFiles`, validates the returned workspace edit, and includes document changes in the preview. Opening renamed files lets servers such as TypeScript initialize project state before searching for references; only files Dirule scanned are opened. The configured server receives their text through its local stdio process, so configure a server command you trust. `apply` applies the edits together with the path changes and records both in the journal. `undo` restores both paths and document contents, and refuses to overwrite document changes made after the apply.

Configs migrate from 1.4 to 1.5 with `lsp.reference_updates = "required"` and an empty `lsp.servers` list.

Output styles are defined as follows:

- `snake-case` lowercases tokens and joins them with `_`.
- `kebab-case` lowercases tokens and joins them with `-`.
- `camel-case` lowercases the first token and title-cases subsequent tokens without separators.
- `pascal-case` title-cases every token and joins them without separators.

Punctuation runs create one separator; leading and trailing punctuation do not create empty tokens. Unicode case conversion and case-boundary detection use Unicode properties supported by the implementation. Scripts without case retain their letters, while punctuation and supported letter/digit transitions follow the same rules. Preserve a dotfile's leading dot as its hidden-file marker while transforming its remaining stem.

## Presets

A preset is a collection of ordinary rules, not a different rule format. A built-in preset is ready-to-use configuration; a user preset uses the same schema and behavior. A built-in transformation is one possible `to` value, not a preset.

Built-in choices must resolve to ordinary editable rule values. They must not rely on hidden matching or transformation behavior that user-authored rules cannot express.

User presets are TOML files named `<name>.toml` in the platform user configuration directory under `dirule/presets/`: `$XDG_CONFIG_HOME` or `$HOME/.config` on Unix, `$HOME/Library/Application Support` on macOS, and `%APPDATA%` on Windows. Configuration base paths must be absolute; ignore a relative XDG path and use an absolute home-directory fallback, or report an error if no absolute base is available. Names contain only ASCII letters, digits, `_`, or `-`. Preset names must be unique across built-in and user presets; report ambiguity instead of choosing silently. The built-in preset catalog may initially be empty.

`dirule presets list` lists the available names. `--format json` emits a versioned JSON document for scripts.

`dirule presets save NAME` validates the current directory's `dirule.toml` and saves its contents as a user preset. `--config FILE` selects a different source file, resolved from the current working directory when relative. Saving does not modify the source configuration and must fail if a preset with that name already exists; users must choose another name rather than silently replacing a preset. A saved preset is loaded with `--preset NAME`.

`dirule presets export NAME` validates a saved user preset and writes its original TOML document to stdout without other output. Users can redirect that output to share or back up the preset, then save it on another machine with `dirule presets save NAME --config FILE`. Exporting an invalid or missing preset fails without writing TOML to stdout.

`dirule presets remove NAME` removes a user preset after an interactive confirmation. In a non-interactive run, require `--yes`. It must not remove built-in presets.

## Target and config selection

- `target` defaults to `.`. An explicit CLI `--target` overrides the configured value.
- Relative targets and explicit `--config FILE` paths are resolved from the directory where Dirule is run. A target equal to `~` or beginning with `~/` (or `~\\` on Windows) expands against the user's home directory, without shell evaluation. Other environment-variable and command substitutions are not performed.
- `--config FILE` and `--preset NAME` are mutually exclusive and take precedence over discovery. Each loads exactly the selected source; neither is merged with discovered configuration.
- Without either option, the chosen directory is the explicit `--target`, or the current working directory when `--target` is absent. First load `dirule.toml` directly inside that directory, if present. Do not search parent directories. A present but unreadable or invalid local config is an error; do not silently fall back to a preset.
- If the chosen directory has no local config, find user presets whose explicitly configured `target` resolves to that same directory. Only absolute paths and supported `~` paths are target bindings; relative preset targets such as `.` are not global bindings. Compare resolved paths so symlink aliases to the same existing directory match. A single match is selected. Multiple matches are an actionable ambiguity error; no matches report that configuration must be selected or created. A preset that cannot be parsed enough to read its `target` is not an implicit-match candidate; selecting it explicitly reports its configuration error. Discovered configurations are never merged.
- `validate` has no `--target`, so its implicit discovery uses the current working directory. An explicitly selected preset may use its own configured target; an explicit CLI `--target` overrides that preset target for `plan`, `check`, and `apply`.

`dirule validate` loads and compiles the selected configuration without scanning or requiring its target to exist. It accepts the same `--config FILE` and `--preset NAME` options and uses the same current-directory default and source-selection errors. It succeeds only when parsing, migrations, rule validation, and compilation succeed. Human output identifies the selected source and rule count. `--format json` emits a versioned object with `valid`, `source` (`kind` and `name`), and `rule_count` fields. Validation never changes the configuration, target, or user state.

## Evaluation and filesystem safety

- Scan files and directories below the selected target; do not rename the target directory itself.
- Apply configured `ignore` patterns and VCS ignore filters before matching. Configured `ignore` patterns always exclude matching source paths, including Git-tracked paths. In Git repositories, respect nested `.gitignore` files, `.git/info/exclude`, and the configured global Git ignore file. Git-tracked paths remain eligible despite VCS ignore rules; VCS ignore rules exclude untracked paths. Obtain tracked paths from the Git index and combine them with the ignore-aware filesystem walk. Outside Git, support `.ignore` files with the same pattern syntax. A `.diruleignore` file adds project-specific exclusions to tracked and untracked paths; it cannot include VCS-ignored paths or negate configured `ignore` patterns.
- If the Git executable is unavailable inside a Git repository, fail planning instead of silently omitting tracked paths.
- Do not hardcode exclusions such as `node_modules` or `build`. Do not exclude hidden files just for being hidden.
- Never scan Git metadata entries named `.git`, whether a directory or a linked-worktree metadata file, at any depth. Reject a target equal to or inside a `.git` metadata path.
- Match all rules against original paths in declaration order. The first matching rule owns an entry; later rules cannot claim it based on its planned destination. Unmatched entries retain their paths.
- Build one virtual plan from all matched entries before changing the filesystem. Do not apply a rule and rescan before evaluating the next rule.
- A template provides a complete target-relative destination. Entries without their own destination inherit a renamed directory's virtual path; an explicit child template destination is used as written.
- Create missing destination parents only during apply. There is no standalone directory-creation rule in the initial action set.
- `plan` and `check` are read-only. `plan` reports proposed operations, findings, and conflicts. `check` succeeds only when every apply-mode path is unchanged, every check-mode rule's expected destination is satisfied, and the plan has no conflicts. Informational findings alone, such as a skipped symlink, do not fail it. `apply` executes the same validated plan with apply-time revalidation and never executes a check-mode rule's expected rename.
- Never overwrite an existing path. An existing destination is available when another active planned move vacates that path; Dirule stages dependent sources through unique sibling paths so move chains and cycles can complete without overwriting. Other existing destinations, duplicate planned destinations, stale sources, and failed dependencies are conflicts. Apply aborts on conflicts by default. `--on-conflict skip` may apply independent operations and reports skipped conflicts. If a move is skipped, skip moves that depend on its destination becoming free. If a directory move is skipped, skip dependent descendant operations. If multiple entries claim one destination, skip every conflicting claim.
- Support case-only renames on case-insensitive filesystems by moving through a unique sibling path with no-replace operations. If the second move fails, restore the original path and report any rollback failure.
- Revalidate the target directory identity, source identities, and destination availability during apply. Reject a target directory replaced since scanning before making any changes.
- If a filesystem operation fails after apply has moved any entries, roll completed moves back in reverse order and remove destination directories created by that apply. If rollback cannot restore the original state, report each rollback failure alongside the original operation error.
- Skip symlinks and never follow them. Report skipped links; explicit symlink operations need a separate design.
- Skip and report entries whose names cannot be represented as valid Unicode config paths; never create a lossy destination.
- On Windows, reject destination components with reserved characters or control characters, trailing spaces or periods, and reserved DOS device names instead of letting Win32 reinterpret them.
- If an ignore file cannot be read or parsed, fail planning with its path and error instead of using a partial file list.
- Top-level `ignore` patterns use the same target-relative glob grammar as rule patterns, must be non-empty and valid, and match original paths. A matching directory excludes its descendants. Legacy configs without `ignore` behave as if the list were empty.

For example, if a specific `HTTPServer.pdf` rule precedes a general `**/*.pdf` rule, the first rule owns the original path. Its planned output is not reconsidered by later rules. A file such as `MyReport.pdf` can still be owned by the general rule.

## Boundaries

LSP edits are limited to regular text files inside the selected target. Resource operations and edits outside the target are rejected. Dynamic imports, reflection, generated code, and non-source references are only updated when the configured language server returns edits for them. Review the plan before applying; language servers cannot promise to find every semantic reference.

The configuration is shared by the CLI and any future frontend. Matching, transformation, and planning must have consistent semantics regardless of which frontend loads it.

## Acceptance cases

Tests should verify:

- Valid configuration parsing; useful errors for missing or malformed fields, patterns, captures, templates, and transformation names.
- Migration of a document through a complete sequence of format migrations, including meaning preservation; rejection of unknown future versions and missing migration steps.
- Root-level and nested glob matching, including `**/` matching zero directories, named captures, and safe template expansion.
- File and directory kind filters, including later rules matching entries whose kind did not match an earlier rule.
- Rule exclusions, including fall-through to later rules, unmatched entries remaining unchanged, and rejection of malformed exclusion patterns.
- Check-mode rules report expected destinations without applying them, retain first-match ownership, and leave descendants under check-only directories at their actual paths.
- All four transformations for ordinary names, uppercase runs, letter/digit transitions, punctuation, Unicode cased letters, scripts without case, dotfiles, and multiple extensions.
- Built-in directory transformations preserve leading dots on `.github`, `.config`, and nested hidden directories.
- Acronym behavior for generic uppercase runs such as `HTTPServer`, configured mixed-case spellings such as `iOSGuide`, longest-first tokenization, and non-matching inside a larger token.
- Identical transformation output from built-in `to` values and equivalent template filters.
- First-match ownership against original paths and one combined plan detecting collisions across rules before any filesystem change.
- Directory rename propagation, explicit child destinations, missing destination-parent dependencies, conflict abort/skip behavior, and unrelated moves continuing when a dependency chain is skipped.
- File and directory move chains and cycles, including sources staged through unique sibling paths without overwriting existing paths.
- Case-only file and directory renames on case-insensitive filesystems, including descendants under renamed directories.
- Runtime failure after an earlier move restores completed moves and removes directories created by apply; incomplete rollback is reported with its failed paths.
- Apply rejects a source replaced since planning even when the replacement has the same length and modification time.
- Git ignore behavior for nested rules, tracked paths, global excludes, `.ignore` outside Git, and hidden files; unreadable or malformed ignore files must stop planning.
- `.diruleignore` exclusions apply to tracked and untracked paths; negation patterns fail validation, and VCS-ignored untracked paths stay excluded.
- Configured `ignore` patterns exclude matching files and whole directory subtrees even when their paths are Git-tracked or not listed in any VCS ignore file; unmatched paths still follow normal VCS ignore behavior.
- Git metadata entries are skipped at any depth, ordinary hidden project files remain eligible, and targets equal to or inside `.git` metadata are rejected.
- Explicit source precedence; target-local config selection; exact absolute or home-relative preset-target matches; ambiguity errors; relative preset targets not acting as bindings; explicit target override; no parent-directory config search.
- Configuration validation accepts a valid config whose target does not exist, rejects invalid config and preset sources, and does not scan or mutate the target.
- `check` succeeds when paths satisfy the rules and fails for a proposed path change or conflict without changing the filesystem.
- `check` exits with status `0` when paths satisfy the rules, status `1` when a proposed change or conflict is found, and status `2` when an error prevents evaluation.
