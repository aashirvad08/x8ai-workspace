# Models and providers

The app lets the user save a provider's API key once and start an agent session
with a chosen provider and model. It does not call models itself, and it does not
proxy model traffic: the agent talks to the provider directly, configured by the
app at launch.

Built in Phase 6. Decisions: ADR 0014 (secret storage), ADR 0015 (environment
precedence and material changes), ADR 0016 (provider and model configuration).

## Who owns what

| The app owns | The agent owns |
| --- | --- |
| Provider and model metadata (built-in definitions, models found locally, ids the user added) | How it reads configuration: its variables, flags and config files |
| Credentials, in the macOS Keychain | Its own settings, logins and subscriptions |
| Which provider and model a session uses | Talking to the provider |
| Finding local providers (Ollama), when asked | Anything it does with the key once it has it |

The bridge between them is an **agent adapter** (`crates/agents/src/adapter/`), one
per agent. The provider layer knows nothing about agents; an adapter knows one
agent and nothing about the others.

```
 Models view ──set/remove key──▶ provider_* commands ──▶ Keychain (secrets)
      │                                 │                providers.json (model ids)
      │                                 ▼
 Agents view ──launch(agent, provider/model)──▶ agent_* commands
                                        │  1. plan: executable on PATH, login environment
                                        │  2. adapter: endpoint, variables, arguments
                                        │  3. key from the Keychain (native only)
                                        │  4. trust + approval (provider and endpoint pinned)
                                        ▼
                             agent process on a PTY, in its worktree
```

## Providers

`crates/providers/src/builtin.json`. A provider is data: who hosts it, the wire APIs
it serves at which base URLs, and how it authenticates.

| Provider | Hosting | Endpoints | Auth | Models listed by the app |
| --- | --- | --- | --- | --- |
| Anthropic | hosted | Anthropic Messages `https://api.anthropic.com` | API key | Claude Opus 5.5, Fable 5.1, Sonnet 5, Haiku 4.5 (ids only) |
| OpenAI | hosted | Responses and Chat Completions `https://api.openai.com/v1` | API key | none |
| Google | hosted | Gemini `https://generativelanguage.googleapis.com/v1beta` | API key | none |
| OpenRouter | gateway | Anthropic Messages `https://openrouter.ai/api`; Chat Completions `https://openrouter.ai/api/v1` | API key | none |
| Ollama | local | Anthropic Messages `http://localhost:11434`; Chat Completions `http://localhost:11434/v1` | none | the models Ollama has |

Definitions are validated (`ModelProviderDefinition::validate`): a provider that
sends a key must use HTTPS unless it is on the local machine, and URLs cannot embed
credentials.

## Models

A model the app offers comes from one of three places, and nothing is guessed:

- **Built-in** (`builtIn`): ids the provider definition lists. Only ids known to be
  real are listed, and no capability (context window, price, speed) is claimed:
  `contextWindow` stays empty unless verified.
- **On this machine** (`local`): the models Ollama reports (`GET /api/tags`).
- **Added by the user** (`custom`): any model id, in `providers.json`. This is how
  OpenAI, Google and OpenRouter models are chosen: OpenRouter is a gateway to
  hundreds of models that change constantly, so the app does not enumerate them.

A model id must be 1–200 characters of letters, digits and `. _ : / @ + - [ ]`,
starting with a letter or digit (`is_model_id`). It can never start with `-`, so it
can never be read as a command-line option when an adapter passes it to an agent.

**Discovery.** The app makes no request to a hosted provider. Listing models from
hosted APIs needs an HTTPS client, which the app does not have yet (and would be
network access on the user's behalf); it is deferred. The only discovery is Ollama,
on the loopback address, when the user opens the Models view or presses refresh.
Nothing happens at startup: the app starts offline.

### Ollama

`crates/providers/src/ollama.rs`. Detection, when asked:

1. `GET http://127.0.0.1:11434/api/version` (HTTP/1.0, 0.5 s to connect, 2 s to
   answer). A JSON answer means **Available**, with its version; then
   `GET /api/tags` lists its models.
2. Otherwise, the `ollama` command on the user's login `PATH`, or `Ollama.app` in
   `/Applications` or `~/Applications`, means **Installed** (not running).
3. Otherwise **Unavailable**.

Nothing is installed, started or downloaded. The UI says how to pull a model
(`ollama pull <model>`, run by the user). A server moved with `OLLAMA_HOST` is not
found.

## Credentials

`crates/secrets/`. See ADR 0014.

- **Where:** the login Keychain, one generic password per provider: service
  `com.x8ai.workspace.providers`, account = the provider's secret name
  (`anthropic`, `openai`, `google`, `openrouter`), label "x8ai Workspace provider
  key (…)" in Keychain Access.
- **Saving:** the webview sends the key once, in `provider_set_credential`. It is
  checked (not empty, at most 4096 bytes, one line, no control characters; a pasted
  newline is trimmed), wrapped in `SecretValue`, and written to the Keychain. The
  input field is cleared. The command returns the provider's status, never the key.
- **Reading:** only the agent commands read a key, natively, when the agent of a
  session using that provider starts, and only to place it in that agent's
  environment. Approving and creating the session only check that one is saved,
  which reads nothing. The key is kept in the app's memory after the first read,
  until the app quits or the key is replaced or removed in Models: macOS may ask
  for the Keychain password on a read (always for a build it does not recognize,
  such as an unsigned one that was rebuilt), so it asks at most once per app
  run. No command returns a key; `provider_list` reports only
  `credential: notNeeded | missing | inKeychain` (checked without reading the key).
- **Removing:** `provider_remove_credential` deletes the Keychain item. An agent
  already running keeps the key it was started with; any new run of any session
  needs a key again and fails with "no API key for … is saved".
- **In memory:** `SecretValue` has no `Display` and no `Serialize`, and its `Debug`
  prints `SecretValue(<redacted>)`. `LaunchPlan`, `Configuration` and the PTY's
  `Environment` print variable names only. Errors say what is wrong, never what
  was entered.
- **Never on disk** in anything the app writes: not in `providers.json`, the
  approval or trust stores, worktree metadata, the project, or the worktree (tests
  scan every file for the key).

## Environment construction and precedence

See ADR 0015. An agent starts with the user's login environment (read once from
their login shell, docs/agent-runtime.md), exactly as in Phase 4, plus:

**Agent's own configuration** (no model chosen, the default): nothing is added or
removed. The agent uses its own settings and whatever the shell exports. The
session shows which provider variables the shell sets (names only), for example
"from your shell: ANTHROPIC_API_KEY".

**App-configured** (a provider and model chosen):

1. Every variable the agent's adapter **controls** (the ones that choose its
   provider, endpoint, credentials or model) is removed from the inherited
   environment.
2. The adapter's variables are set.
3. Everything else is kept: `PATH`, `HOME`, `EDITOR`, proxies, the user's other
   keys (for Claude Code, an `OPENAI_API_KEY` stays).

The app's configuration wins, entirely: the shell's provider settings are not mixed
in. The session names the shell variables it replaced ("Replaces from your shell:
ANTHROPIC_BASE_URL, ANTHROPIC_MODEL"), and the approval dialog says the app
replaces them. A Keychain key is used only for a session whose provider was chosen
in the app, never added to other sessions.

## Agent adapters

`crates/agents/src/adapter/`. One trait:

```rust
pub trait AgentAdapter: Send + Sync {
    fn agent(&self) -> &'static str;                    // the agent definition's id
    fn endpoint<'p>(&self, provider: &'p ModelProviderDefinition)
        -> Result<&'p ProviderEndpoint, String>;         // which endpoint, or why not
    fn controls(&self, variable: &str) -> bool;          // what it replaces
    fn configure(&self, provider, endpoint, model, credential) -> Configuration; // env + args
}
```

The adapter is looked up by agent id in one table (`adapter()`); nothing else
compares agent ids. An agent without an adapter (Aider, for example) runs with
its own configuration only, and the UI offers it no model choice.

### Claude Code

Only documented mechanisms: the variables in code.claude.com/docs/en/env-vars and
the `--model` flag; OpenRouter's and Ollama's published Claude Code guides for
those two. `~/.claude` and Claude Code's settings files are never written.

| Provider | Claude Code gets |
| --- | --- |
| Anthropic | `ANTHROPIC_API_KEY=<key>`, `--model <id>` |
| OpenRouter | `ANTHROPIC_BASE_URL=https://openrouter.ai/api`, `ANTHROPIC_AUTH_TOKEN=<key>`, `ANTHROPIC_API_KEY=` (empty, as OpenRouter requires), `ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU,FABLE}_MODEL=<id>`, `--model <id>` |
| Ollama | `ANTHROPIC_BASE_URL=http://localhost:11434`, `ANTHROPIC_AUTH_TOKEN=ollama` (Ollama's documented placeholder), `ANTHROPIC_API_KEY=`, the four aliases, `--model <id>` |
| All of them | `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST=1` |
| OpenAI, Google | not supported: Claude Code speaks only the Anthropic Messages API |

- `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST` is documented "for host platforms that
  embed Claude Code and manage model provider routing": Claude Code then ignores
  provider, endpoint and credential variables in its settings files, so a settings
  file (including a project's `.claude/settings.json`) cannot send the session, or
  the key, elsewhere.
- `--model` takes precedence over `ANTHROPIC_MODEL` and the settings files for the
  session. The aliases make Claude Code's background work and subagents use the
  chosen model on a gateway or local server, where Anthropic's default ids may not
  exist.
- **Controlled:** every `ANTHROPIC_*` variable, and `CLAUDE_CODE_USE_{BEDROCK,
  VERTEX,FOUNDRY,MANTLE,ANTHROPIC_AWS}`, `CLAUDE_CODE_SKIP_*_AUTH`,
  `CLAUDE_CODE_OAUTH_{TOKEN,REFRESH_TOKEN,SCOPES}`, `AWS_BEARER_TOKEN_BEDROCK`,
  `CLAUDE_CODE_SUBAGENT_MODEL[_FORCE]`, `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST`.
- With an API key and an interactive session, Claude Code asks once whether to use
  the key instead of a subscription the user is logged in to. That question is
  Claude Code's own.

### OpenCode

Only documented mechanisms (opencode.ai/docs/config, /providers): the provider's
key in the variable OpenCode reads for it, and `OPENCODE_CONFIG_CONTENT`, the inline
configuration that takes precedence over the global and the project's
`opencode.json`. OpenCode's files are never written.

| Provider | OpenCode gets |
| --- | --- |
| Anthropic | `ANTHROPIC_API_KEY`; config `model: anthropic/<id>`, `provider.anthropic.options.baseURL: https://api.anthropic.com/v1` |
| OpenAI | `OPENAI_API_KEY`; `model: openai/<id>`, `baseURL: https://api.openai.com/v1` |
| Google | `GOOGLE_GENERATIVE_AI_API_KEY`; `model: google/<id>`, `baseURL: https://generativelanguage.googleapis.com/v1beta` |
| OpenRouter | `OPENROUTER_API_KEY`; `model: openrouter/<id>`, `baseURL: https://openrouter.ai/api/v1` |
| Ollama | no key; an inline `ollama` provider (`npm: @ai-sdk/openai-compatible`, `baseURL: http://localhost:11434/v1`, the model), `model: ollama/<id>` |

The inline configuration pins `baseURL`, so a project's `opencode.json` cannot send
the key the app supplies to another host. The key is only in its variable, never in
the configuration text. **Controlled:** `OPENCODE_CONFIG_CONTENT` and the four key
variables. OpenCode is not installed on the development machine, so this adapter is
implemented from its documentation and unit-tested, but not verified against a
running OpenCode.

### Codex

Codex speaks only the OpenAI Responses API (`wire_api = "responses"`; it
refuses `"chat"`), so it can use OpenAI. It is configured through its
documented command-line options only, for the run:

- `-c` overrides, which take precedence over `~/.codex/config.toml`, declare a
  model provider of the app's, `x8ai`: `name`, `base_url` (the provider's
  endpoint, pinned; a provider in the user's or the project's Codex
  configuration cannot take the key elsewhere), `wire_api = "responses"`, and
  `env_key = "X8AI_CODEX_API_KEY"`. Then `model_provider = "x8ai"` selects it.
- `-m <model>`.
- The key, in `X8AI_CODEX_API_KEY`, in that session's environment only. The
  shell's `OPENAI_API_KEY`, `OPENAI_BASE_URL`, `CODEX_API_KEY`,
  `OPENAI_ORGANIZATION` and `OPENAI_PROJECT` are removed (ADR 0015).

Codex's files (`~/.codex`) are never written by the app. Without a model chosen
in the app, Codex runs with its own configuration, as before. MCP servers and
skills are not supported for it. Verified against Codex 0.153 (a request with
an invalid key reached `https://api.openai.com/v1/responses` and was refused
with `invalid_api_key`).

### Opening a model from the Catalog

A model whose provider is set up can be dragged from the Catalog onto the
terminal, or opened with **Open in a terminal**. It opens in the installed
agent from the provider's own publisher (Codex for OpenAI, Claude Code for
Anthropic, as the catalog's metadata says), or else the first installed agent
that can use it, through the usual trust and approval.

## Sessions

The model is chosen per session, at launch, in the Agents view: "Claude Code's own
configuration" (the default) or one of the supported providers' models. Only
providers the agent's adapter supports and that have what they need (a saved key,
or none needed) are offered.

| Session field | |
| --- | --- |
| Agent, Workspace, Worktree | as in Phase 5 (docs/multi-agent.md) |
| Provider, Model | `model: {provider, model}`; `None` for the agent's own configuration |
| Configuration source | `configuration`: `agent` (with the shell's provider variables, by name) or `app` (provider, model, endpoint, whether a key is saved, the shell variables replaced) |

The session keeps its model for every run: restart and "Terminal" relaunch with it,
with the key as the app keeps it (above). The model is saved in the
worktree's metadata (`~/.x8ai/worktrees/…/<name>.json`: provider id and model id,
never a key), so a session found after the app restarts keeps it. The catalog
(Phase 8, docs/catalog.md) lists the same providers and models from this
registry. "Use for the next launch" there only selects the model on the agent
card. A session runs
only with its own model (`Mismatch` otherwise). To use another model, start another
session.

## Approval and material changes

See ADR 0015. An approval (ADR 0012) covers exactly: the workspace, the agent, its
executable, its arguments, and now **the provider and the endpoint** the agent will
send code and the key to. Approvals for different providers of the same agent stand
side by side.

| Change | Material? | Why |
| --- | --- | --- |
| Another executable or other fixed arguments | yes | a different program (Phase 4) |
| Agent's own configuration → a provider, or back | yes | who configures the destination changes |
| Another provider | yes | code and a key go to someone else |
| Same provider, another endpoint | yes | code and a key go to another host |
| Another model of the same provider | no | same destination, same key; the id is checked and cannot become an option |
| A new key for the same provider | no | same destination; key rotation must not need re-approval |
| Model ids added or removed, names, descriptions, UI state | no | metadata |

## IPC

| Command | Does |
| --- | --- |
| `provider_list(checkLocal)` | Every provider: hosting, whether a key is saved, local availability (with `checkLocal`, Ollama is looked for first), models |
| `provider_set_credential(provider, key)` | Saves the key in the Keychain; returns the status, not the key |
| `provider_remove_credential(provider)` | Deletes it |
| `provider_add_model(provider, model)` / `provider_remove_model` | A model id the user knows, in `providers.json` |
| `agent_list` | Now also, per agent, which providers its adapter supports and why not |
| `agent_request_approval(agent, model)` | The dialog names the provider and endpoint |
| `agent_create_session(agent, model)` | The session, and its worktree, keep the model |
| `agent_run(session)` | Reads the key again; refused if it was removed |

## Files

| File | Holds | Never holds |
| --- | --- | --- |
| Keychain, `com.x8ai.workspace.providers` | API keys | — |
| `<app data>/providers.json` (0600) | model ids the user added, per provider | keys |
| `<app data>/agent-approvals.json` (0600) | agent, program, arguments, provider id, endpoint | keys, model ids |
| `~/.x8ai/worktrees/<repo>/<name>.json` | the worktree's agent, token, base commit, provider id and model id | keys |

## Security boundaries

| Rule | How |
| --- | --- |
| Keys only in the Keychain | `x8ai-secrets`; tests scan every file the app and a session touched for the key |
| The webview never receives a key | No command returns one; `ProviderStatus` has no field for one (tested) |
| A key reaches only the agent it was chosen for | Read when that session is created and run, placed in that process's environment only. The app's own environment never holds it, so shells do not inherit it (tested with a child's environment) |
| No key in logs or errors | Redacted `Debug` on every type that can hold one; errors never echo input; no logging of environments (reviewed: the native side logs two things, neither with an environment) |
| The destination is approved | Provider and endpoint are pinned in the approval; endpoints come from built-in definitions, never from the webview |
| The shell cannot redirect an app-configured session | Controlled variables are removed; Claude Code is told the host manages its provider; OpenCode's endpoint is pinned inline |
| Model ids cannot inject options | `is_model_id` natively, in the adapter, the settings store and worktree metadata |
| No network at startup | Ollama is probed only on request, on the loopback address; hosted providers are never contacted by the app |

**Honest limits.**

- Once an agent has a key, the agent decides what to do with it. It is in the
  agent's environment, so the commands the agent runs inherit it unless the agent
  removes it, and processes running as the user can read a process's environment
  (`ps -E`), as with any variable exported in a shell.
- A key is sent as the user typed it; the app does not check it with the provider
  (that would be a network request). A wrong key shows up as the agent's own
  authentication error.
- Claude Code's settings files can still set a background model alias; the main
  model is `--model`, which they cannot override.
- A project's `opencode.json` can still add other options to OpenCode's provider
  (OpenCode's own project-trust model); the endpoint and model are the app's.
- Model lists from hosted providers, key checks, local servers other than Ollama,
  model pulls and hardware-based suggestions are not implemented (roadmap).

## Tests

- `crates/providers`: definitions valid and unique, keys required for hosted
  providers, no unverified capabilities; model sources merged without duplicates;
  `providers.json` persists across reloads with only model ids, 0600, damaged file
  set aside; Ollama detection against a fake server (available with models,
  chunked, not Ollama, installed, app bundle, relative `PATH` entries).
- `crates/secrets`: redaction, validation, memory store; the real Keychain
  (`--ignored`): set, find from a new instance, replace, remove.
- `crates/agents/tests/providers.rs`: the agent's own configuration untouched;
  Claude Code with Anthropic, OpenRouter, Ollama; unsupported combinations with
  reasons; no adapter; missing key; model ids that look like options; OpenCode's
  inline configuration; nothing printed holds a key; approval for provider and
  endpoint but not model or key; a fake Claude Code receives exactly the
  configuration, nothing on disk holds the key, a shell does not inherit it, the
  model survives a restart and a session runs only with it; tampered metadata is
  ignored.
- `crates/workspace/tests/store.rs`: provider approvals side by side, endpoint
  pinned, old approvals still valid for the agent's own configuration.
- `src-tauri`: the provider list sent to the webview never holds a key.
- Frontend: the client sends a key only to save it and has no method to read one;
  model choices only for supported providers with a key; the key is kept out of
  stores and messages; removal asks; launch passes the model to approval and
  session.
- Live (`crates/agents/tests/live_providers.rs`, `--ignored`): the real Claude Code
  with an invalid key from a throwaway Keychain service.
