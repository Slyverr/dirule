# Dirule

Dirule is a Rust workspace at its initial project scaffold. The application and
filesystem crates are placeholders; no Dirule commands or file rules are
implemented yet.

## Workspace

- `crates/dirule-core`: placeholder for shared rule logic.
- `crates/dirule-fs`: placeholder for filesystem functionality; depends on
  `dirule-core`.
- `crates/dirule`: binary crate; currently prints `Hello, world!`.

## Run

With Rust and Cargo installed, run the default workspace binary:

```sh
cargo run
```

The workspace uses Rust edition 2024 and Cargo resolver 3.
