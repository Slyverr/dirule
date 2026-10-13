# Contributing

Dirule is a Rust Cargo workspace. Keep contributions focused and follow the workspace boundaries below. Update user documentation when implemented behavior changes.

## Workspace boundaries

- Put rule data, matching, transformations, validation, and planning in `dirule-core`.
- Put traversal and filesystem operations in `dirule-fs`.
- Keep CLI parsing and orchestration in `dirule`.
- Add a dependency to the narrowest crate that needs it. Prefer a maintained library over a custom parser or filesystem implementation when it fits the required behavior.

## Rust style

Use Rust 1.88 or newer for local development and validation.

Use blank lines to separate distinct steps in function bodies and type declarations. Keep related declarations and operations together, and keep the import block contiguous. Do not add blank lines mechanically between every statement; use them to make logical transitions easier to scan. Run `cargo fmt` for standard Rust formatting.

## Validation

Run the checks relevant to the change from the workspace root:

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

For changes to rule behavior, cover the behavior and its edge cases. For filesystem changes, also cover conflicts, ignored paths, and failure paths. State which checks ran when reporting the change.

Put integration tests for public behavior in the crate's `tests/` directory, grouped by the behavior they cover. These tests should exercise the crate's public API. Keep unit tests that need private implementation details beside the code they test in a `#[cfg(test)]` module. Avoid collecting crate-wide behavior tests in `lib.rs`.

## Benchmarks

Run the Criterion benchmarks with:

```sh
cargo bench -p dirule-core --bench core
cargo bench -p dirule-fs --bench scan
```

The core benchmarks cover config parsing, named-pattern matching, filename transforms, and planning at several input sizes. The filesystem benchmark measures repeat scans of nested trees with ignored paths. Save a local baseline with `-- --save-baseline NAME` and compare it later with `-- --baseline NAME`. Compare runs on the same machine and build setup.

## Commits

Use this first-line format:

```text
type(crate/area): summary
```

Use exactly two scope parts for crate changes: the crate and one broad affected area. Do not add nested module or file paths. For repository-wide documentation, use `docs: summary` without a scope. Examples:

```text
feat(dirule-core/transform): add kebab-case conversion
fix(dirule-fs/scan): preserve tracked ignored files
chore(dirule/deps): add clap for argument parsing
docs: clarify project setup
```

A one-line commit is enough when it explains the change. Add a body only when it adds useful context: a short paragraph followed by bullets when listing distinct changes helps; omit an empty or repetitive body. Stage only files that belong to that commit, and keep experiments or unrelated edits out of it.

If the latest commit is local and unpushed, and current work is a missed piece of that same change, fold it into that commit with a soft reset instead of adding a fixup commit. Do this only when the work truly belongs together; keep unrelated changes separate.
