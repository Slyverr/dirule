# Custom download categories

This catalog adds a `spreadsheets` category and an `archives` category. Install it as `categories.toml` in Dirule's user configuration directory; the [usage guide](../../docs/usage.md#sort-and-rename-without-a-project-config) lists the platform-specific locations.

List the effective categories, then preview a sort of Downloads:

```sh
dirule categories list
dirule sort spreadsheets archives --target ~/Downloads
```

When the preview looks right, add `--apply` to request the changes. Dirule will ask for confirmation and record successful moves in its undo journal.
