# Project naming rules

This config normalizes Rust source filenames to `snake-case` and directory names under `assets/` to `kebab-case`. It leaves file contents untouched.

Preview it against a project before applying anything:

```sh
dirule validate --config examples/project-naming/dirule.toml
dirule plan --config examples/project-naming/dirule.toml --target ./my-project
dirule apply --config examples/project-naming/dirule.toml --target ./my-project
```

`plan` shows the proposed paths. `apply` shows the plan again and asks for confirmation. Dirule will not rewrite imports, links, or other references inside files after a rename.
