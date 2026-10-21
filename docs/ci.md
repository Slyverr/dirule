# CI and automation

Commit `dirule.toml` at the project root, then run `dirule check` from that directory. `check` never changes paths. It exits with status `0` when names comply, `1` when changes or conflicts are found, and `2` when Dirule cannot evaluate the check.

Use JSON when another tool needs the plan details:

```sh
dirule check --format json --color never
```

Successful JSON output goes to stdout. A check that finds violations also writes a versioned error document to stderr and exits with status `1`. See the [CLI output specification](specs/cli-output.md) for JSON schemas and stable error codes.

## GitHub Actions

The reusable workflow checks out the calling repository, installs a pinned Dirule release from crates.io, and runs `dirule check` on Ubuntu:

```yaml
name: Dirule

on:
  push:
  pull_request:

permissions:
  contents: read

jobs:
  check:
    uses: Slyverr/dirule/.github/workflows/dirule-check.yml@main
    with:
      dirule-version: "0.4.1"
```

The workflow version reference and CLI version are independent: use a full commit SHA instead of `@main` when you need to pin the workflow itself, and update `dirule-version` when upgrading the CLI. A failing check exits nonzero and fails the job.

For a custom job or runner, install Dirule and run the check directly:

```yaml
- uses: actions/checkout@v7
- uses: dtolnay/rust-toolchain@stable
- name: Install Dirule
  run: cargo install --locked --version 0.4.1 dirule
- name: Check project paths
  run: dirule check --color never
```

## Pre-commit

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

Install Dirule separately and make sure it is on `PATH`. This hook checks the configured target as a whole; it does not limit the check to staged paths.
