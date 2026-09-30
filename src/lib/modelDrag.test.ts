import { describe, expect, it } from "vitest";

import { carriesModel, MODEL_DRAG_TYPE, readModelDrag, setModelDrag } from "./modelDrag";

function transfer(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  return {
    setData: (type: string, value: string) => void data.set(type, value),
    getData: (type: string) => data.get(type) ?? "",
    get types() {
      return [...data.keys()];
    },
  };
}

describe("model drag", () => {
  it("carries the provider and model, and reads them back on drop", () => {
    const t = transfer();
    setModelDrag(t, { provider: "openai", model: "gpt-6.1-sol", name: "gpt-6.1-sol" });
    expect(carriesModel(t)).toBe(true);
    expect(t.getData("text/plain")).toBe("gpt-6.1-sol");
    expect(readModelDrag(t)).toEqual({ provider: "openai", model: "gpt-6.1-sol", name: "gpt-6.1-sol" });
  });

  it("ignores anything else that is dropped", () => {
    expect(carriesModel(transfer({ "text/plain": "rm -rf /" }))).toBe(false);
    expect(readModelDrag(transfer({ "text/plain": "x" }))).toBeNull();
    expect(readModelDrag(transfer({ [MODEL_DRAG_TYPE]: "not json" }))).toBeNull();
    expect(readModelDrag(transfer({ [MODEL_DRAG_TYPE]: JSON.stringify({ provider: 1, model: "m", name: "m" }) }))).toBeNull();
    expect(readModelDrag(transfer({ [MODEL_DRAG_TYPE]: JSON.stringify({ provider: "openai", model: "", name: "m" }) }))).toBeNull();
    expect(readModelDrag(transfer({ [MODEL_DRAG_TYPE]: JSON.stringify({ provider: "x".repeat(65), model: "m", name: "m" }) }))).toBeNull();
  });
});
