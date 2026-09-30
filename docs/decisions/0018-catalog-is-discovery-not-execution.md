# 0018. The catalog is discovery and orchestration, not execution

**Status:** Accepted (Phase 8)

## Context

Phase 8 adds a catalog: one place to see every agent, model provider, model,
MCP server and skill the app knows, whether each is ready, and what it needs.
By this point each kind of item already has a system that owns it, with its own
rules:

- the **agent runtime** (ADR 0012) finds programs on the login `PATH`, and runs
  them only in a trusted folder, once approved there;
- the **provider registry** (ADR 0016) keeps definitions, with keys only in the
  Keychain (ADR 0014);
- the **MCP registry** (ADR 0017) keeps servers the user added, and starts them
  only for a session, once approved, when the agent connects;
- **skills**, new in this phase, are instructions for an agent session.

A catalog could become a second way to do what these systems do: a second
configuration for Claude Code, a way to start a server "to check it", an
installer, or a place that reads a key "to test it". Each of these would
bypass rules that took a phase to get right. The questions:

1. Can the catalog **execute** anything: a program, a shell command, a server?
2. Can it **grant** anything: trust or approval?
3. Can it **read secrets**?
4. Can it **install** software?
5. Where does an item's **truth** live: in catalog metadata, or in its system?

## Decision

**The catalog is discovery and orchestration, not execution.** It lists what the
owning systems report and sends the user to them. It cannot execute, grant,
unlock or install.

- **It cannot execute.** `x8ai-catalog` is a pure crate. It depends on the
  contracts and serialization only, and not on the PTY, the agent or MCP
  runtimes, the Keychain, the workspace stores, an HTTP client or Tauri. Its
  output is data. The desktop command `catalog_list` only gathers statuses the
  systems already report: whether the runtime finds a program (no program is
  run to find its version), a provider's key state and Ollama's last detection
  (never probed from the catalog), MCP servers' configuration (none is started).
  Tests check the crate's manifest and source, and the command's source.
- **It cannot grant approval or trust.** Nothing in it can reach the trust or
  approval stores. Choosing a model, server or skill in the catalog only fills in
  the next launch's choices on the agent card. Launching still goes through the
  single approval dialog (agent, model, MCP servers, skills, folder), and a
  server still starts only as ADR 0017 decided.
- **It cannot access secrets.** It sees credential *states* (saved or not),
  never values, as the rest of the webview does. Skills, which are text the agent
  and the user can read, refuse anything that looks like a key.
- **It cannot install arbitrary software.** It has no installer, no package
  manager, no downloads, no URLs. An agent that is not installed is shown as
  such, and the user installs it. Metadata that claims to be remote is refused.
- **The underlying systems stay authoritative.** An item is built from the facts
  its system reports. The catalog's own metadata is presentation only (publisher
  when known, tags, a metadata version), with closed fields: it cannot carry a
  command, an endpoint or a setting. Metadata that matches nothing the app has is
  not shown. An item is never *installed* because metadata describes it.

**Skills are instructions, not programs.** A skill is a name, a description,
instructions, suggested tools (shown, never granted), a source and a scope.
Skills are stored in their own registry (`skills.json`), apart from provider and
MCP data and the Keychain. They reach an agent through its adapter, for one
session: Claude Code's documented `--append-system-prompt`. They are
unsupported for OpenCode, which reads extra instructions only from files, and
for Codex, which has no adapter. A session records each skill's id, version and
fingerprint. It runs with exactly those, or refuses and says why: a skill is
never silently upgraded or substituted.

**A future remote catalog** fits behind seams in `x8ai-catalog`
(`MetadataSource`, `SignedMetadata`, `Verifier`): signed metadata, verified
against keys the user trusts, and packages installed only through the system
that owns them, behind that system's approval. None of this is implemented.

## Consequences

- One source of truth per item. A change made in Models, MCP or Agents appears
  in the catalog, and a catalog action is a request to the same command those
  views use, so there is no drift to reconcile.
- The catalog is safe to open at any time, including at startup or in an
  untrusted folder. It starts nothing, probes nothing and connects nowhere. It
  reads the login environment that the Agents view already reads.
- The catalog can say less than a marketplace would. There is no installed
  version unless a system already knows it, no "install" button, and no list of
  MCP servers to pick from. ADR 0017 expected the catalog to bring pinned,
  reviewed MCP definitions. That needs the signed remote design, so it waits for
  it, rather than shipping definitions that would run code on a click.
- Codex is now a built-in agent definition with no adapter. The runtime can say
  whether it is installed. It launches with its own configuration only, and the
  catalog shows it offering no model, MCP or skill choices.
- Skills are visible on Claude Code's command line to the user's own processes.
  They must not hold secrets, and validation refuses what looks like one.
- Suggested tools are informational. The app cannot restrict an agent's tools,
  and says so.

## Alternatives considered

- **Catalog entries carrying full definitions** (commands, endpoints, arguments)
  that the catalog registers or runs. This would be a second configuration path
  around each system's validation and approval, and the catalog would become an
  installer. Rejected for this phase. A future remote catalog would deliver
  verified packages to the owning system instead.
- **Detect installed versions by running `--version`.** This executes a program
  just to show a number, and would run on every catalog open. Rejected. Versions
  are shown only where a system already learns them (Ollama).
- **Probe Ollama from the catalog.** This breaks the Phase 6 rule that Ollama is
  checked only when the user asks. Rejected. The last detection is shown.
- **Built-in MCP servers** (GitHub, Playwright) in the catalog. Selecting one
  would run `npx` packages the app did not review or pin. Rejected until
  signed, pinned metadata exists.
- **Skills written into the agent's files** (`CLAUDE.md`, `~/.claude/skills`,
  OpenCode's instructions file). These change the user's or the project's
  configuration, outlive the session and can be committed. Rejected for the
  documented per-session flag. OpenCode stays unsupported.
- **Skills that bundle scripts or grant tools.** A skill would then be code and a
  permission. Rejected: skills are text.
