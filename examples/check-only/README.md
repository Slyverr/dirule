# Check-only naming rules

Use this config when a project wants to report naming violations in local checks or CI without renaming paths. It expects shell scripts under `scripts/` to use `kebab-case`.

```sh
dirule validate --config examples/check-only/dirule.toml
dirule check --config examples/check-only/dirule.toml --target ./my-project
```

`check` exits with status `1` when a matching filename needs a different name. It never changes paths. An `apply` run also leaves entries owned by a `mode = "check"` rule unchanged.
