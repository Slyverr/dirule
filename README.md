# Dirule

Dirule is a local-first CLI tool planned for defining reusable rules over file and directory paths. Its intended workflow is to check whether paths follow those rules, preview required changes, and apply an approved plan.

## Status

Dirule is at its initial workspace scaffold. The binary crate currently prints a placeholder message; configuration, rule evaluation, planning, and filesystem changes are not implemented yet.

## Workspace

- `crates/dirule`: binary entry point.
- `crates/dirule-core`: placeholder for shared rule logic.
- `crates/dirule-fs`: placeholder for filesystem operations; it will depend on `dirule-core`.

The workspace uses Rust edition 2024 and Cargo resolver 3.

## Development

See [Contributing](CONTRIBUTING.md) for workspace checks and commit conventions.

## License

Dirule is licensed under the [MIT License](LICENSE).
