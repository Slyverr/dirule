# Repository instructions

Dirule is a Rust workspace for a local-first filesystem rules CLI. Use the [README](README.md) for the human-facing overview and [CONTRIBUTING.md](CONTRIBUTING.md) for contribution and commit conventions.

## Source of truth

- Follow the user's current instructions and preserve unrelated working-tree changes, including staged and experimental work.
- When feature specs are present in `docs/specs/`, follow their explicit decisions; illustrative examples are not additional requirements.
- Source code and tests describe what is implemented. Do not describe planned behavior as available functionality.
- Do not convert a CLI experiment, generated output, or an uncommitted prototype into accepted product direction or a commit without the user's direction.

## Workspace boundaries

- `crates/dirule`: CLI and application orchestration.
- `crates/dirule-core`: configuration, matching, transformation, validation, and virtual planning; keep it independent of filesystem I/O.
- `crates/dirule-fs`: traversal and filesystem operations; depend on `dirule-core` for shared rule data and plans.
- Keep dependencies flowing from `dirule` to the libraries and from `dirule-fs` to `dirule-core`; avoid cycles.

The CLI is Dirule's only user interface at this stage. Prefer portable standard-library behavior; isolate platform-specific filesystem guarantees inside `dirule-fs` when they are required.

## Filesystem behavior

When implementing file changes, calculate and validate a complete plan before mutation. Never overwrite an existing path. Respect the project's ignore rules, skip symlinks unless a later design explicitly supports them, and revalidate sources and destinations when applying a plan. Keep path matching and transformations in `dirule-core`; do not let filesystem traversal determine rule semantics.

## Path checks

- Before changing paths in this repository, inspect the proposed moves with `cargo run -p dirule -- plan --format json`.
- Verify the result with `cargo run -p dirule -- check --format json`; `check` never changes files. Status `0` means the names comply, `1` means the plan contains changes or conflicts, and `2` means Dirule could not evaluate the check. Treat status `2` as an error, not as a pass.
- Dirule changes paths only. Update code references separately and run the relevant project checks afterward.

## Work and validation

Inspect the affected crate and relevant spec before changing behavior. Make the smallest coherent change, keep module boundaries clear, and avoid comments that merely repeat the code. Add dependencies only when they materially simplify or strengthen the implementation.

Run relevant workspace checks when validation is requested or required for the task, and report exactly what ran:

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```
