---
name: dirule
description: Configure and use the Dirule CLI for project or personal file and directory naming, sorting, checks, presets, and undo. Use when a user asks an agent to set up, inspect, preview, apply, or maintain Dirule rules.
---

# Use Dirule

Use this skill when managing files through Dirule in a user's project or personal folders. It is for using Dirule, not changing Dirule's source code.

## Check the available version and scope

1. Confirm the executable and its version with `dirule --version`. If it is unavailable, tell the user; do not install software unless asked.
2. Read `dirule --help` and the relevant subcommand help before relying on options. Installed versions may differ from the online documentation.
3. Establish the exact target directory and whether the request concerns project rules, a one-time sort, or user-wide presets/categories.
4. Before editing, inspect existing `dirule.toml` and `.diruleignore` in the target. For global preset or category tasks, inspect the matching list command first. Preserve existing settings and unrelated files.

## Configure rules

- Create a starter project configuration with `dirule init` only when the user asks to configure that project and no `dirule.toml` exists. Never overwrite an existing config to start over.
- Get the desired naming or sorting behavior from the user's request and existing project conventions. Do not invent broad rules that could move unrelated paths.
- Rules are ordered: put specific rules before general ones. Patterns are globs, not regular expressions; named captures use angle brackets, such as `<year>`. Destinations use `builtin:` transformations or `template:` values.
- A rule with `mode = "check"` reports a naming violation but is never applied; independent rules with the default `mode = "apply"` may still be applied.
- Use `dirule validate` after editing a configuration. Resolve validation errors instead of attempting to apply an invalid config.
- Treat exclusions deliberately: VCS ignore rules filter ignored untracked paths; top-level `ignore` in `dirule.toml` is a Dirule-specific exclusion; `.diruleignore` adds project-local exclusions. Do not edit `.gitignore` or another VCS ignore file unless the user asks.
- Use the installed CLI's help and the [configuration guide](https://github.com/Slyverr/dirule/blob/main/docs/configuration.md) for exact syntax and behavior. The [usage guide](https://github.com/Slyverr/dirule/blob/main/docs/usage.md) covers commands, presets, sorting, and undo.

## Preview before changing paths

- For configured rules, run `dirule plan` against the exact target and selected config or preset. For one-off sorting or renaming, run the operation without `--apply` first; quick operations preview by default.
- Review every proposed source and destination. Stop and report conflicts or unexpected moves; do not bypass them to force an apply.
- Apply only when the user has asked for the filesystem changes. `dirule apply` previews again and normally asks for confirmation. Do not use `--yes` unless the user explicitly requested non-interactive application after the plan has been reviewed.
- After applying configured rules, run `dirule check`; after a quick operation, rerun that operation as a preview and confirm no moves remain. Report the result and any run ID. Dirule changes paths, not file contents or code references; warn the user when a rename may require reference updates. Do not run `dirule undo` unless requested or needed to recover from a failed operation the user authorized.

## Use model-assisted previews

- Use `dirule ask [PROMPT...]` when a user wants natural-language help selecting a category or understanding a selected configuration's preview. Ask uses a bounded model/tool loop but currently exposes read-only tools only; it cannot write configuration or change paths.
- Check the installed `dirule ask --help` and provider setup in the [Ask guide](https://github.com/Slyverr/dirule/blob/main/docs/assistant.md) before using it. The prompt, target path, selected rule excerpts, entry names, and preview results may be sent to the configured provider. Do not send sensitive names or paths without the user's direction, and never claim the model applied changes.
- Prefer the normal deterministic CLI commands when the user already knows the desired operation. Ask is optional and requires a configured provider profile and credentials.

## Keep user-wide settings separate

Project rules belong in the target's `dirule.toml`. Presets and custom sort categories are user-wide resources. Use `dirule presets list` and `dirule categories list` to inspect them. Manage presets with the `dirule presets` commands; for custom categories, follow the platform-specific `categories.toml` location and schema in the [usage guide](https://github.com/Slyverr/dirule/blob/main/docs/usage.md#sort-and-rename-without-a-project-config). Don't copy global settings into a project config or guess paths under the user's config directory.
