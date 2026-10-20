# dirule-core

The rule engine behind Dirule. It handles versioned configuration, ordered glob matching, named captures, destination templates, name transformations, and rename planning.

This crate does not scan or modify the filesystem. Use `dirule-fs` for filesystem operations and `dirule` for the command-line application.

The crate is part of the Dirule workspace and shares its pre-1.0 versioning. Its API may change before `1.0`.

[Project and usage documentation](https://github.com/Slyverr/dirule)
