# Project naming rules

This config normalizes Rust source filenames to `snake-case` and directory names under `assets/` to `kebab-case`. It does not configure language-server reference updates, so this example changes paths and leaves file contents untouched.

Preview it against a project before applying anything:

```sh
dirule validate --config examples/project-naming/dirule.toml
dirule plan --config examples/project-naming/dirule.toml --target ./my-project
dirule apply --config examples/project-naming/dirule.toml --target ./my-project
```

`plan` shows the proposed paths. `apply` shows the plan again and asks for confirmation. This example does not update imports, links, or other file references because its config does not set up a language server. See the [configuration guide](../../docs/configuration.md#language-server-reference-updates) to enable reference updates for a project.
