# dirule-fs

Ignore-aware scanning, conflict checks, safe path changes, and recovery journals for Dirule. It applies filesystem operations planned by `dirule-core` and respects VCS ignore rules plus Dirule exclusions.

This crate is a building block for Dirule, not the user-facing CLI. Use `dirule` for the complete command-line application.

The crate is part of the Dirule workspace and shares its pre-1.0 versioning. Its API may change before `1.0`.

[Project and usage documentation](https://github.com/Slyverr/dirule)
