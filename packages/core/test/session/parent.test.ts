import { describe, expect, it } from "vitest";
import {
  AgentSession,
  InvalidEntryError,
  MemoryLogStore,
  passthroughView,
  UnknownEntryKindError,
  type ViewContext,
} from "../../src/index.ts";
import { createFaux, MODEL, SYSTEM, stopScript } from "../support/faux.ts";

function user(text: string) {
  return {
    role: "user" as const,
    content: [{ type: "text" as const, text }],
  };
}

describe("parent view context", () => {
  it("passes the parent path snapshot and depth into the view", async () => {
    const store = new MemoryLogStore();
    store.append("parent", { kind: "message", payload: user("from-parent") });
    const leafId = store.leaf("parent");
    if (!leafId) throw new Error("missing parent leaf");
    let seen: ViewContext | undefined;
    const { adapter } = createFaux([stopScript("child-done")]);
    const session = new AgentSession({
      store,
      sessionId: "child",
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [],
      parent: { sessionId: "parent", leafId, depth: 1 },
      viewProvider: {
        view(ctx) {
          seen = ctx;
          const base = passthroughView.view(ctx);
          if (base instanceof Promise) {
            throw new Error("passthrough view must be synchronous");
          }
          return base;
        },
      },
    });

    await session.run("hello child");

    if (!seen) throw new Error("view was not called");
    expect(seen.depth).toBe(1);
    expect(seen.parentPath?.map((entry) => entry.payload)).toEqual([
      user("from-parent"),
    ]);
    expect(seen.path.some((entry) => entry.id === leafId)).toBe(false);
    expect(JSON.stringify(seen.path)).toContain("hello child");
    expect(JSON.stringify(seen.path)).not.toContain("from-parent");
  });
});

describe("subtask entry kind", () => {
  it("accepts a subtask payload and rejects a malformed one", () => {
    const store = new MemoryLogStore();
    const entry = store.append("child", {
      kind: "subtask",
      payload: {
        parentSessionId: "parent",
        parentLeaf: "leaf-1",
        query: "look here",
        budget: 1000,
        readOnly: true,
      },
    });
    expect(entry.kind).toBe("subtask");
    expect(store.all("child")[0]?.payload).toEqual({
      parentSessionId: "parent",
      parentLeaf: "leaf-1",
      query: "look here",
      budget: 1000,
      readOnly: true,
    });
    expect(() =>
      store.append("child", {
        kind: "subtask",
        payload: { parentSessionId: "parent" },
      }),
    ).toThrow(InvalidEntryError);
    expect(() =>
      store.append("other", { kind: "not-subtask", payload: {} }),
    ).toThrow(UnknownEntryKindError);
  });
});
