import { memo, type ReactNode, useEffect, useRef, useState } from "react";

import type { SearchMatch } from "../contracts/generated/SearchMatch";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import { basename, dirname } from "../lib/paths";
import type { Store } from "../lib/store";
import { useStore } from "../lib/useStore";
import type { SearchActions } from "./actions";
import type { FileMatches, Search, SearchSnapshot } from "./search";

interface Props {
  search: Search;
  workspace: Store<WorkspaceInfo | null>;
  actions: SearchActions;
}

/** Plain-text search across the open folder (⇧⌘F). */
export const SearchView = memo(function SearchView({ search, workspace, actions }: Props) {
  const info = useStore(workspace);
  const snapshot = useStore(search);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, [snapshot.focusRequest]);

  if (!info) {
    return (
      <div className="sidebar-empty">
        <p>Open a folder to search in it.</p>
      </div>
    );
  }

  return (
    <div className="search" role="search">
      <div className="search-field">
        <input
          ref={input}
          className="search-input"
          placeholder={`Search in ${info.name}`}
          aria-label="Search text"
          spellCheck={false}
          value={snapshot.text}
          onChange={(event) => search.setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") void search.run();
          }}
        />
        <button
          type="button"
          className={snapshot.caseSensitive ? "search-option search-option-on" : "search-option"}
          aria-pressed={snapshot.caseSensitive}
          title="Match Case"
          aria-label="Match case"
          onClick={() => search.setCaseSensitive(!snapshot.caseSensitive)}
        >
          Aa
        </button>
      </div>
      <p className={snapshot.status === "failed" ? "search-status search-error" : "search-status"} role="status">
        {statusText(snapshot)}
      </p>
      {/* A new search starts with every file expanded. */}
      <Results key={snapshot.run} results={snapshot.results} actions={actions} />
    </div>
  );
});

function Results({ results, actions }: { results: readonly FileMatches[]; actions: SearchActions }) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const toggle = (path: string) =>
    setCollapsed((current) => {
      const next = new Set(current);
      if (!next.delete(path)) next.add(path);
      return next;
    });

  return (
    <div className="search-results">
      {results.map(({ path, matches }) => {
        const open = !collapsed.has(path);
        return (
          <div key={path} className="search-file" role="group" aria-label={path}>
            <button type="button" className="search-file-header" aria-expanded={open} title={path} onClick={() => toggle(path)}>
              <span className={open ? "tree-chevron tree-open" : "tree-chevron"}>›</span>
              <span className="search-file-name">{basename(path)}</span>
              <span className="search-file-dir">{dirname(path)}</span>
              <span className="search-count">{matches.length}</span>
            </button>
            {open &&
              matches.map((match) => (
                <button
                  key={match.line}
                  type="button"
                  className="search-match"
                  title={`${path}:${match.line}:${match.column + 1}`}
                  onClick={() => actions.openMatch(path, match)}
                >
                  <span className="search-line">{match.line}</span>
                  <Preview match={match} />
                </button>
              ))}
          </div>
        );
      })}
    </div>
  );
}

/** The line with every match marked. */
function Preview({ match }: { match: SearchMatch }) {
  const parts: ReactNode[] = [];
  let at = 0;
  for (const [start, end] of match.ranges) {
    if (start > at) parts.push(match.preview.slice(at, start));
    parts.push(<mark key={start}>{match.preview.slice(start, end)}</mark>);
    at = end;
  }
  parts.push(match.preview.slice(at));
  return <span className="search-preview">{parts}</span>;
}

function statusText({ status, text, results, summary, error }: SearchSnapshot): string {
  switch (status) {
    case "idle":
      return text === "" ? "Matches text literally. Skips ignored, generated and binary files." : "";
    case "searching":
      return results.length === 0 ? "Searching…" : `Searching… ${plural(results.length, "file")} so far`;
    case "failed":
      return `Search failed: ${error}`;
    case "done": {
      if (!summary || summary.matches === 0) return "No results.";
      const found = `${plural(summary.matches, "result")} in ${plural(summary.files, "file")}`;
      return summary.truncated ? `${found}. Stopped at the result limit; refine the search to see the rest.` : `${found}.`;
    }
  }
}

function plural(count: number, noun: string): string {
  return `${count.toLocaleString()} ${noun}${count === 1 ? "" : "s"}`;
}
