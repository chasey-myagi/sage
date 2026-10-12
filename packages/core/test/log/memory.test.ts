import { describe, expect, it } from "vitest";
import { MemoryLogStore } from "../../src/index.ts";

describe("MemoryLogStore", () => {
  it("appends 100 entries with monotonic seq and parent links", () => {
    const store = new MemoryLogStore();
    const sessionId = "s";
    const appended = [];
    for (let i = 0; i < 100; i++) {
      appended.push(
        store.append(sessionId, {
          kind: "message",
          payload: { role: "user", content: `m${i}` },
        }),
      );
    }

    expect(appended).toHaveLength(100);
    expect(store.all(sessionId)).toHaveLength(100);
    for (let i = 0; i < 100; i++) {
      const entry = store.all(sessionId)[i];
      expect(entry?.seq).toBe(i);
      expect(entry?.parentId).toBe(i === 0 ? null : appended[i - 1]?.id);
      expect(entry?.kind).toBe("message");
    }
    expect(store.leaf(sessionId)).toBe(appended[99]?.id);
  });

  it("moves the leaf with a leaf entry and does not copy history", () => {
    const store = new MemoryLogStore();
    const sessionId = "branch";
    const root = store.append(sessionId, {
      kind: "message",
      payload: { role: "user", content: "root" },
    });
    const second = store.append(sessionId, {
      kind: "message",
      payload: { role: "user", content: "second" },
    });
    const third = store.append(sessionId, {
      kind: "message",
      payload: { role: "user", content: "third" },
    });
    const before = store.all(sessionId).length;

    const pointer = store.append(sessionId, {
      kind: "leaf",
      parentId: third.id,
      payload: { target: root.id },
    });

    expect(store.leaf(sessionId)).toBe(root.id);
    expect(pointer.kind).toBe("leaf");
    expect(pointer.parentId).toBe(third.id);
    expect(store.all(sessionId)).toHaveLength(before + 1);

    const next = store.append(sessionId, {
      kind: "message",
      payload: { role: "user", content: "after branch" },
    });
    expect(next.parentId).toBe(root.id);
    expect(next.seq).toBe(before + 1);

    const chain = store.path(sessionId);
    expect(chain.map((entry) => entry.id)).toEqual([root.id, next.id]);
    expect(chain.some((entry) => entry.kind === "leaf")).toBe(false);
    expect(store.get(sessionId, second.id)?.payload).toEqual({
      role: "user",
      content: "second",
    });
    expect(store.all(sessionId).some((entry) => entry.id === second.id)).toBe(
      true,
    );
  });
});
