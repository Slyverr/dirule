# Dirule

Dirule is a local-first CLI for applying reusable rules to file and directory names. It can check a target, preview planned renames, and apply an approved plan. Rules are stored in a project's `dirule.toml`; Dirule changes paths, not file contents.

## Development status

Dirule is under active development. Its configuration format and command behavior may change.

## Install

From the repository root, install the current checkout with Cargo:

```sh
cargo install --path crates/dirule --locked
```

Make sure Cargo's binary directory is on `PATH` to run `dirule` from any project.

After the repository is public and a release is published, install the latest version with the shell installer on Linux or macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Slyverr/dirule/releases/latest/download/dirule-installer.sh | sh
```

On Windows, run the PowerShell installer:

```powershell
powershell -c "irm https://github.com/Slyverr/dirule/releases/latest/download/dirule-installer.ps1 | iex"
```

Once the repository is public or your Nix installation is configured to access it, run or install Dirule from GitHub:

```sh
nix run github:Slyverr/dirule -- --help
nix profile install github:Slyverr/dirule
```

The shell installer does not support NixOS; use the flake there. Releases also include archives for manual installation.

## Quick start

From a project checkout, create the starter configuration and preview it:

```sh
cargo run -p dirule -- init
cargo run -p dirule -- plan
cargo run -p dirule -- check
cargo run -p dirule -- apply
```

`apply` asks for confirmation in an interactive terminal. Use `--yes` for a non-interactive run. `plan`, `check`, and `apply` accept `--target PATH` and either `--config FILE` or `--preset NAME`. Use `--format json` for structured output, or `--verbose` to show the rule responsible for each rename and identify paths inherited from a renamed directory.

Use `--color auto|always|never` to control terminal colors. Automatic color follows terminal support and honors non-empty `NO_COLOR` and `TERM=dumb`; an explicit `--color` choice takes precedence.

With `apply --format json`, the result stays on stdout. When confirmation is required, Dirule writes the human-readable plan preview to stderr.

Targets equal to `~` or beginning with `~/` expand against the user's home directory (`~\\` is also accepted on Windows). No shell or environment-variable expansion is performed.

JSON plans include the canonical target path; their source and destination paths are target-relative and use `/` separators on every platform.

## Rules

`dirule init` creates a starter `dirule.toml`. Put project-specific rules before its general `**/*` rule: rules run in order, and the first match owns each original path.

```toml
version = "1.0"
target = "."

[[rules]]
name = "Archive invoices"
match = "Invoices/<year>-<vendor>-<title>.pdf"
to = "template:Archive/${year}/${vendor|kebab-case}/${title|kebab-case}.pdf"

[[rules]]
name = "Normalize other names"
match = "**/*"
to = "builtin:kebab-case"
```

`<year>` and the other named captures come from the matched path; `${year}` inserts a capture in a `template:` destination. `builtin:` applies a built-in case transformation to the entry name. See the [configuration specification](docs/specs/dirule-config.md) for matching, templates, and path behavior.

## Apply history

Each successful apply records its filesystem moves in Dirule's user state directory and prints a run ID. List runs and undo a run with:

```sh
dirule runs list
dirule runs show RUN_ID
dirule undo RUN_ID
```

`runs show` displays the target and renames recorded for a run. Undo asks for confirmation; use `--yes` in a non-interactive session. Dirule refuses to overwrite a path or undo a file whose recorded identity has changed. Non-empty directories created during apply are kept and reported after undo. The journal stores paths and filesystem metadata, not file contents, so it is recovery history rather than a backup.

## Presets

Save a project's validated rules as a reusable user preset, then select it for another project:

```sh
dirule presets save downloads
dirule presets remove downloads
dirule plan --preset downloads
dirule presets list --format json
```

By default, `presets save` reads `dirule.toml` from the current directory. Pass `--config FILE` to save another configuration. Presets are stored in Dirule's platform-specific user configuration directory, and an existing preset is never overwritten. Preset listings can be emitted as versioned JSON. Removing a preset asks for confirmation; pass `--yes` for a non-interactive run.

## Shell completions

Generate a completion script from the current CLI definition with `dirule completions SHELL`. Supported shells are Bash, Elvish, Fish, PowerShell, and Zsh. Redirect the output to the completion location used by your shell.

## Nix

Run Dirule from the checkout with `nix run . -- --help`, enter the development shell with `nix develop`, or install it to your user profile with `nix profile add .`. The GitHub flake commands in [Install](#install) require public repository access or configured GitHub authentication.

The flake supports x86_64 and ARM64 Linux, plus ARM64 macOS. On Intel macOS, install from source with Cargo or use a release archive.

## Workspace

- `crates/dirule`: CLI and application orchestration.
- `crates/dirule-core`: configuration, matching, transforms, and planning.
- `crates/dirule-fs`: ignore-aware scanning and filesystem operations.

The workspace uses Rust 1.88 or newer, Rust edition 2024, and Cargo resolver 3. See [Contributing](CONTRIBUTING.md) for development checks and commit conventions.

## License

Dirule is licensed under the [MIT License](LICENSE).
