# Dirule

Dirule is a local-first CLI for applying reusable rules to file and directory names. It can check a target, preview planned renames, and apply an approved plan. Rules are stored in a project's `dirule.toml`; Dirule changes paths, not file contents.

## Development status

Dirule is under active development. Its configuration format and command behavior may change.

## Install

After a release is published, install the latest version with the shell installer on Linux or macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Slyverr/dirule/releases/latest/download/dirule-installer.sh | sh
```

On Windows, run the PowerShell installer:

```powershell
powershell -c "irm https://github.com/Slyverr/dirule/releases/latest/download/dirule-installer.ps1 | iex"
```

Nix users can run or install Dirule directly from the flake:

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

`apply` asks for confirmation in an interactive terminal. Use `--yes` for a non-interactive run. `plan`, `check`, and `apply` accept `--target PATH` and either `--config FILE` or `--preset NAME`. Use `--format json` for structured output.

JSON plan paths are target-relative and use `/` separators on every platform.

## Presets

Save a project's validated rules as a reusable user preset, then select it for another project:

```sh
dirule presets save downloads
dirule presets remove downloads
dirule plan --preset downloads
```

By default, `presets save` reads `dirule.toml` from the current directory. Pass `--config FILE` to save another configuration. Presets are stored in Dirule's platform-specific user configuration directory, and an existing preset is never overwritten. Removing a preset asks for confirmation; pass `--yes` for a non-interactive run.

## Nix

Run Dirule from the checkout with `nix run . -- --help`, enter the development shell with `nix develop`, or install it to your user profile with `nix profile add .`.

## Workspace

- `crates/dirule`: CLI and application orchestration.
- `crates/dirule-core`: configuration, matching, transforms, and planning.
- `crates/dirule-fs`: ignore-aware scanning and filesystem operations.

The workspace uses Rust edition 2024 and Cargo resolver 3. See [Contributing](CONTRIBUTING.md) for development checks and commit conventions.

## License

Dirule is licensed under the [MIT License](LICENSE).
