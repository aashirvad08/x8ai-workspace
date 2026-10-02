import type { AgentChanges } from "../contracts/generated/AgentChanges";
import type { AgentSessionInfo } from "../contracts/generated/AgentSessionInfo";
import type { ChangeKind } from "../contracts/generated/ChangeKind";

/**
 * Handing context from agent sessions to another (docs/multi-agent.md): the text
 * the user sends, composed from what the app knows about each session. Nothing
 * here reads an agent's own files or transcripts: only its changes (from Git)
 * and what its terminal shows.
 */

/** Most diff text taken from one session. */
export const MAX_DIFF_CHARS = 40_000;
/** Most terminal lines taken from one session. */
export const MAX_OUTPUT_LINES = 150;
/** Most text sent at once; more is cut, and says so. */
export const MAX_CONTEXT_CHARS = 120_000;

/** What one session contributes, as the user chose it. */
export interface ContextSource {
  readonly name: string;
  /** Its branch, or a description of where it works. */
  readonly where: string;
  /** Its changes, when "What changed" or "The diff" is chosen; `null` if they could not be read. */
  readonly changes: AgentChanges | null;
  /** Why its changes could not be read. */
  readonly changesProblem?: string | null;
  readonly includeChanges: boolean;
  readonly includeDiff: boolean;
  /** Its recent terminal output, when chosen; `null` when not chosen. */
  readonly output: string | null;
}

const LETTERS: Record<ChangeKind, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  untracked: "?",
  other: "~",
};

/** Escape sequences (CSI, OSC, other ESC sequences) and control characters but newlines and tabs. */
const ESCAPES = /\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-_]|[\x00-\x08\x0b-\x1f\x7f-\x9f]/g;

/** Terminal text without escape sequences or control characters; carriage returns end lines. */
export function stripAnsi(text: string): string {
  return text.replace(/\r\n?/g, "\n").replace(ESCAPES, "");
}

/**
 * Terminal output as it is worth reading: plain text, trailing spaces gone,
 * runs of blank lines folded, at most `maxLines` lines from the end.
 */
export function tidyOutput(text: string, maxLines = MAX_OUTPUT_LINES): string {
  const lines = stripAnsi(text)
    .split("\n")
    .map((line) => line.trimEnd());
  const folded: string[] = [];
  for (const line of lines) {
    if (line === "" && (folded.length === 0 || folded.at(-1) === "")) continue;
    folded.push(line);
  }
  while (folded.at(-1) === "") folded.pop();
  return folded.slice(-maxLines).join("\n");
}

/**
 * Text safe to paste into a terminal: no escape character at all, so nothing in
 * it can end a bracketed paste early (`ESC[201~`) and run what follows, no other
 * control characters, Unix line ends, and no trailing newline, so pasting
 * never presses Enter.
 */
export function pasteable(text: string): string {
  return stripAnsi(text).replace(/\x1b/g, "").replace(/\n+$/, "");
}

/** Lines added and removed per file, from a unified diff. */
export function diffStat(diff: string): Map<string, { added: number; removed: number }> {
  const stat = new Map<string, { added: number; removed: number }>();
  let file: { added: number; removed: number } | null = null;
  for (const line of diff.split("\n")) {
    const header = /^diff --git a\/(.+) b\/(.+)$/.exec(line);
    if (header) {
      file = { added: 0, removed: 0 };
      stat.set(header[2]!, file);
    } else if (!file || line.startsWith("+++") || line.startsWith("---")) {
      continue;
    } else if (line.startsWith("+")) {
      file.added++;
    } else if (line.startsWith("-")) {
      file.removed++;
    }
  }
  return stat;
}

function fence(text: string, language = ""): string {
  // A fence longer than any run of backticks inside, so the text cannot close it.
  const longest = Math.max(2, ...[...text.matchAll(/`+/g)].map((m) => m[0].length));
  const ticks = "`".repeat(longest + 1);
  return `${ticks}${language}\n${text}\n${ticks}`;
}

function changesSection(source: ContextSource): string[] {
  const out: string[] = [];
  const changes = source.changes;
  if (!changes) {
    out.push(`What changed: unknown${source.changesProblem ? ` (${source.changesProblem})` : ""}.`);
    return out;
  }
  if (source.includeChanges) {
    const commits = changes.commits === 1 ? "1 commit" : `${changes.commits} commits`;
    const files = changes.files.length === 1 ? "1 file" : `${changes.files.length} files`;
    out.push(
      `What changed since ${changes.base.slice(0, 10)}: ${files}, ${commits}${changes.uncommitted ? ", not all committed" : ""}.`,
    );
    const stat = diffStat(changes.diff);
    for (const file of changes.files) {
      const lines = stat.get(file.path);
      const counts = lines ? ` (+${lines.added} −${lines.removed})` : "";
      out.push(`- ${LETTERS[file.change]} ${file.from ? `${file.from} → ` : ""}${file.path}${counts}`);
    }
  }
  if (source.includeDiff && changes.diff.trim() !== "") {
    const cut = changes.diff.length > MAX_DIFF_CHARS;
    const diff = cut ? `${changes.diff.slice(0, MAX_DIFF_CHARS)}\n…` : changes.diff.trimEnd();
    out.push("", `The diff${cut || changes.truncated ? " (truncated)" : ""}:`, fence(diff, "diff"));
  }
  return out;
}

/** The context text for the chosen sources and the user's note. */
export function composeContext(sources: readonly ContextSource[], note: string): string {
  const parts: string[] = [
    "Context from other agent sessions in this workspace, shared by the user in x8ai Workspace.",
    "Use it to continue their work; ask the user if anything is unclear.",
  ];
  for (const source of sources) {
    const section = [`## ${source.name} (${source.where})`];
    if (source.includeChanges || source.includeDiff) section.push(...changesSection(source));
    if (source.output !== null) {
      const output = tidyOutput(source.output);
      section.push("", `Its terminal, last ${MAX_OUTPUT_LINES} lines at most:`, output ? fence(output) : "(nothing shown)");
    }
    if (section.length > 1) parts.push("", ...section);
  }
  if (note.trim() !== "") parts.push("", "## Note from the user", note.trim());
  const text = parts.join("\n");
  return text.length > MAX_CONTEXT_CHARS ? `${text.slice(0, MAX_CONTEXT_CHARS)}\n… (cut: too long to send at once)` : text;
}

/** Where a session works, for the text and the composer: its branch, or the folder itself. */
export function sessionWhere(session: AgentSessionInfo): string {
  return session.worktree ? session.worktree.branch : "in the folder itself, not isolated";
}

/** A session's name with what tells it apart from others of the same agent. */
export function sessionLabel(session: AgentSessionInfo): string {
  const where = session.worktree ? session.worktree.branch.replace(/^agent\/[^/]+\//, "") : "your folder";
  return `${session.name} · ${where}`;
}

/** The sessions `/get <name>` or `/give <name>` means: by agent name or id, or part of the branch. */
export function matchSessions(arg: string, sessions: readonly AgentSessionInfo[]): AgentSessionInfo[] {
  const wanted = arg.trim().toLowerCase();
  if (wanted === "") return [];
  return sessions.filter(
    (s) =>
      s.name.toLowerCase().startsWith(wanted) ||
      s.agent.toLowerCase().startsWith(wanted) ||
      (s.worktree?.branch.toLowerCase().includes(wanted) ?? false),
  );
}
