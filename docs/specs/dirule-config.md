# Dirule configuration 1.0

## Purpose and scope

This specification defines the project configuration file, rule format, and behavior shared by `plan`, `check`, and `apply`. The file is named `dirule.toml` and uses TOML. A configuration can combine built-in transformations with custom path matching and destination templates.

The format must be able to grow through explicit migrations. New matchers, transformations, and filesystem actions may be added without silently changing the meaning of existing configurations.

## Format evolution

Every configuration declares `version` as a quoted `major.minor` string. The reader parses that field first, migrates older documents through a complete sequence of registered one-step migrations, then validates and loads the current schema. The runtime configuration does not carry format-version branches.

A migration must preserve the document's meaning, update its version, and produce a document accepted by the next migration. Readers reject unknown future versions and any older version without a complete migration path; they never skip versions or guess how to interpret fields. Changes that preserve existing meaning may advance the minor component. Changes that alter existing meaning require a major component change and an explicit migration policy.

## Configuration shape

`name` is optional and labels the configuration. `target` is optional and defaults to `.`. `rules` is an ordered, non-empty array. The optional `acronyms` table lists project-specific spellings for the word splitter used by built-in case transformations and template filters.

```toml
version = "1.0"
name = "Downloads"
target = "."

[acronyms]
entries = ["iOS", "eBay"]

[[rules]]
name = "Sort invoices by year and vendor"
match = "Invoices/<year>-<vendor>-<title>.pdf"
to = "template:Archive/${year}/${vendor}/${title|kebab-case}.pdf"

[[rules]]
name = "Normalize remaining filenames"
match = "**/*"
to = "builtin:kebab-case"
```

Rules are evaluated in declaration order. The first rule matching an entry owns it. The parser splits `to` at its first colon, so colons later in a value remain part of that value. Unknown fields, namespaces, transformations, malformed patterns, and invalid templates are configuration errors with the relevant field or rule identified.

## Matching

Patterns match the complete path relative to the selected target. `/` is the separator in configuration patterns on every platform. Matching is case-sensitive. The supported glob syntax is:

| Syntax | Meaning |
| --- | --- |
| `*` | Zero or more characters within one path component. |
| `?` | One character within one path component. |
| `[abc]`, `[a-z]` | One character from a set or range. |
| `[!abc]` | One character outside a set. |
| `{jpg,png}` | One alternative; nested alternatives are unsupported. |
| `**` | Zero or more path components. |
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

A built-in transformation changes the entry name and preserves its planned parent. For files, it transforms the stem and preserves the extension. For directories, it transforms the directory name; descendants inherit the resulting virtual parent path.

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

The initial built-in transformations are `snake-case`, `kebab-case`, and `camel-case`. They operate on tokens produced by one shared word splitter:

- Split between a lowercase letter and a following uppercase letter: `MyConfig` becomes `My` + `Config`.
- Split an uppercase run before its final capital when that capital starts a lowercase word: `HTTPServer` becomes `HTTP` + `Server`; `MyXMLParser` becomes `My` + `XML` + `Parser`.
- Split at letter/digit transitions. Treat punctuation as a boundary and normalize it to the selected output style.
- Recognize exact acronym spellings from the configured lexicon at token boundaries before applying generic case boundaries.

The `[acronyms]` table has an optional `entries` list, empty by default. Add exact, non-empty alphabetic spellings when a project needs a mixed-case boundary that generic case rules cannot infer, such as `iOS` in `iOSGuide`. Entries apply to every built-in transformation and template filter. Duplicate entries are rejected; longer entries are matched first and only at token boundaries. They do not define match patterns or destinations.

There is no bundled acronym dictionary. Generic case rules already recognize uppercase runs such as `HTTP` and `XML`; project entries cover ambiguous mixed-case spellings. The lexicon does not infer product-specific capitalization or perform dictionary-based word segmentation.

Output styles are defined as follows:

- `snake-case` lowercases tokens and joins them with `_`.
- `kebab-case` lowercases tokens and joins them with `-`.
- `camel-case` lowercases the first token and title-cases subsequent tokens without separators.

Punctuation runs create one separator; leading and trailing punctuation do not create empty tokens. Unicode case conversion and case-boundary detection use Unicode properties supported by the implementation. Scripts without case retain their letters, while punctuation and supported letter/digit transitions follow the same rules. Preserve a dotfile's leading dot as its hidden-file marker while transforming its remaining stem.

## Presets

A preset is a collection of ordinary rules, not a different rule format. A built-in preset is ready-to-use configuration; a user preset uses the same schema and behavior. A built-in transformation is one possible `to` value, not a preset.

Built-in choices must resolve to ordinary editable rule values. They must not rely on hidden matching or transformation behavior that user-authored rules cannot express.

User presets are TOML files named `<name>.toml` in the platform user configuration directory under `dirule/presets/`: `$XDG_CONFIG_HOME` or `$HOME/.config` on Unix, `$HOME/Library/Application Support` on macOS, and `%APPDATA%` on Windows. Names contain only ASCII letters, digits, `_`, or `-`. Preset names must be unique across built-in and user presets; report ambiguity instead of choosing silently. The built-in preset catalog may initially be empty.

`dirule presets save NAME` validates the current directory's `dirule.toml` and saves its contents as a user preset. `--config FILE` selects a different source file, resolved from the current working directory when relative. Saving does not modify the source configuration and must fail if a preset with that name already exists; users must choose another name rather than silently replacing a preset. A saved preset is loaded with `--preset NAME`.

`dirule presets remove NAME` removes a user preset after an interactive confirmation. In a non-interactive run, require `--yes`. It must not remove built-in presets.

## Target and config selection

- `target` defaults to `.`. An explicit CLI `--target` overrides the configured value.
- Relative targets and explicit `--config FILE` paths are resolved from the directory where Dirule is run.
- `--config FILE` and `--preset NAME` are mutually exclusive. Each loads exactly the selected source; neither is merged with discovered configuration.
- Without either option, load `dirule.toml` from the current working directory only. Do not search parent directories. If absent, report an error that points to `init`, `--config`, and `--preset`.

## Evaluation and filesystem safety

- Scan files and directories below the selected target; do not rename the target directory itself.
- Apply VCS ignore filters before matching. In Git repositories, respect nested `.gitignore` files, `.git/info/exclude`, and the configured global Git ignore file. Git-tracked paths remain eligible even if ignored; ignore rules exclude untracked paths. Obtain tracked paths from the Git index and combine them with the ignore-aware filesystem walk. Outside Git, support `.ignore` files with the same pattern syntax.
- Do not hardcode exclusions such as `node_modules` or `build`. Do not exclude hidden files just for being hidden.
- Match all rules against original paths in declaration order. The first matching rule owns an entry; later rules cannot claim it based on its planned destination. Unmatched entries retain their paths.
- Build one virtual plan from all matched entries before changing the filesystem. Do not apply a rule and rescan before evaluating the next rule.
- A template provides a complete target-relative destination. Entries without their own destination inherit a renamed directory's virtual path; an explicit child template destination is used as written.
- Create missing destination parents only during apply. There is no standalone directory-creation rule in the initial action set.
- `plan` and `check` are read-only. `plan` reports proposed operations, findings, and conflicts. `check` succeeds only when every final planned path equals its original path and the plan has no conflicts. Informational findings alone, such as a skipped symlink, do not fail it. `apply` executes the same validated plan with apply-time revalidation.
- Never overwrite an existing path. Existing destinations, duplicate planned destinations, stale sources, and failed dependencies are conflicts. Apply aborts on conflicts by default. `--on-conflict skip` may apply independent operations and reports skipped conflicts. If a directory move is skipped, skip dependent descendant operations. If multiple entries claim one destination, skip every conflicting claim.
- Support case-only renames on case-insensitive filesystems by moving through a unique sibling path with no-replace operations. If the second move fails, restore the original path and report any rollback failure.
- Revalidate source identity and destination availability during apply.
- If a filesystem operation fails after apply has moved any entries, roll completed moves back in reverse order and remove destination directories created by that apply. If rollback cannot restore the original state, report each rollback failure alongside the original operation error.
- Skip symlinks and never follow them. Report skipped links; explicit symlink operations need a separate design.
- Skip and report entries whose names cannot be represented as valid Unicode config paths; never create a lossy destination.
- If an ignore file cannot be read or parsed, fail planning with its path and error instead of using a partial file list.

For example, if a specific `HTTPServer.pdf` rule precedes a general `**/*.pdf` rule, the first rule owns the original path. Its planned output is not reconsidered by later rules. A file such as `MyReport.pdf` can still be owned by the general rule.

## Boundaries

A rename changes filesystem paths only. It must not silently rewrite references inside source files. Language-aware reference updates through editor LSP clients require a separate integration and preview design.

The configuration is shared by the CLI and any future frontend. Matching, transformation, and planning must have consistent semantics regardless of which frontend loads it.

## Acceptance cases

Tests should verify:

- Valid configuration parsing; useful errors for missing or malformed fields, patterns, captures, templates, and transformation names.
- Migration of a document through a complete sequence of format migrations, including meaning preservation; rejection of unknown future versions and missing migration steps.
- Root-level and nested glob matching, including `**/` matching zero directories, named captures, and safe template expansion.
- All three transformations for ordinary names, uppercase runs, letter/digit transitions, punctuation, Unicode cased letters, scripts without case, dotfiles, and multiple extensions.
- Acronym behavior for generic uppercase runs such as `HTTPServer`, configured mixed-case spellings such as `iOSGuide`, longest-first tokenization, and non-matching inside a larger token.
- Identical transformation output from built-in `to` values and equivalent template filters.
- First-match ownership against original paths and one combined plan detecting collisions across rules before any filesystem change.
- Directory rename propagation, explicit child destinations, missing destination-parent dependencies, and conflict abort/skip behavior.
- Case-only file and directory renames on case-insensitive filesystems, including descendants under renamed directories.
- Runtime failure after an earlier move restores completed moves and removes directories created by apply; incomplete rollback is reported with its failed paths.
- Git ignore behavior for nested rules, tracked paths, global excludes, `.ignore` outside Git, and hidden files; unreadable or malformed ignore files must stop planning.
- Explicit target override, configured target, current-directory default, exact config/preset selection, and no parent-directory config search.
- `check` succeeds when paths satisfy the rules and fails for a proposed path change or conflict without changing the filesystem.
