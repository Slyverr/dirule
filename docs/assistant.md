# Ask with a model

`dirule ask` is an experimental, provider-configurable assistant for Dirule. The model interprets your request and can call only Dirule's registered tools. It can inspect a bounded sample of target paths, list available rule-source names and sorting categories, inspect a selected rule config or named global preset, preview configured rules, quick sorting, or renaming, and inspect apply history for the selected target. It can apply sort, rename, configured-rule, or undo plans after you approve the exact path and directory actions in an interactive terminal. It can also validate a proposed `dirule.toml` and create it in the selected target after you approve its exact contents. Ask does not read project file contents, and it never overwrites an existing config. Source listing returns whether the target has a local `dirule.toml` and up to 64 global preset names; it does not send preset contents or storage paths. When more than 64 presets exist, the result reports that the list was truncated. A listed preset can be inspected, previewed, or applied by name during the same Ask run.

## Configure a provider profile

Create `assistant.toml` in Dirule's per-user config directory:

- Linux and other Unix systems: `$XDG_CONFIG_HOME/dirule/assistant.toml`, or `~/.config/dirule/assistant.toml` when `XDG_CONFIG_HOME` is unset.
- macOS: `~/Library/Application Support/dirule/assistant.toml`.
- Windows: `%APPDATA%\dirule\assistant.toml`.

Each profile's `api_key_env` names its credential. Dirule uses a non-empty process environment variable as an override; if it is unset or blank, Dirule reads the same name from `credentials.env` beside `assistant.toml` in the per-user config directory. This lets local users keep a persistent key in one private file while CI or a secret manager can provide a per-run override. For example, on a default Linux setup:

```sh
mkdir -p ~/.config/dirule
chmod 700 ~/.config/dirule
touch ~/.config/dirule/credentials.env
chmod 600 ~/.config/dirule/credentials.env
```

```dotenv
# ~/.config/dirule/credentials.env
GEMINI_API_KEY=your-key
NVIDIA_API_KEY=your-key
```

On Windows, keep it in your user-only `%APPDATA%` directory. The file is plain text, not encrypted storage. Dirule uses only the selected profile's key; it does not load project `.env` files or execute this file as shell code. Lines use `NAME=value`; blank lines and full-line `#` comments are allowed, and matching single or double quotes may surround a value. No variable expansion or escape processing is performed. Environment variables remain useful for CI and secret managers.

These examples use currently documented tool-capable endpoints. Model availability and free access can change, so check the linked provider catalog before testing.

For OpenAI's native Responses API:

```toml
[profiles.openai]
adapter = "openai-responses"
endpoint = "https://api.openai.com/v1/responses"
model = "YOUR_MODEL_ID"
api_key_env = "OPENAI_API_KEY"
```

Replace `YOUR_MODEL_ID` with a model that supports function calling. This adapter sends `store = false` and keeps response items in memory only while the current Ask tool loop runs. OpenAI says Responses are stored for 30 days by default and that `store = false` disables that response storage; other provider data handling still applies. See OpenAI's [function-calling guide](https://developers.openai.com/api/docs/guides/function-calling) and [conversation-state and retention guide](https://developers.openai.com/api/docs/guides/conversation-state).

For Google's OpenAI-compatible Gemini endpoint:

```toml
version = "1.0"
default_profile = "google"

[profiles.google]
adapter = "openai-compatible"
endpoint = "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
model = "gemini-3.8-flash"
api_key_env = "GEMINI_API_KEY"
```

Google documents this endpoint, model, and function-calling format in its [OpenAI compatibility guide](https://ai.google.dev/gemini-api/docs/openai). Gemini's free tier may use submitted content to improve Google's products; see [Gemini pricing and data use](https://ai.google.dev/gemini-api/docs/pricing).

For NVIDIA's hosted NIM endpoint, use a model listed as a free endpoint and supporting tool calls. One currently listed example is `openai/gpt-oss-20b`:

```toml
[profiles.nvidia]
adapter = "openai-compatible"
endpoint = "https://integrate.api.nvidia.com/v1/chat/completions"
model = "openai/gpt-oss-20b"
api_key_env = "NVIDIA_API_KEY"
```

See [NVIDIA's model page](https://build.nvidia.com/openai/gpt-oss-20b) and [Chat Completions API reference](https://docs.api.nvidia.com/nim/reference/openai-gpt-oss-20b-infer). NVIDIA describes hosted NIM endpoints as free for prototyping; model availability and limits can vary.

To override the per-user file for one shell or CI job, set the variable directly. For example, in Bash:

```sh
export GEMINI_API_KEY="your-key"
export NVIDIA_API_KEY="your-key"
```

Or in PowerShell:

```powershell
$env:GEMINI_API_KEY = "your-key"
$env:NVIDIA_API_KEY = "your-key"
```

Dirule never copies key values into `assistant.toml`. On NixOS, use your secret manager to provision the per-user `credentials.env` file with private permissions, or provide the named variable to the process. Do not put a secret literal in a Nix expression that would store it in the Nix store. Project `dirule.toml` files cannot configure credentials. `--assistant-config FILE` selects an alternate profile file; credential lookup remains in Dirule's per-user config directory.

To use a non-default profile, add another `[profiles.NAME]` section and select it with `--profile NAME`. Pass `--assistant-config FILE` to load profiles from another file.

## Try a read-only preview

Create a synthetic target and ask Dirule for a preview:

```sh
mkdir -p /tmp/dirule-ask-demo
touch /tmp/dirule-ask-demo/photo.png
dirule ask --target /tmp/dirule-ask-demo "Preview sorting the images in this folder"
```

Select the NVIDIA profile with `--profile nvidia`. The command sends the prompt, target path, and any requested preview data to the selected provider. It does not send file bytes or modify the target.

## Approve a change

Ask can apply a sort, rename, selected rule config, or journal undo after it displays every proposed path and directory action and you confirm with `y`. Approval is per operation. Declining leaves the target unchanged. Ask refuses filesystem changes when standard input is not an interactive terminal; it has no `--yes` bypass. Conflicting plans and plans with more than 40 actions are refused.

```sh
dirule ask --target ~/Downloads "Sort images into category folders and apply the result"
```

Successful changes use Dirule's normal journal, so the run ID can be undone with `dirule undo RUN_ID` or from a later Ask session. Ask can list recent runs for the selected target, inspect a run, and preflight its inverse actions. Undo displays every inverse move and directory action before approval. Directories created by the run are removed only when empty; non-empty directories are retained and reported.

## Draft a project config

Ask can validate a proposed TOML document with Dirule's configuration reader. If you ask it to create the config, it displays the exact TOML and writes `dirule.toml` in the selected target only after approval. It refuses to replace an existing file.

## Ask for a preview

```sh
dirule ask --target ~/Downloads "Preview sorting images into the images category"
dirule ask --target ./project --config ./dirule.toml "Explain the configured naming rules"
dirule ask --target ./project --preset downloads "Preview the configured rules"
```

When no `--config` or `--preset` is given, Ask uses the shared rule-source discovery for the selected target: a target-local `dirule.toml`, then one global preset configured for that exact target. Ambiguous preset matches are reported as an error. With no match, Ask still supports category listing and quick sort or rename previews.

With no prompt argument, `dirule ask` prompts for a task in a terminal or reads the prompt from standard input:

```sh
printf '%s\n' "Preview a kebab-case rename" | dirule ask --target ./project
```

The selected provider receives your prompt and the target path. If Dirule lists sources, it sends available preset names; if it inspects a rule config or preset, its rule names, patterns, templates, and exclusions are sent. If Dirule calls a draft-config tool, the proposed TOML is returned to the model for validation and explanation. If it calls a preview, apply, or journal tool, the provider also receives relevant entry names, run IDs, statuses, and proposed path or directory actions. Journal tools expose runs for the selected target only. The tools do not read project file bytes. A successful config creation, apply, or undo happens only after local approval. Dirule does not save conversations to local files. The native Responses adapter requests `store = false`; provider retention and safety logging still follow that provider's policies.

Ask bounds each run to 10,000 scanned entries and three minutes. Target inspection returns at most 100 paths; larger listings are marked as truncated. If a preview reaches the scan limit, choose a narrower target.

Provider data policies differ. Google's [Gemini API pricing and data-use documentation](https://ai.google.dev/gemini-api/docs/pricing) describes how free and paid tier data is handled. Review the terms for your configured provider before sending private paths or names.

## Test a live provider

The ordinary test suite uses local mock endpoints. To verify a real provider's tool-calling behavior, configure a profile above and make its named key available through the process environment or the per-user `credentials.env`, then run the opt-in smoke test:

```sh
DIRULE_LIVE_PROFILE=google cargo test -p dirule-assistant --test live_provider -- --ignored --nocapture
```

Set `DIRULE_LIVE_PROFILE` to any configured compatible or native Responses profile, such as `nvidia` or `openai`. The test creates a temporary folder with a synthetic image name, requires the model to call Dirule's read-only `preview_sort` tool, and checks that the file stays in place. It does not run in ordinary tests or CI. Set `DIRULE_ASSISTANT_CONFIG` to use a profile file outside Dirule's standard user config path.
