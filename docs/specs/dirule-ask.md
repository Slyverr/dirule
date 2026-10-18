# Dirule Ask

Status: approved for the initial read-only implementation.

## Purpose

`dirule ask` is a small domain-specific harness around a model chosen by the user. It helps the model understand a filesystem or naming request, choose among Dirule's capabilities, and explain the result in terms the user can review.

The model interprets intent. Dirule supplies its usage guidance and relevant target context, exposes a narrow set of typed tools, validates each tool request, and runs the existing deterministic application services. Dirule remains responsible for matching, planning, conflict detection, and filesystem changes. The model does not receive shell access or unrestricted filesystem access.

## User interaction

The first interface is a one-shot command with a prompt supplied as arguments or standard input:

```text
dirule ask [PROMPT...]
```

The initial CLI accepts a target, a provider profile, an explicit assistant profile file, and a rule config or preset. The default target is the current directory. When no rule source is explicit, Ask uses the shared target-local config and exact-target global-preset discovery. One invocation can run a bounded provider and tool-call loop, then ends with one final response. It does not support interactive user clarification or persist chat transcripts.

Example requests include:

- “Sort the images and videos in this folder into category subdirectories.”
- “Why does my current naming plan have conflicts?”
- “Draft rules that make source filenames kebab-case while leaving directories alone.”
- “Preview these rules on `./src`, then apply them if the destinations are safe.”

The model may inspect available Dirule capabilities, request a preview, and explain findings. Any request to write configuration or change target paths must stop at a review step until the user approves the exact proposal.

## Model providers

Provider wire formats are adapters around one provider-independent request, response, and tool-call model. Tool definitions and execution policy belong to Dirule, not to an external provider SDK.

The provider boundary is designed for:

- OpenAI Responses API.
- OpenAI-compatible Chat Completions APIs, with a configurable endpoint and model. This adapter can target Google Gemini's OpenAI-compatible endpoint and compatible NVIDIA NIM models when the selected model supports tool calling.

The current implementation includes the OpenAI-compatible Chat Completions adapter. The native OpenAI Responses adapter is still to be added. The compatible adapter can target Google Gemini and NVIDIA NIM models that implement the same tool-calling contract. Provider and model capabilities vary. Dirule reports malformed or unsupported responses as errors; compatibility must be verified per model rather than assumed for every model behind a compatible endpoint. Native Gemini and Anthropic adapters can be added later without changing the tool catalog.

Provider profiles are user-level Dirule settings, separate from project `dirule.toml`. A profile records its adapter, endpoint URL, model, and the environment-variable name for its API key. At runtime a non-empty process environment value overrides the matching entry in `credentials.env` beside the user-level `assistant.toml`; the file is used if the variable is unset or blank. This gives CI and secret managers a per-run override while keeping the local default in one private user file. Dirule never searches project directories for credentials or loads project `.env` files. The credentials file is a bounded, data-only `NAME=value` file, not a shell script; on Unix it must be private to its owner. On NixOS, a secret manager can provision the per-user file or provide the process environment without putting the secret in the Nix store. Project rule configs cannot select credential sources. Dirule uses only the selected key and never writes or prints its value.

## Dirule tools

Tools are named, typed operations that call existing Dirule application services. The catalog is intentionally small and grows by adding independent tools. The current implementation includes bounded target inspection, category listing, selected-config inspection, and read-only configured-rule, quick-sort, and quick-rename previews. The planned catalog also includes:

- List available configuration sources.
- Draft and validate a project `dirule.toml` using Dirule's configuration reader.
- Save a reviewed configuration after explicit approval.
- Apply a reviewed preview after explicit approval, using Dirule's normal conflict checks and journal.
- Inspect journal runs and show an undo preview; undo also requires explicit approval.

The model can request only tools registered by the active Dirule version. Each tool validates its arguments and calls an existing app service; it does not reimplement matching, scanning, planning, or mutation logic. Unknown tools, invalid arguments, oversized results, and exhausted limits produce controlled errors.

## Approval and safety

- Read-only inspection, validation, and previews may run without approval.
- Saving a config, applying a filesystem plan, or undoing a run requires explicit user approval after Dirule displays the exact paths and actions.
- A model request, a prior approval, or text such as “do whatever is needed” does not authorize a later or different mutation.
- Non-interactive use must provide a separate explicit approval mechanism; it must not infer approval from the prompt.
- Apply and undo use the existing identity revalidation, conflict policy, and journal services. The model never receives direct filesystem write access.
- No arbitrary shell, command execution, or unrestricted file read/write tool is provided.
- Bound provider turns, tool calls, request and response bytes, target entries, and elapsed time. The initial implementation has a three-minute overall deadline, provider request timeouts, a 10,000-entry scan cap, and bounded tool results. Handle cancellation, API errors, malformed tool calls, and limit exhaustion without applying changes.

## Data sent to providers

Before a request, the CLI identifies the selected provider profile and model and states that the prompt, target path, and requested names or preview data will be sent. Tool definitions and Dirule's instructions are also part of the model request. File bytes are excluded by default. Any future feature that sends file contents must define its own explicit data flow and user consent.

Provider data policies differ. Google's Gemini free tier may use submitted content to improve its products, while paid-tier content is not used for that purpose. Dirule cannot infer which account tier an API key belongs to, so it must not claim that a configured profile is on a particular tier. Documentation links to current provider terms; live tests should use synthetic paths and names unless the user chooses otherwise.

## Architecture

- Put the assistant loop and provider adapters in a separate `dirule-assistant` crate.
- Let that crate depend on `dirule-app` for filesystem-related operations. Keep network clients and model-specific types out of `dirule-core` and `dirule-fs`.
- Define one normalized model conversation/tool-call contract. Provider adapters translate that contract to and from provider wire formats.
- Keep model transport separate from a typed Dirule tool catalog and dispatcher.
- Make the assistant service frontend-neutral so the CLI can use it first and a future TUI can reuse the same tool policy and services.
- Do not require MCP for `dirule ask`. A future MCP transport may expose the same catalog if a concrete external-host use case emerges.
- Do not load arbitrary third-party code or user-supplied executable plugins in the initial implementation.

## Delivery sequence

1. Establish the normalized provider, conversation, tool, request/response bounds, and approval interfaces. Done; the tool loop is covered by deterministic fake-provider tests. Provider requests receive the remaining overall timeout, and turn, tool-call, elapsed-time, result-byte, and scan-entry limits are enforced.
2. Add versioned user profiles and the first network adapter. Done for OpenAI-compatible Chat Completions; native OpenAI Responses remains to be implemented. Request construction and response parsing are covered by a local mock endpoint.
3. Add read-only target inspection, categories, config validation, source listing, and previews using `dirule-app`. In progress; bounded target inspection, category and configuration-source listing, selected-config inspection, configured-rule preview, and quick sort/rename previews are implemented. Source listing exposes only local-config availability and global preset names. All Ask previews use a capped scan and reject partial plans; target inspection and preset listing may return clearly marked partial results. Broader edge-case coverage remains.
4. Add config save, apply, and undo tools behind exact-proposal approval.
5. Add opt-in live smoke tests for user-configured endpoints. An ignored integration test now uses the selected profile against a synthetic target and verifies the model calls a read-only Dirule tool. Never put API keys in CI or require network access for ordinary tests.

The first runnable milestone is the read-only CLI slice. Its live-provider check should exercise one configured Google Gemini or NVIDIA NIM profile against a synthetic target. It is separate from ordinary CI and requires the selected key to be available through the process environment or Dirule's per-user credentials file.

## First runnable milestone acceptance

- The CLI can make a model request through a configured OpenAI-compatible Chat Completions endpoint.
- The model can request only registered Dirule tools and receive validated, bounded category, config, or preview results.
- Read-only previews leave target paths and project configuration unchanged.
- Credentials are read from the configured environment variable or the matching entry in Dirule's per-user credentials file. Dirule never writes or prints credential values; file contents are not sent.
- Ordinary offline tests cover the provider wire format and tool loop; an ignored live smoke test uses a synthetic target and a user-provided API key source.
- Core CLI features remain usable without an assistant profile, API key, network access, or installed model SDK.

## Full assistant acceptance

- The model can interpret a natural-language Dirule task and either explain it or request a supported tool.
- Provider adapters use the normalized contract, and offline tests cover tool-call parsing and dispatch with fake providers.
- Tool requests are validated, bounded, and dispatched only through the active Dirule tool catalog.
- Read-only requests and previews do not mutate target paths or project configuration.
- Every write or filesystem mutation requires explicit approval of the exact proposed action.
- Applied filesystem changes use existing Dirule apply, conflict, and journal behavior and can be inspected or undone through the supported tools.
- Credentials are never written by Dirule or printed; file contents are not sent by default.
- Core CLI features remain usable without an assistant profile, API key, network access, or installed model SDK.
