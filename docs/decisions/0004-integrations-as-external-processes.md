# 0004 — Integrations are external processes described by declarative definitions

**Status:** Accepted (Phase 0, 2026-09-28)

## Context

The product hosts agents (Claude Code, OpenCode, Codex, Aider, …), model providers
(Anthropic, OpenAI, Google, OpenRouter, Ollama, …) and MCP servers (GitHub,
Playwright, databases, …). The set will grow, and it will eventually come from a
catalog. We must not build our own agent or LLM, and we must not couple the app to
any one vendor.

## Decision

1. **Agents and stdio MCP servers are external programs** started by the native
   layer from an explicit `LaunchSpec` (program, args, env). No shell
   interpolation. No in-process plugins.
2. **Agents run inside terminal sessions.** Every target agent is an interactive
   terminal program, so the Phase 1 PTY session is the agent substrate. The agent
   runtime (Phase 4) is a thin layer over it. Structured agent protocols can be
   added later as a second runtime kind.
3. **Integrations are data first.** `AgentDefinition`, `ModelProviderDefinition` and
   `McpServerDefinition`, unified as `IntegrationDefinition`, live in `x8ai-core`.
4. **Compatibility is by protocol, not by name.**
   - Agents declare the model APIs they speak (`ProviderApi`:
     `anthropicMessages`, `openAiChatCompletions`, `openAiResponses`, `gemini`).
     Providers declare the endpoints they serve. They are compatible when they
     share an API.
   - Agents declare the MCP transports they support (`stdio`, `streamableHttp`).
5. **Per-agent configuration quirks** (flags, environment variables, config file
   formats) are handled by small Rust adapters in later phases. They are never
   handled in the UI, and never with vendor checks scattered through the code.
6. **Secrets are references** (`SecretName`) in definitions, never values.
7. **No unenforceable permission declarations.** Definitions do not include
   filesystem or network "permission" fields, because nothing can enforce them on
   a process running with the user's privileges. They record only what the app
   controls: the exact program, its arguments, its environment and the secrets it
   receives. Enforceable policy is added when enforcement exists (see
   `docs/security.md`).

## Consequences

- Claude Code is one definition among several. The fixture tests show that one
  local Ollama provider serves Claude Code, OpenCode and Aider through different
  protocols, with no special cases.
- A new agent, provider or server that uses known protocols needs only a
  definition.
- Some agent capabilities (for example rich tool approval UI) are out of reach
  until a structured runtime exists. This is accepted.
- Definitions reject unknown fields, so a typo or a newer manifest cannot silently
  drop a setting. Catalog versioning (`schemaVersion`) comes in Phase 10.

## Alternatives considered

- **Embed agent logic, calling model APIs ourselves:** contradicts the product
  principle, and duplicates work those tools do well.
- **Vendor-specific integration code per agent in the UI:** fast at first, but
  couples the UI to vendors and breaks the extension story.
- **Declaring filesystem and network permissions per integration now:** looks
  secure, but enforces nothing. Rejected as security theater.
