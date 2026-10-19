# Dirule

Dirule is a local-first CLI for bringing order to file and directory names. Define naming rules once, preview how they affect a project or folder, then apply the changes after review.

It is meant for the cleanup people keep repeating: organizing personal files, keeping project paths consistent, or checking naming conventions in CI. Dirule changes paths, not file contents, so it will not update references inside source code after a rename.

## What it does

- Match paths with ordered glob rules and named captures.
- Build destination paths with templates and built-in case transformations.
- Sort files by extension or category, with support for user-defined categories.
- Check naming conventions without changing files.
- Preview changes, detect conflicts, and refuse to overwrite existing paths.
- Record applied changes so runs can be inspected and safely undone.
- Save and reuse global presets across projects.

## Development status

Dirule is under active development. The current release is `v0.4.0`; configuration and command behavior may change before a stable `1.0` release. See the [changelog](CHANGELOG.md) for release contents.

## Install

Install the current checkout with Cargo:

```sh
cargo install --path crates/dirule --locked
```

Or install the published release with Cargo:

```sh
cargo install --locked --git https://github.com/Slyverr/dirule --tag v0.4.0 dirule
```

NixOS users can install from the flake. Release installers, checksums, platform details, and shell completions are in the [installation guide](docs/installation.md).

## Quick start

If Dirule is not installed yet, see [Install](#install). From the directory you want to manage, create a starter config, preview the changes, and apply them:

```sh
dirule init
dirule validate
dirule plan
dirule apply
dirule check
```

`validate` checks the config without scanning files. `plan` previews proposed changes and `check` verifies that no changes or conflicts remain; neither changes paths. `apply` shows the plan again and asks for confirmation. Use `--yes` only when an automated run should skip that prompt; it does not bypass safety checks. A `check` run before applying will exit with status `1` if the target still needs changes.

## Define naming rules

Rules live in the project's `dirule.toml`. Put specific rules before general rules because the first matching rule owns each original path.

```toml
version = "1.4"
target = "."

[[rules]]
name = "Archive invoices"
match = "Invoices/<year>-<vendor>-<title>.pdf"
kind = "file"
to = "template:Archive/${year}/${vendor|kebab-case}/${title|kebab-case}.pdf"

[[rules]]
name = "Normalize other filenames"
match = "**/*"
kind = "file"
to = "builtin:kebab-case"
```

The glob captures `<year>`, `<vendor>`, and `<title>` provide values for the destination template. Built-in transformations include `snake-case`, `kebab-case`, `camel-case`, and `pascal-case`; project-specific acronyms can be listed in the config. Use `ignore` for paths Dirule must always skip, and `.diruleignore` for additional project-local exclusions. Dirule also respects VCS ignore rules. The [configuration guide](docs/configuration.md) covers matching, targets, transformations, and exclusions.

## Sort common files

For a one-off sort, use a quick operation instead of creating a project config. These commands preview by default:

```sh
dirule categories list
dirule sort --target ~/Downloads
dirule sort images videos audio documents --target ~/Downloads
dirule sort pdf png --target ~/Downloads
dirule sort images --target ~/Downloads --apply
dirule rename kebab-case --target ~/Downloads
```

A bare `sort` previews every category in the effective catalog, including custom categories and built-in overrides. It scans direct files by default; add `--recursive` for nested files. Built-in categories group common extensions into destinations such as `media/images/` and `documents/`. Add categories or customize those defaults with `categories.toml`. Quick operations use the same conflict checks and undo history as configured rules. See [Usage](docs/usage.md#sort-and-rename-without-a-project-config) for custom categories and more examples.

## Ask a model (experimental)

`dirule ask "..."` sends one request to a configured model. Run `dirule ask` in a terminal to start a conversation with follow-up turns, or pipe one request to it for scripting. The assistant can inspect and preview Dirule operations, and can create a config or apply a change after you approve the exact proposal. It does not read file contents or save conversation transcripts. See the [Ask guide](docs/assistant.md) for provider setup, data handling, and session commands.

## Check in CI

Commit `dirule.toml` with the project and run:

```sh
dirule check --format json --color never
```

The command does not rename anything. It exits `0` when paths comply, `1` when a rename or conflict is found, and `2` when Dirule cannot complete the check. The [CI guide](docs/ci.md) shows how to run the check in GitHub Actions or pre-commit.

## Review and undo

Each successful apply prints a run ID. Inspect or undo that run with:

```sh
dirule runs list
dirule runs show RUN_ID
dirule undo RUN_ID
```

The journal stores paths and filesystem metadata, not file contents. It is recovery history, not a backup. Dirule verifies paths before undoing and will not overwrite a file that has changed or appeared since the run. See [Usage](docs/usage.md#review-and-undo-applied-runs) for preset management and run details.

## Use with coding agents

Dirule includes an [Agent Skill](.agents/skills/dirule/SKILL.md) that helps compatible coding agents configure and use Dirule in a project or personal folder. Run the [Skills CLI](https://github.com/vercel-labs/skills) with `npx` or Bun's `bunx`:

```sh
npx skills add Slyverr/dirule --skill dirule
# or
bunx skills add Slyverr/dirule --skill dirule
```

Add `-g` to make the skill available globally for your agent. The skill provides instructions; install Dirule itself separately using the [installation guide](docs/installation.md).

## Help and contributions

For bugs, questions, or feature ideas, open a [GitHub issue](https://github.com/Slyverr/dirule/issues). Contributions are welcome; see [Contributing](CONTRIBUTING.md) for development checks and project conventions.

## License

Dirule is licensed under the [MIT License](LICENSE).
