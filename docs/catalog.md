# Catalog

The catalog (⇧⌘K) lists everything the app knows how to use: agents, model
providers and their models, MCP servers, and skills. For each item it shows what
the item is, where it comes from, what it needs and whether it is ready. It
searches, filters, and sends the user to the part of the app that owns an item
when there is something to do. The catalog itself does nothing else: it starts
no process, contacts no server, reads no secret, installs nothing, and cannot
trust a folder or approve anything.

Built in Phase 8. Decision: ADR 0018. It builds on the agent runtime
(docs/agent-runtime.md, docs/multi-agent.md), the provider layer
(docs/models.md) and the MCP layer (docs/mcp.md). Phase 8 also adds **skills**,
which are described below.

## Architecture

```
 Catalog view ──catalog_list──▶ catalog.rs (desktop) ──▶ x8ai-catalog::assemble(metadata, facts)
                                       │ facts, from each owning system:
                                       ├─ agent runtime   x8ai_agents::status      (program on the login PATH?)
                                       ├─ providers       x8ai_providers::status   (key saved? Ollama last seen?)
                                       ├─ MCP registry    Mcp::statuses            (enabled, configured, supported)
                                       └─ skill registry  Skills::statuses         (which agents take skills)
                                         metadata: crates/catalog/src/builtin.json (publisher, tags, version)

 acting on an item ──▶ that system's own command, and its own checks
   agent         Open in Agents
   provider      Set up in Models (keys: provider_* commands, Keychain)
   model         Use for the next launch  → the agent card's model choice
   MCP server    Enable / Disable (mcp_set_enabled), Configure in MCP,
                 Attach to the next launch → the agent card's server choice
   skill         Attach to the next launch → the agent card's skill choice,
                 New / Edit / Remove (skill_* commands)

 launch (unchanged): agent card ──▶ approval dialog (agent + model + MCP + skills + folder) ──▶ session
```

`x8ai-catalog` is a pure crate. It depends on the contracts (`x8ai-core`) and
serialization only: no PTY, agent runtime, MCP runtime, Keychain, workspace
stores, HTTP client or Tauri. A test checks its manifest and source for each of
these, and for process, socket and file-writing APIs. The desktop command that
calls it, `catalog_list`, only gathers the status each system already reports.
A test checks its source too.

## Item types

| Type | Id | Comes from |
| --- | --- | --- |
| Agent | `agent.<agent id>` (`agent.claude-code`, `agent.opencode`, `agent.codex`) | The agent runtime's built-in definitions (`crates/agents/src/builtin.json`) |
| Model provider | `provider.<provider id>` (`provider.anthropic`, … `provider.ollama`) | The provider registry (`crates/providers/src/builtin.json`) |
| Model | `model.<provider id>.<model id>` (`model.anthropic.claude-sonnet-5`) | The provider registry: built-in ids, models Ollama reported, ids the user added |
| MCP server | `mcp.<server id>` (`mcp.github`) | The MCP registry: servers the user added |
| Skill | `skill.<skill id>` (`skill.tests-first`) | The skill registry: built-in and the user's |

Providers are listed with models (the Models category), since a provider is
what a model is set up through.

An item has an id, a name and a display name, a description, its type, a catalog
version and an installed software version, a publisher, a source, capabilities,
tags, a status (with a detail), requirements, and typed details
(`CatalogDetails`: the agent's executable and what it supports; a provider's
hosting and key state; a model's provider and model id; a server's transport,
scope and enabled state; a skill's version, source, scope and suggested tools).

Ids are stable: they are derived from the owning system's own ids, so the same
item has the same id every time, and nothing is keyed by a display name.

### What an item claims

Only what its system knows. A publisher is shown only where the metadata states
one (Claude Code: Anthropic; Codex: OpenAI; the hosted providers; the app for
built-in skills). OpenCode has none. The user's MCP servers, model ids and
skills have none. A model's publisher is shown only for a hosted provider's
built-in models, since a gateway's or a user's model id says nothing about who
made it. Capabilities come from definitions: the APIs and transports an agent
declares, and whether the app can give it a model, MCP servers and skills. A
model shows a context window only if its definition has one. No context windows,
benchmarks, prices or latencies are invented.

The **catalog version** is the version of the catalog's metadata for an item (or
a skill's own version). The **installed software version** is the version of the
installed program. It is shown only when a system already knows it (Ollama
reports its version when Models checks it). Otherwise it reads "not checked":
the catalog runs no program to ask. The two are never confused.

## Statuses

| Status | Agent | Provider | Model | MCP server | Skill |
| --- | --- | --- | --- | --- | --- |
| **Installed** | The runtime found its program on the login `PATH` | Ollama found, not running | — | — | In the app, and an agent here takes skills |
| **Configured** | — | Hosted: API key in the Keychain. Ollama: running | Its provider is configured | Enabled and nothing missing | — |
| **Available** | — | Hosted: no key yet. Ollama: not checked yet | Its provider is not | Enabled, a secret or setting missing | — |
| **Unavailable** | Program not found | Ollama not found | — | Disabled | — |
| **Unsupported** | Not for this operating system | — | — | No agent the app can configure supports it | No agent here takes skills |

An item is never *Installed* because metadata describes it: the catalog has
metadata for Codex, and Codex is *Installed* only when the runtime finds `codex`.
Metadata that matches nothing the app has (an agent that is not defined, a
provider that was removed) is not shown. The catalog reports a warning instead.

**Ollama stays lazy.** The catalog uses the last detection Models made and never
probes it. Until the user opens Models (or refreshes there), Ollama is
*Available*, "Not checked yet". The catalog never starts, installs or pulls
anything for Ollama.

## Sources

`CatalogSource`: **Builtin** (ships with the app: agents, providers, built-in
models and skills), **Local** (found on this machine: models Ollama reported),
**UserDefined** (added by the user: MCP servers, model ids, skills) and
**Remote**. Remote exists as a value only, for the future design below. Metadata
that claims to be remote is refused when it is read, and nothing fetches any.

The catalog's own metadata (`crates/catalog/src/builtin.json`) holds only
presentation: an id, a publisher when known, a version, a source and tags. Its
fields are closed (`deny_unknown_fields`), so an entry cannot carry a command, a
URL or a setting. Reading it refuses invalid ids, duplicates, remote sources,
control characters and oversized text. A test feeds it damaged and malicious
entries.

## Ownership: one source of truth

| Item | Owned by | The catalog |
| --- | --- | --- |
| Agents | Agent runtime: `AgentDefinition`, found on `PATH`, adapters | Shows its status; "Open in Agents" |
| Providers, models | Provider registry: `ModelProviderDefinition`, `ModelDefinition`, keys in the Keychain | Shows its status; "Set up in Models", "Use for the next launch" |
| MCP servers | MCP registry: `McpServerDefinition`, secrets, approvals | Shows its status; Enable/Disable through `mcp_set_enabled`, "Configure in MCP", "Attach to the next launch" |
| Skills | Skill registry | Shows it; New/Edit/Remove through the `skill_*` commands, "Attach to the next launch" |

There is no second copy: the catalog has no Claude Code configuration, no
provider endpoints and no server commands. Changing something means asking its
system, through the same command the rest of the app uses, with that system's
validation, dialogs and approvals. After such a change the catalog is listed
again.

MCP: there are no built-in MCP servers. The catalog shows the servers the user
configured. It adds none, and selecting one runs nothing: a server still runs
only as Phase 7 decided (catalog → MCP configuration → folder trust → approval
of exactly what runs → session → started when the agent connects).

## Skills

A skill is **instructions** an agent gets for a session: text, never a program.

- **Fields:** id, name, description, version, instructions (up to 16 KiB,
  required), suggested tools (at most 50, shown only and never granted), source
  (built-in or the user's), scope (every session, sessions in one folder, or only
  when chosen at launch).
- **Built-in skills:** *Explain before editing*, *Tests first*, *Python
  debugging*. They are session-scoped, version 1, read-only, and compiled into
  the app.
- **User skills:** in `skills.json` in the app's data directory (0600, replaced
  atomically). This file is separate from the MCP and provider files and from the
  Keychain. A damaged file is set aside as `.corrupt`. Entries that claim to be
  built in, or reuse an id, are dropped with a warning. At most 200. The id is
  derived natively from the name, and a workspace scope's folder is the open
  folder, both decided natively. Editing the name or instructions increments the
  version.
- **No secrets.** Validation refuses control characters, and any word that looks
  like a key or token (known prefixes such as `sk-`, `ghp_`, `xoxb-`, `AKIA`,
  followed by at least eight characters: the check MCP arguments get). A skill
  cannot hold a key, and nothing stores one next to it.
- **No effects.** A skill has no field that could change a provider, an MCP
  server, an agent's configuration, trust or approvals, and the skill commands
  touch only the skill registry. Suggested tools are text in the catalog. No
  permission is granted from them.

### How an agent gets them

Through its adapter, for the session only, like models and MCP servers:

- **Claude Code:** `--append-system-prompt <text>`, which Claude Code documents
  for interactive sessions. The text lists the session's skills, each under
  "## Skill: <name>". At most 64 KiB in all. Nothing is written to `~/.claude`,
  `CLAUDE.md` or the worktree.
- **OpenCode:** unsupported. It reads extra instructions only from files, and the
  app writes no files for it. Choosing a skill for it is refused, with this
  reason.
- **Codex:** unsupported (see below).

## Session integration

The launch flow is Phase 7's, with skills added:

```
 agent + model + MCP servers + skills + folder
   → approval dialog (lists all of them; the approval pins what runs)
   → session created (records what it was created with)
   → run
```

- **Choosing from the catalog** only fills in the agent card's choices for the
  next launch ("Use for the next launch", "Attach to the next launch"), for every
  installed agent that can use the item. The card shows what is chosen. Nothing
  launches until the user presses Launch, and then the approval dialog appears as
  before.
- **The approval dialog** lists the skills ("instructions only: they run nothing
  and need no approval"). Skills are not part of what an approval pins, since
  they run nothing. The agent's own permission prompts apply to whatever the
  agent does with them.
- **A session records** its agent, its model (provider and model id), its MCP
  server ids and its skills: each skill's id, version and a fingerprint of its
  name and instructions. The text stays in the registry. In a Git repository the
  worktree's metadata (`~/.x8ai/worktrees/…json`, outside the worktree) keeps the
  same, so a session is the same session after the app restarts. Nothing of the
  catalog or of a skill is written into the worktree.
- **Global and workspace skills** apply to new sessions of agents that take
  skills. Session skills apply only when chosen.
- **The session shows** its agent, model, MCP servers and skills (with versions).

### Removed, changed or unavailable items

A session never runs with something other than what it recorded, and never
silently switches to something else:

| Change | What happens |
| --- | --- |
| A recorded skill was removed | The session shows the skill as removed ("start a new session without it") and does not run: "the skill … was removed; start a new session without it". |
| A recorded skill changed (version or text) | The session shows the skill as changed ("now vN: start a new session to use it") and does not run until a new session is started. It is never upgraded silently. |
| A recorded MCP server was removed or changed | As in Phase 7: refused, or asks for approval again. |
| The model's key was removed, or its provider is gone | As in Phase 6: refused with the reason. |
| The agent is no longer installed | The runtime refuses, "not installed". |
| An agent cannot take something chosen for it | The launch is refused with the adapter's reason (for example, OpenCode and skills). |

## Codex

Codex is a built-in agent definition (`codex` on the `PATH`), so the runtime
can find it and the catalog can say whether it is installed. Its adapter gives
it an OpenAI model for a session (docs/models.md); it takes no MCP servers or
skills from the app. Its global configuration (`~/.codex`) is never written.

A model whose provider is set up can be dragged from the Catalog onto the
terminal, or opened with **Open in a terminal**: it opens in the agent that can
use it, through the usual trust and approval (docs/models.md).

## Search and filters

Local and instant, over the items already listed. Search matches every word of
the query against an item's name, display name, description, publisher, provider
name, type, tags and capabilities. The categories are All, Agents, Models, MCP
and Skills. A status filter picks any status, installed or configured, installed
only, or configured only. There is no remote search.

## Installation policy

The catalog installs nothing, in this phase and by design (ADR 0018):

- An agent that is not installed says so: "Not installed: `codex` was not found
  on your PATH. The app does not install programs; install it yourself, then
  refresh."
- A provider needs a key saved in Models, or Ollama installed and running by the
  user.
- An MCP server's command must already be installed.
- Skills are added as text in the app. There is no remote install, no package
  manager and no download.

## Security boundary

The catalog is **untrusted metadata** until a system that owns execution
approves it. A catalog entry never directly:

- executes a process or a shell command,
- starts an MCP server,
- changes workspace trust or grants an approval,
- reads a Keychain secret.

It can only ask the system that owns an item, through that system's command,
which applies its own checks. What the webview receives from the catalog holds
no secret: key and secret states only (desktop tests put a real key and a real
server secret in a store and check the catalog's JSON). Opening the catalog
starts nothing and makes no network connection. It reads the login environment
the Agents view already reads (cached for the app's run) to find programs on the
`PATH`.

## Future remote catalogs

Designed for, not implemented. A later phase can add:

```
 remote catalog → signed metadata → verification against keys the user trusts
                → verified package or artifact → installation (shown, approved, visible)
                → registered with the system that owns it (runtime, providers, MCP, skills)
```

The seams exist in `x8ai-catalog`: `MetadataSource` (only `Builtin` implements
it), `SignedMetadata` (bytes, detached signature, key id) and `Verifier`
(unimplemented). Metadata that claims `remote` is refused today. A remote
catalog would add presentation and, separately, verified packages. It would
still configure nothing directly, and every install would go through its owning
system and that system's approval.

Not built, and not to be built as shortcuts: a GitHub marketplace, npm or other
package-manager installs, arbitrary URLs, `curl`, remote code execution,
auto-update, telemetry, accounts, payments, ratings or download counts.

## IPC

| Command | Takes | Returns |
| --- | --- | --- |
| `catalog_list` | nothing | `CatalogList { items, warnings }` |
| `skill_list` | nothing | `SkillList { skills: SkillStatus[] }` (each with which agents take it) |
| `skill_add` | `skill: SkillInput` | `Skill` |
| `skill_update` | `id`, `skill: SkillInput` | `Skill` (version + 1 if its text changed) |
| `skill_remove` | `id` | nothing. Built-in skills cannot be changed or removed. |
| `agent_request_approval`, `agent_create_session` | now also `skills: string[]` (session skill ids) | as before |

`AgentStatus` gained `skills` (supported, with a reason) and `capabilities`.
`AgentSessionInfo` gained `skills: SessionSkill[]` (id, name, version, and
attached, changed or removed).

## Files

| File | Holds |
| --- | --- |
| `crates/catalog/` | `x8ai-catalog`: metadata, assembly, status rules, source seams |
| `crates/catalog/src/builtin.json` | Presentation metadata for built-in agents and providers |
| `crates/skills/` | `x8ai-skills`: the registry, attach and resolve |
| `crates/skills/src/builtin.json` | Built-in skills |
| `crates/core/src/{catalog,skill}.rs` | Contracts |
| `src-tauri/src/{catalog,skills}.rs` | Commands |
| `src/catalog/`, `src/skills/`, `src/agents/draft.ts` | The Catalog tab, the skill store, and the next launch's choices |
| `~/Library/Application Support/com.x8ai.workspace/skills.json` | The user's skills (0600) |

## Known limitations

- No installation of any kind (by design), and so no update notices: the
  installed software version is shown only where a system already knows it.
- No remote catalog, no MCP server definitions to pick from (GitHub, Playwright),
  no signed index: future work, as above.
- OpenCode and Codex take no skills, and Codex takes no MCP servers.
- A skill's suggested tools are informational: the app has no way to restrict an
  agent's tools, and does not pretend to.
- Skills are passed on Claude Code's command line (`--append-system-prompt`), so
  their text is visible to other processes of the user (`ps`), like any argument.
  They must not hold secrets, and validation refuses what looks like one.

## Tests

- `crates/catalog/tests/catalog.rs`: all four kinds with stable, unique ids;
  damaged and malicious metadata refused; metadata without an item not shown;
  status rules; agents installed only when the runtime finds them; models from
  the provider registry claiming nothing unverified; Ollama unchecked until Models
  checks it; MCP servers and skills from their registries; no secret in the
  output; agents without an adapter; the crate cannot run, install, fetch, unlock,
  trust or approve anything; metadata only from the app itself.
- `crates/skills/tests/skills.rs`: built-in skills valid and read-only, user
  skills persisted with versions, no secrets, damaged or forged files, which
  skills a session gets, and a session runs only with exactly what it recorded.
- `crates/agents/tests/skills.rs`: Claude Code gets skills through
  `--append-system-prompt` and nothing else changes; unsupported agents are
  refused; Codex is found, takes a model and no MCP or skills; a session runs only with its recorded
  skills; a worktree records skills by reference only.
- Desktop (`src-tauri/src`): session skill states, which agents are offered
  skills, the catalog JSON with a real key and a real secret in the stores, and
  the catalog command's source.
- Frontend: search and filters, the catalog store, launch drafts, skill choices,
  client commands (the catalog can only list), and the workbench (opening the
  catalog lists and probes nothing, and nothing happens at startup; choosing a
  model, a server or a skill fills the next launch without launching; launching
  with skills; adding, editing and removing skills).
