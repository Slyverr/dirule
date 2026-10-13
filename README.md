# Dirule

Dirule is a local-first CLI for applying reusable rules to file and directory names. It can check a target, preview planned renames, and apply an approved plan. Rules are stored in a project's `dirule.toml`; Dirule changes paths, not file contents.

## Development status

Dirule is under active development. Its configuration format and command behavior may change.

## Quick start

From a project checkout, create the starter configuration and preview it:

```sh
cargo run -p dirule -- init
cargo run -p dirule -- plan
cargo run -p dirule -- check
cargo run -p dirule -- apply
```

`apply` asks for confirmation in an interactive terminal. Use `--yes` for a non-interactive run. `plan`, `check`, and `apply` accept `--target PATH` and either `--config FILE` or `--preset NAME`. Use `--format json` for structured output.

## Nix

Run Dirule from the checkout with `nix run . -- --help`, enter the development shell with `nix develop`, or install it to your user profile with `nix profile add .`.

## Workspace

- `crates/dirule`: CLI and application orchestration.
- `crates/dirule-core`: configuration, matching, transforms, and planning.
- `crates/dirule-fs`: ignore-aware scanning and filesystem operations.

The workspace uses Rust edition 2024 and Cargo resolver 3. See [Contributing](CONTRIBUTING.md) for development checks and commit conventions.

## License

Dirule is licensed under the [MIT License](LICENSE).
