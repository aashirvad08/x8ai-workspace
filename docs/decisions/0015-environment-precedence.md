# 0015. Environment precedence, and what an approval covers

**Status:** Accepted (Phase 6)

## Context

An agent reads its provider configuration from its environment and its own
settings. The user's shell may already configure one (`ANTHROPIC_API_KEY`,
`ANTHROPIC_BASE_URL`, `CLAUDE_CODE_USE_BEDROCK`…), and the app can now configure
another. Mixing them silently gives surprising results: the app's key sent to the
shell's proxy, or the shell's model with the app's provider. And approval (ADR 0012)
must say whether a change of provider or model needs the user again, without
asking for every harmless change.

## Decision

**Two modes, never mixed.**

- **Agent's own configuration** (no model chosen; the default, and Phase 4/5
  behaviour): the login environment is passed unchanged. No Keychain key is added.
  The session shows, by name, which provider variables the shell sets.
- **App-configured** (a provider and model chosen): every variable the agent's
  adapter controls (those that choose its provider, endpoint, credentials or model)
  is removed from the login environment, then the adapter's are set. All other
  variables are kept, so the shell's `PATH`, tools, proxies and unrelated keys
  still work. The app's configuration wins entirely; the session and the approval
  dialog name the shell variables it replaced.

Each adapter defines what it controls from the agent's documentation. For Claude
Code this is every `ANTHROPIC_*` variable plus its provider switches, OAuth and
subagent-model variables; it also sets `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST=1`, so
Claude Code ignores provider, endpoint and credential variables in its settings
files. For OpenCode it is `OPENCODE_CONFIG_CONTENT` and the key variables it reads;
the inline configuration pins the endpoint.

**Material changes need approval again.** An approval pins the workspace, agent,
executable, arguments, and now the **provider and endpoint** (the destination of
the code and the key). Approvals for several providers of one agent coexist.

| Material | Not material |
| --- | --- |
| executable, fixed arguments | the model, within the same provider and endpoint |
| own configuration ↔ a provider | a new key for the same provider |
| another provider | model ids added or removed, names, UI state |
| same provider, another endpoint | |

The model is not material because it does not change where code or the key goes.
It is passed as a checked model id (`is_model_id`: never starting with `-`), after
the approved arguments, so it cannot become an option such as a permission bypass.
Key rotation is not material, so rotating a key is not a reason to click "Allow"
again.

## Consequences

- A session configured in the app behaves the same whatever the shell exports; a
  session using the agent's own configuration behaves exactly as before Phase 6.
- Approvals from Phase 4 and 5 remain valid for the agent's own configuration; the
  first launch with a provider asks once per provider and workspace.
- A new endpoint for a built-in provider (an app update) asks again, by design.
- Removing a controlled variable can remove something the user wanted for that
  agent (for example `ANTHROPIC_BETAS`). They can use the agent's own configuration
  instead.

## Alternatives considered

- **Shell wins.** The app's choice would silently not apply whenever the shell set
  something. Rejected.
- **Merge, app variables overriding only the ones they set.** Leaves the shell's
  endpoint or provider switch in place next to the app's key: the most surprising
  result. Rejected.
- **Strip every provider variable of every agent from every session.** Changes the
  behaviour of sessions the app does not configure, and of other agents. Rejected.
- **The model is material.** Every model switch would ask again for the same
  destination and key, training the user to click "Allow". Rejected.
- **Any change is material, including key rotation.** Same objection.
