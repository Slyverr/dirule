# Repository instructions

Dirule is a Rust workspace for a local-first filesystem rules CLI. Use the [README](README.md) for the human-facing overview and [CONTRIBUTING.md](CONTRIBUTING.md) for contribution, commit, versioning, and release conventions.

## Source of truth

- Follow the user's current instructions and preserve unrelated working-tree changes, including staged and experimental work.
- When feature specs are present in `docs/specs/`, follow their explicit decisions; illustrative examples are not additional requirements.
- Source code and tests describe what is implemented. Do not describe planned behavior as available functionality.
- Do not convert a CLI experiment, generated output, or an uncommitted prototype into accepted product direction or a commit without the user's direction.
- Treat the config `version` as a schema version, not a software or commit version. Group schema additions made for the same unreleased change set into one schema bump; do not bump once per field, feature, commit, or implementation pass. Before choosing the next value, check the latest schema version exposed to users and follow the migration policy in `CONTRIBUTING.md`.

## Consumer-facing agent skill

- `.agents/skills/dirule/SKILL.md` is an installable skill for agents using Dirule in users' projects or personal folders. It is not contributor guidance for changing Dirule itself.
- Keep the skill aligned with implemented CLI behavior. Source and tests are authoritative; user documentation explains supported syntax and workflows. Link to those docs instead of maintaining a second full configuration reference in the skill.
- When a change affects public commands, configuration or category formats, preset selection, ignore behavior, or filesystem safety, review the skill and update any workflow that changed. Verify command examples against the current CLI and docs.

## Documentation maintenance

- For user-visible behavior, command, configuration, or release-workflow changes, identify the affected canonical docs in `README.md`, `docs/`, and `examples/` and update them with the change. Review relevant specs when the documented contract changes. Keep examples accurate and avoid duplicating full references across files.
- Keep each fact in its canonical document and link to it from the README or agent skill where useful. Code and tests remain authoritative for implemented behavior; specs describe intended contracts.

## Workspace boundaries

- `crates/dirule`: CLI parsing, output, and command orchestration.
- `crates/dirule-app`: frontend-neutral configuration, preset, preview, and apply services shared by the CLI and future interfaces.
- `crates/dirule-core`: configuration, matching, transformation, validation, and virtual planning; keep it independent of filesystem I/O.
- `crates/dirule-fs`: traversal and filesystem operations; depend on `dirule-core` for shared rule data and plans.
- Keep application behavior in `dirule-app`, which coordinates `dirule-core` and `dirule-fs`. Frontends may depend on those crates for public types, but should not duplicate their behavior. `dirule-fs` depends on `dirule-core`; avoid cycles and keep frontend-specific dependencies out of `dirule-app`.

The CLI is Dirule's only user interface at this stage. Prefer portable standard-library behavior; isolate platform-specific filesystem guarantees inside `dirule-fs` when they are required.

## Filesystem behavior

When implementing file changes, calculate and validate a complete plan before mutation. Never overwrite an existing path. Respect the project's ignore rules, skip symlinks unless a later design explicitly supports them, and revalidate sources and destinations when applying a plan. Keep path matching and transformations in `dirule-core`; do not let filesystem traversal determine rule semantics.

## Path checks

- Before changing paths in this repository, inspect the proposed moves with `cargo run -p dirule -- plan --format json`.
- Verify the result with `cargo run -p dirule -- check --format json`; `check` never changes files. Status `0` means the names comply, `1` means the plan contains changes or conflicts, and `2` means Dirule could not evaluate the check. Treat status `2` as an error, not as a pass.
- Configured project rules can use matching language servers to propose code-reference edits with a rename. Review those edits in the plan and run the relevant project checks after applying. Quick sorting and renaming operations remain path-only.

## Work and validation

Inspect the affected crate and relevant spec before changing behavior. Make the smallest coherent change, keep module boundaries clear, and avoid comments that merely repeat the code. Add dependencies only when they materially simplify or strengthen the implementation.

Run relevant workspace checks when validation is requested or required for the task, and report exactly what ran:

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```
