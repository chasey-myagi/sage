import { describe, expect, it } from "vitest";
import { AgentSession, MemoryLogStore } from "../../src/index.ts";
import {
  createFaux,
  MODEL,
  SYSTEM,
  stopScript,
  toolScript,
} from "../support/faux.ts";

function throwingTool(name: string, mode: "async" | "sync") {
  return {
    def: { name, description: name, inputSchema: { type: "object" } },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    execute() {
      if (mode === "sync") throw new Error("nope");
      return Promise.reject(new Error("nope"));
    },
  };
}

function toolResults(store: MemoryLogStore, sessionId: string) {
  const blocks: { type?: string; isError?: boolean; content?: string }[] = [];
  for (const entry of store.path(sessionId)) {
    if (entry.kind !== "message") continue;
    const payload = entry.payload as { role?: string; content?: unknown[] };
    if (payload.role !== "user" || !Array.isArray(payload.content)) continue;
    for (const block of payload.content) {
      if (typeof block !== "object" || block === null) continue;
      const typed = block as { type?: string };
      if (typed.type === "tool_result") blocks.push(typed);
    }
  }
  return blocks;
}

function terminalHealth(store: MemoryLogStore, sessionId: string) {
  const terminal = store.path(sessionId).at(-1);
  const payload = terminal?.payload as {
    outcome?: string;
    health?: { toolErrors?: number };
  };
  return { kind: terminal?.kind, payload };
}

describe("issue 79 tool errors do not end the session", () => {
  it("turns an async tool throw into an isError tool_result and keeps going", async () => {
    const store = new MemoryLogStore();
    const sessionId = "async-throw";
    const { adapter, requests } = createFaux([
      toolScript("t1", "boom"),
      stopScript("done"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [throwingTool("boom", "async")],
    });

    const summary = await session.run("go");
    expect(summary.outcome).not.toBe("error");
    expect(toolResults(store, sessionId)).toEqual([
      {
        type: "tool_result",
        toolUseId: "t1",
        content: "boom: nope",
        isError: true,
      },
    ]);
    const view = requests[1]?.messages.flatMap((message) => message.content);
    expect(view).toContainEqual({
      type: "tool_result",
      toolUseId: "t1",
      content: "boom: nope",
      isError: true,
    });
    expect(terminalHealth(store, sessionId).payload?.outcome).not.toBe("error");
  });

  it("turns a sync tool throw into an isError tool_result and keeps going", async () => {
    const store = new MemoryLogStore();
    const sessionId = "sync-throw";
    const { adapter, requests } = createFaux([
      toolScript("t1", "boom"),
      stopScript("done"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [throwingTool("boom", "sync")],
    });

    const summary = await session.run("go");
    expect(summary.outcome).not.toBe("error");
    expect(toolResults(store, sessionId)[0]).toMatchObject({
      content: "boom: nope",
      isError: true,
    });
    const view = requests[1]?.messages.flatMap((message) => message.content);
    expect(view).toContainEqual({
      type: "tool_result",
      toolUseId: "t1",
      content: "boom: nope",
      isError: true,
    });
  });

  it("counts five tool errors and still finishes without a terminal error", async () => {
    const store = new MemoryLogStore();
    const sessionId = "five";
    const { adapter } = createFaux([
      toolScript("t1", "boom"),
      toolScript("t2", "boom"),
      toolScript("t3", "boom"),
      toolScript("t4", "boom"),
      toolScript("t5", "boom"),
      stopScript("done"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [throwingTool("boom", "async")],
    });

    const summary = await session.run("go");
    expect(summary.outcome).not.toBe("error");
    expect(toolResults(store, sessionId)).toHaveLength(5);
    expect(toolResults(store, sessionId).every((block) => block.isError)).toBe(
      true,
    );
    const logged = terminalHealth(store, sessionId);
    expect(logged.kind).toBe("terminal");
    expect(logged.payload?.outcome).toBe("done");
    expect(logged.payload?.health?.toolErrors).toBe(5);
  });
});
