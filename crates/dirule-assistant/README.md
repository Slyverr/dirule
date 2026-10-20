# dirule-assistant

Experimental assistant sessions and tool orchestration for Dirule. It supports OpenAI-compatible Chat Completions and the OpenAI Responses API, with bounded tool calls and approval before changes are applied.

The assistant tools operate on paths and Dirule configuration; they do not read file contents. Requests can include project paths, names, rule excerpts, and preview results. Use `dirule` for the complete command-line application.

This crate is a building block for Dirule and shares its pre-1.0 versioning. Its API and assistant behavior may change before `1.0`.

[Project and assistant documentation](https://github.com/Slyverr/dirule)
