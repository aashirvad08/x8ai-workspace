# 0016. Providers as data, agents configured by adapters

**Status:** Accepted (Phase 6)

## Context

The app hosts several agents (Claude Code, OpenCode, Codex…) and several providers
(Anthropic, OpenAI, Google, OpenRouter, Ollama). Each agent is configured
differently, and not every agent can use every provider. The design must not
couple the app to one agent, must not put agent checks throughout the code, must
not guess undocumented flags, and must not claim model capabilities it cannot
verify. It must also not make network requests on the user's behalf just to list
things.

## Decision

**Providers are data** (`crates/providers/src/builtin.json`,
`ModelProviderDefinition`): hosting (hosted, gateway, local), endpoints as
(wire API, base URL) pairs, auth (none, or an API key by secret name) and the model
ids known to exist. Nothing about agents. Models come from the definition, from
the local server (Ollama), or from the user (ids in `providers.json`). No context
window, price or speed is recorded unless verified. Gateways (OpenRouter) are not
enumerated: the user adds the ids they use.

**Agents are configured by adapters** (`crates/agents/src/adapter/`), one per
agent behind one trait: which of a provider's endpoints the agent would use (or why
it cannot), which variables it controls, and the variables and arguments for a
provider, endpoint, model and key. Adapters are looked up by agent id in one table;
nothing else in the app compares agent ids. Each adapter cites the agent's own
documentation for every variable and flag it sets:

- **Claude Code:** Anthropic, OpenRouter and Ollama through their Anthropic
  Messages endpoints, with `ANTHROPIC_*` variables, `--model`, and
  `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST`. Not OpenAI or Google.
- **OpenCode:** all five, through each provider's key variable and
  `OPENCODE_CONFIG_CONTENT` (model and pinned endpoint; an inline Ollama provider).
- **Codex:** no adapter. It keeps its own configuration.

**The app does not proxy model traffic and does not call hosted providers.** Agents
talk to providers directly. Local discovery (Ollama) is two loopback HTTP requests,
only when the user asks. Model listing and key checks against hosted APIs are
deferred: they need an HTTPS client and are network requests the user should ask
for.

**Model selection is per session**, kept with the session (and its worktree's
metadata) so restarts reuse it.

## Consequences

- Adding a provider an adapter already knows how to reach is a JSON entry. Adding
  an agent that can be configured is one adapter.
- Supported combinations are exactly what the adapters say; the UI offers only
  those, with reasons for the rest.
- The OpenCode adapter is written from documentation and unit-tested; it has not
  run against OpenCode, which is not installed on the development machine.
- No automatic model lists for OpenAI, Google or OpenRouter until discovery exists.
- A local gateway translating between wire APIs (so Claude Code could use OpenAI)
  remains possible later, with its own ADR: it would put the app on the data path
  of every prompt.

## Alternatives considered

- **Compatibility by `ProviderApi` alone** (the Phase 0 sketch). Necessary but not
  sufficient: sharing a wire API does not say how the agent authenticates to that
  provider (Claude Code needs a bearer token for OpenRouter and an empty API key,
  a placeholder token for Ollama). The adapter decides, using the API match.
- **Generating the agents' config files** (`~/.claude/settings.json`,
  `opencode.json`). Writes into the user's or the project's configuration, outlives
  the session, and can carry a key into a commit. Rejected: environment and
  documented inline configuration only.
- **Hard-coding model capabilities** for display. Unverifiable and quickly wrong.
  Rejected.
- **Listing OpenRouter's catalog.** Hundreds of changing entries, a network
  request, and catalog territory (Phase 8). Rejected for now.
