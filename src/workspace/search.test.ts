import { describe, expect, it } from "vitest";

import type { SearchEvent } from "../contracts/generated/SearchEvent";
import type { SearchQuery } from "../contracts/generated/SearchQuery";
import { NativeError } from "../native";
import { Search } from "./search";

/** A native search the test finishes by hand. */
function fakeNative() {
  const searches: { query: SearchQuery; send: (event: SearchEvent) => void; finish: (error?: Error) => void }[] = [];
  let cancelled = 0;
  const native = {
    search: (query: SearchQuery, listener: (event: SearchEvent) => void) =>
      new Promise<void>((resolve, reject) => {
        searches.push({ query, send: listener, finish: (error) => (error ? reject(error) : resolve()) });
      }),
    cancelSearch: async () => void cancelled++,
  };
  return { native, searches, cancelled: () => cancelled };
}

const done = (files: number, matches: number): SearchEvent => ({ type: "done", files, matches, truncated: false, cancelled: false });
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("search", () => {
  it("shows only the latest search's results", async () => {
    const { native, searches } = fakeNative();
    const search = new Search(native);
    search.setText("old");
    const first = search.run();
    search.setText("new");
    const second = search.run();

    searches[0]!.send({ type: "file", path: "stale.txt", matches: [] });
    searches[0]!.send(done(1, 0));
    searches[0]!.finish();
    searches[1]!.send({ type: "file", path: "fresh.txt", matches: [] });
    searches[1]!.send(done(1, 0));
    searches[1]!.finish();
    await Promise.all([first, second]);

    expect(searches.map((s) => s.query.text)).toEqual(["old", "new"]);
    expect(search.get().results.map((r) => r.path)).toEqual(["fresh.txt"]);
    expect(search.get().status).toBe("done");
  });

  it("reports a failed search", async () => {
    const { native, searches } = fakeNative();
    const search = new Search(native);
    search.setText("x");
    const running = search.run();
    searches[0]!.finish(new NativeError("workspace_search", "notFound", "no workspace is open"));
    await running;
    expect(search.get()).toMatchObject({ status: "failed", error: "no workspace is open" });
  });

  it("clears and cancels when the text is emptied", async () => {
    const { native, cancelled } = fakeNative();
    const search = new Search(native);
    search.setText("x");
    void search.run();
    search.setText("");
    await search.run();
    await settle();
    expect(search.get()).toMatchObject({ status: "idle", results: [] });
    expect(cancelled()).toBe(1);
  });

  it("searches again with the new case setting", () => {
    const { native, searches } = fakeNative();
    const search = new Search(native);
    search.setText("Foo");
    void search.run();
    search.setCaseSensitive(true);
    expect(searches.map((s) => s.query)).toEqual([
      { text: "Foo", caseSensitive: false },
      { text: "Foo", caseSensitive: true },
    ]);
  });
});
