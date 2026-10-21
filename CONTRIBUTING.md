# Contributing

Dirule is a Rust Cargo workspace. Keep contributions focused and follow the workspace boundaries below. Update user documentation when implemented behavior changes.

The consumer-facing Agent Skill at `.agents/skills/dirule/SKILL.md` teaches agents to use Dirule in other projects. Review it alongside user docs when CLI or assistant behavior, configuration, presets, categories, or filesystem safety workflows change, including provider data flow and approval gates. Keep its examples aligned with implemented behavior and link to user docs for full details rather than copying the full configuration reference.

## Workspace boundaries

- Put rule data, matching, transformations, validation, and planning in `dirule-core`.
- Put traversal and filesystem operations in `dirule-fs`.
- Keep CLI parsing and output in `dirule`.
- Put frontend-neutral configuration, preset, preview, and apply services in `dirule-app`; do not add CLI or UI dependencies there.
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
cargo deny check
```

For changes to rule behavior, cover the behavior and its edge cases. For filesystem changes, also cover conflicts, ignored paths, and failure paths. State which checks ran when reporting the change.

## GitHub workflows

The repository uses separate workflows for code validation, dependency policy, crate publishing, and downloadable GitHub releases. A green CI run validates code; it does not publish packages or create a release.

| Workflow | When it runs | What it does |
| --- | --- | --- |
| `CI` (`ci.yml`) | Branch pushes, pull requests, or manual runs | Skips the expensive jobs for documentation-only changes. Otherwise checks the Nix package, runs format/check/test/Clippy on Rust 1.88, and runs check/test/Clippy plus CLI install smoke tests on stable Rust for Linux, macOS, and Windows. Version tags are handled by `Release`, not by this workflow. |
| `Dependency audit` (`dependency-audit.yml`) | Dependency-policy changes in pull requests or pushes to `main`/`develop`; weekly; or manually | Runs `cargo deny` against the workspace dependencies. |
| `Publish crates` (`publish-crates.yml`) | Relevant pull requests; or called by `Release` | Pull requests dry-run packaging. On a stable version tag, it tests on Linux, macOS, and Windows, verifies that the tag matches every workspace version and points to a commit on `main`, and dry-runs only crate versions that are not already on crates.io. It publishes missing versions through Trusted Publishing and safely skips versions already uploaded. |
| `Release` (`release.yml`) | Pull requests; or version tags such as `v0.5.0` | Pull requests validate the cargo-dist plan. A pushed version tag builds the configured archives, installers, and checksums, then calls `Publish crates`. It creates the GitHub Release only after crate publishing succeeds. Prerelease tags create prereleases and skip crates.io publishing. |
| `Dirule path check` (`dirule-check.yml`) | Only when another repository calls this reusable workflow | Installs a selected published Dirule version and runs `dirule check` in that repository. It does not run automatically as part of this repository's CI. |

The release tag is the single release trigger. Update all workspace package versions and the changelog, merge that commit to `main`, wait for CI to pass, then push a matching `vX.Y.Z` tag at that commit. The tag run builds the downloadable assets, checks the tag is on `main`, publishes any crate versions that are not already on crates.io, and creates the GitHub Release only after publishing succeeds. Re-running the same tag does not try to upload versions that already exist. A normal branch push never publishes or announces a release. Repository rulesets must mark checks as required if pushes or merges should be blocked until they pass.

Put integration tests for public behavior in the crate's `tests/` directory and group larger suites by behavior. For a multi-file suite, use one integration-test entry point with sibling modules, such as `tests/cli_flow.rs` and `tests/cli_flow/`; keep shared fixtures in that suite's `common.rs`. Exercise public APIs as a downstream caller would, or invoke the real CLI in an isolated temporary project. Keep focused unit tests beside the implementation only when they need private behavior that cannot be covered meaningfully through the public API; do not expose internals just to test them. Avoid duplicating the same behavior at both levels or collecting crate-wide behavior tests in `lib.rs`.

Prefer real deterministic logic over mocks. Use mocks or fakes at genuine external boundaries when they make a test more reliable; use temporary directories for filesystem behavior and invoke the CLI for end-to-end command behavior.

## Benchmarks

Run the Criterion benchmarks with:

```sh
cargo bench -p dirule-core --bench core
cargo bench -p dirule-fs --bench scan
cargo bench -p dirule-fs --bench apply
```

The core benchmarks cover config parsing, named-pattern matching, filename transforms, and planning at several input sizes. The filesystem benchmarks measure repeat scans of nested trees with VCS and configured ignore patterns, destination validation, and full application of file rename cycles with and without journaling at several input sizes. The scan benchmark separates configured-pattern lookup overhead from the time saved by pruning a generated subtree, and measures when configured ignores skip Git-tracked paths before `.diruleignore` recovery is needed. Save a local baseline with `-- --save-baseline NAME` and compare it later with `-- --baseline NAME`. Compare runs on the same machine and build setup.

For a code change, compare the baseline and candidate revisions with the same toolchain and benchmark setup. Record both commit IDs with the result, run the measurements close together, and repeat comparisons when load or outliers make them unclear. Treat short Criterion runs as exploratory; do not use them alone to claim a performance improvement. Criterion output is local data and should not be committed.

## Releases

Dirule uses lockstep workspace versions. During `0.x`, use a patch release for bug fixes and compatible maintenance; use a minor release for new user-facing capabilities or intentional CLI/config behavior changes. Do not bump the version for each commit. Treat `0.x` as pre-stable: compatibility is not guaranteed. Move to `1.0.0` when the CLI and configuration behavior are stable enough to support a compatibility promise; after that, use SemVer major for breaking changes, minor for compatible features, and patch for compatible fixes. Release when a coherent change set is ready and CI passes; there is no fixed release calendar.

The `dirule.toml` format version is separate from the workspace release version. Treat it as the version of the configuration schema users can rely on. For one unreleased change set, group compatible schema additions and advance the schema version once from the latest version exposed to users; do not create intermediate schema versions per feature, field, commit, or implementation pass. Before bumping, check released versions, the release branch, and any shared examples or documentation that users may already follow. If an intermediate schema version was intentionally exposed and users may have created configs with it, preserve a sequential migration from it. Otherwise, fold the unreleased changes into the next schema version and migrate directly from the latest exposed version. Update migration tests and shipped examples with the same decision.

Keep completed user-visible changes under `Unreleased` in `CHANGELOG.md`. For a release, update the workspace version in `Cargo.toml` and `Cargo.lock`, move those notes under the release heading, and leave a fresh `Unreleased` section. Run the workspace validation checks above and confirm the release plan:

```sh
cargo install cargo-dist --version 0.33.0 --locked
cargo dist plan --tag=vX.Y.Z --output-format=json
```

Use the `cargo-dist` version pinned in `dist-workspace.toml`.

On Linux x86_64, smoke-test the release archive before tagging:

```sh
cargo dist build --tag=vX.Y.Z --target=x86_64-unknown-linux-gnu --artifacts=local
(cd target/distrib && sha256sum -c dirule-x86_64-unknown-linux-gnu.tar.xz.sha256)
smoke_dir="$(mktemp -d)"
trap 'rm -rf "$smoke_dir"' EXIT
tar -xJf target/distrib/dirule-x86_64-unknown-linux-gnu.tar.xz -C "$smoke_dir"
"$smoke_dir/dirule-x86_64-unknown-linux-gnu/dirule" --version
```

Relevant pull requests run `cargo publish --workspace --dry-run --locked` to check package contents without uploading. The tag workflow uses crates.io's public version endpoint to avoid duplicate uploads, fails closed if that check cannot be completed, and publishes only versions that are still missing. Crates.io publishing uses a short-lived Trusted Publishing token; no registry token belongs in GitHub secrets.

The `dirule` CLI depends on the other workspace crates, so publish all five packages together in lockstep. Before the first upload, review the crate names, versions, metadata, and packaged files: crates.io does not let you delete a published version.

For a crate's first-ever upload, crates.io requires one manual publish before a trusted publisher can be configured. From the checkout containing that initial version, inspect and validate the packages:

```sh
cargo package --workspace --list --locked
cargo publish --workspace --dry-run --locked
```

After the dry run succeeds and the package contents are reviewed, authenticate Cargo locally with a crates.io API token (`cargo login` reads it from stdin) and upload once:

```sh
cargo publish --workspace --locked
```

Dirule's five crates already have an initial published version. Configure a trusted publisher on crates.io for **each** crate, using owner `Slyverr`, repository `dirule`, workflow `release.yml`, and GitHub Actions environment `crates-io`. The tag-triggered release workflow calls the reusable publisher and exchanges GitHub's OIDC identity for a short-lived token. Do not store a crates.io API token in the repository or GitHub secrets. If a crate is already configured for `publish-crates.yml`, update its workflow filename to `release.yml` before the next tag that needs to upload a new version.

The generated cargo-dist workflow keeps `contents: write` only on the job that creates the GitHub Release; other jobs retain read-only permissions, and the release job does not inherit repository secrets. `cargo-dist` cannot currently express that built-in job permission split. To regenerate `.github/workflows/release.yml`, temporarily remove `allow-dirty = ["ci"]`, run `cargo dist generate --mode ci`, restore the permission split and remove secret inheritance, then restore `allow-dirty = ["ci"]`.

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
