# Dirule

Dirule is a local-first CLI for applying reusable rules to file and directory names. It can check a target, preview planned renames, and apply an approved plan. Rules are stored in a project's `dirule.toml`; Dirule changes paths, not file contents.

## Development status

Dirule is under active development. Its configuration format and command behavior may change.

## Release notes

See [CHANGELOG.md](CHANGELOG.md) for features and fixes included in each version.

## Install

From the repository root, install the current checkout with Cargo:

```sh
cargo install --path crates/dirule --locked
```

Make sure Cargo's binary directory is on `PATH` to run `dirule` from any project.

After a release is published, install the latest version with the shell installer on Linux or macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Slyverr/dirule/releases/latest/download/dirule-installer.sh | sh
```

On Windows, run the PowerShell installer:

```powershell
powershell -c "irm https://github.com/Slyverr/dirule/releases/latest/download/dirule-installer.ps1 | iex"
```

The generated shell and PowerShell installers do not verify checksums. Releases also include archives and SHA-256 checksum files for manual installation. After downloading an archive and its matching `.sha256` file into the same directory, verify it with:

```sh
sha256sum -c dirule-x86_64-unknown-linux-gnu.tar.xz.sha256
```

On Windows, compare the archive's checksum with the value in its `.sha256` file using `Get-FileHash -Algorithm SHA256`.

## Quick start

From a project checkout, create the starter configuration and preview it:

```sh
cargo run -p dirule -- init
cargo run -p dirule -- validate
cargo run -p dirule -- plan
cargo run -p dirule -- check
cargo run -p dirule -- apply
```

`validate` checks the selected config and compiles its rules without scanning the target. `apply` asks for confirmation in an interactive terminal. Use `--yes` for a non-interactive run. `plan`, `check`, and `apply` accept `--target PATH` and either `--config FILE` or `--preset NAME`; `validate` accepts the config or preset options without `--target`. Use `--format json` for structured output, or `--verbose` to show the rule responsible for each rename and identify paths inherited from a renamed directory.

Use `--color auto|always|never` to control terminal colors. Automatic color follows terminal support and honors non-empty `NO_COLOR` and `TERM=dumb`; an explicit `--color` choice takes precedence.

With `apply --format json`, the result stays on stdout. When confirmation is required, Dirule writes the human-readable plan preview to stderr.

Targets equal to `~` or beginning with `~/` expand against the user's home directory (`~\\` is also accepted on Windows). No shell or environment-variable expansion is performed.

JSON plans include the canonical target path; their source and destination paths are target-relative and use `/` separators on every platform.

Without `--config` or `--preset`, Dirule looks for `dirule.toml` directly inside the selected target (the current directory if `--target` is omitted). If it is absent, Dirule can select one user preset whose absolute or `~/` target resolves to that same directory. Multiple matches are an error. Relative preset targets do not bind globally, and Dirule does not search parent directories. Explicit `--config` and `--preset` always take precedence.

## Quick operations

Sort files or rename entries without creating a configuration. These commands preview changes by default; pass `--apply` to request changes and `--yes` only when a non-interactive run should skip confirmation.

```sh
dirule sort pdf --target ~/Downloads
dirule sort pdf --target ~/Downloads --apply
dirule sort --by extension --target ~/Downloads --recursive
dirule rename kebab-case --target src/
```

`sort` matches extensions without regard to case. Recursive sorting collects nested files into extension buckets under the target; duplicate destination names are reported as conflicts. `rename` uses the built-in case transformations. Dirule changes paths only and does not update references inside files. See the [quick operations specification](docs/specs/quick-operations.md) for the full behavior.

## CI and automation

Run `check` to verify names without changing the target:

```sh
dirule check --format json
```

It exits with status `0` when paths comply, `1` when the plan contains changes or conflicts, and `2` when Dirule cannot evaluate the check. JSON results go to stdout; execution errors are versioned JSON on stderr with stable error codes and categories. See the [CLI output contract](docs/specs/cli-output.md) for the output shapes and stable values.

### Pre-commit

To check the whole project before each commit, add a local hook to `.pre-commit-config.yaml`:

```yaml
minimum_pre_commit_version: "4.4.0"
repos:
  - repo: local
    hooks:
      - id: dirule-check
        name: Check project paths with Dirule
        entry: dirule check --color never
        language: unsupported
        types: [file]
        always_run: true
        pass_filenames: false
```

Install Dirule separately and make sure it is on `PATH`. The hook checks the configured target as a whole; it does not limit the check to staged paths.

## Rules

`dirule init` creates a starter `dirule.toml`. Put project-specific rules before its general `**/*` rule: rules run in order, and the first match owns each original path.

```toml
version = "1.3"
target = "."
ignore = ["generated/api/**"]

[[rules]]
name = "Archive invoices"
match = "Invoices/<year>-<vendor>-<title>.pdf"
kind = "file"
to = "template:Archive/${year}/${vendor|kebab-case}/${title|kebab-case}.pdf"

[[rules]]
name = "Normalize directory names"
match = "**/*"
kind = "directory"
to = "builtin:kebab-case"

[[rules]]
name = "Normalize other file names"
match = "**/*"
kind = "file"
exclude = ["**/*.md"]
to = "builtin:kebab-case"
```

`<year>` and the other named captures come from the matched path; `${year}` inserts a capture in a `template:` destination. `kind` can select `file`, `directory`, or `any`; it defaults to `any`. `exclude` skips paths from that rule, allowing later rules to match them. `builtin:` applies a built-in case transformation to the entry name. Available transformations are `snake-case`, `kebab-case`, `camel-case`, and `pascal-case`. See the [configuration specification](docs/specs/dirule-config.md) for matching, templates, and path behavior.

### Exclusions

Dirule honors project ignore files such as `.gitignore` and `.ignore`, plus Git's repository and global exclude rules. In a Git repository, ignored untracked paths are skipped while tracked paths remain eligible. Outside Git, `.ignore` files still filter the scan. Add a `.diruleignore` file for Dirule-specific exclusions; its standard `!pattern` syntax can include a VCS-ignored path, and its rules apply to tracked and untracked paths. The top-level `ignore` setting is an absolute exclusion for paths Dirule must always skip. A rule's `exclude` setting skips a path only for that rule, so a later rule may still match it.

## Conflict handling

Dirule never overwrites an existing path. `plan` reports conflicts, and `apply` aborts if any are present. To apply independent safe renames while skipping conflicts, use:

```sh
dirule apply --on-conflict skip
```

Skipped paths are reported. If a directory rename is skipped, its dependent descendant renames are skipped too. If multiple paths claim the same destination, all conflicting claims are skipped.

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
dirule plan --preset downloads
dirule presets list --format json
dirule presets export downloads > downloads.toml
dirule presets remove downloads
```

By default, `presets save` reads `dirule.toml` from the current directory. Pass `--config FILE` to save another configuration. To move a preset to another machine, export it to a TOML file and save it there with `dirule presets save downloads --config downloads.toml`. Presets are stored in Dirule's platform-specific user configuration directory, and an existing preset is never overwritten. Preset listings can be emitted as versioned JSON. Removing a preset asks for confirmation; pass `--yes` for a non-interactive run.

## Shell completions

Generate a completion script from the current CLI definition with `dirule completions SHELL`. Supported shells are Bash, Elvish, Fish, PowerShell, and Zsh. Redirect the output to the completion location used by your shell.

For Bash with `bash-completion`, install it for your user with:

```sh
mkdir -p ~/.local/share/bash-completion/completions
dirule completions bash > ~/.local/share/bash-completion/completions/dirule.bash
```

For Fish, use:

```sh
mkdir -p ~/.config/fish/completions
dirule completions fish > ~/.config/fish/completions/dirule.fish
```

If you set `XDG_DATA_HOME` or `XDG_CONFIG_HOME`, use the corresponding directory under that location instead.

## Nix

Run or install Dirule directly from GitHub:

```sh
nix run github:Slyverr/dirule -- --help
nix profile install github:Slyverr/dirule
```

The flake is the installation path for NixOS; the shell installer does not support it. From a checkout, run Dirule with `nix run . -- --help`, enter the development shell with `nix develop`, or install it to your user profile with `nix profile add .`.

The flake supports x86_64 and ARM64 Linux, plus ARM64 macOS. On Intel macOS, install from source with Cargo or use a release archive.

## Workspace

- `crates/dirule`: CLI parsing, output, and command orchestration.
- `crates/dirule-app`: shared application services for the CLI and future interfaces.
- `crates/dirule-core`: configuration, matching, transforms, and planning.
- `crates/dirule-fs`: ignore-aware scanning and filesystem operations.

The workspace uses Rust 1.88 or newer, Rust edition 2024, and Cargo resolver 3. See [Contributing](CONTRIBUTING.md) for development checks and commit conventions.

## License

Dirule is licensed under the [MIT License](LICENSE).
