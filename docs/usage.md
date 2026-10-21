# Usage

## Preview and apply configured rules

`plan` shows the changes a selected configuration would make. `check` succeeds when paths comply and reports failure when changes or conflicts are found, without changing files. `apply` previews again, asks for confirmation, and applies the approved plan.

```sh
dirule plan
dirule check
dirule apply
```

Use `--config FILE` or `--preset NAME` to select a configuration explicitly, and `--target PATH` to choose the directory. `--yes` skips the apply prompt for non-interactive use; it does not bypass conflict or safety checks. Use `--on-conflict skip` to apply unrelated safe moves while skipping conflicts and moves that depend on them. Dirule never overwrites an existing path.

## Sort and rename without a project config

Quick operations preview by default and do not load `dirule.toml` or a preset. Add `--apply` to request changes; Dirule asks for confirmation unless `--yes` is also supplied.

```sh
dirule categories list
dirule sort --target ~/Downloads
dirule sort pdf --target ~/Downloads
dirule sort images videos documents --target ~/Downloads
dirule sort --by extension --target ~/Downloads --recursive
dirule rename kebab-case --target src/
```

`sort` accepts multiple extensions or categories. With no selectors it previews every category in the effective catalog, including user categories and built-in overrides. Extensions match case-insensitively; direct files are selected by default, and `--recursive` includes nested files. Files that match no category stay in place. The built-in categories are `images`, `videos`, `audio`, and `documents`; they match explicit extension lists, not MIME types or file contents, and route files to their configured folders. `sort --by extension` places each file with a final extension in its own bucket. `rename` transforms file and directory names with `snake-case`, `kebab-case`, `camel-case`, or `pascal-case`; `--recursive` includes nested entries.

Users can add categories or replace built-in category definitions with `categories.toml` in Dirule's user config directory:

- Linux and other Unix systems: `$XDG_CONFIG_HOME/dirule/categories.toml`, falling back to `~/.config/dirule/categories.toml`.
- macOS: `~/Library/Application Support/dirule/categories.toml`.
- Windows: `%APPDATA%/dirule/categories.toml`.

For example:

```toml
version = "1.0"

[[categories]]
name = "ebooks"
extensions = ["epub", "mobi"]
destination = "reading/books"
```

See the [custom download categories example](../examples/download-categories/README.md) for a ready-to-adapt catalog and commands.

Category names and destinations are validated before scanning. Defining a built-in name replaces that category's extensions and destination. See the [quick operations specification](specs/quick-operations.md) for exact matching, recursive behavior, and category rules.

Dirule changes paths only; it does not rewrite file contents or update references in source code.

## Presets

Save a project config as a reusable user preset, then select it for another target:

```sh
dirule presets save downloads
dirule plan --preset downloads --target ~/Downloads
dirule presets list
dirule presets export downloads > downloads.toml
dirule presets remove downloads
```

By default, `presets save` reads `dirule.toml` from the current directory. Use `--config FILE` to save another config. To apply a preset to a different folder, pass `--target PATH`; this overrides the target stored in the preset. To import a preset on another machine, export it and save the TOML file there with `dirule presets save downloads --config downloads.toml`. Existing presets are not overwritten. Removing a preset asks for confirmation; use `--yes` in a non-interactive run.

## Review and undo applied runs

Every successful apply records filesystem moves in Dirule's user state directory and prints a run ID. Inspect and undo a run with:

```sh
dirule runs list
dirule runs show RUN_ID
dirule undo RUN_ID
```

Undo asks for confirmation; use `--yes` in a non-interactive session. Dirule refuses to overwrite a path or undo a file whose recorded identity has changed. It removes only empty destination-parent directories that the run created. The journal stores paths and filesystem metadata, not file contents, so it is recovery history rather than a backup. See the [journal specification](specs/dirule-journal.md) for recovery details.

## Output for scripts

Commands that support `--format` accept `human` or `json`. JSON output is versioned and intended for scripts. Use `--color auto|always|never` to control terminal colors; automatic color also honors `NO_COLOR` and `TERM=dumb`.

See the [CLI output specification](specs/cli-output.md) for schemas, stable error codes, and exit behavior.
