import { describe, expect, it } from "vitest";
import { AgentSession, MemoryLogStore } from "../../src/index.ts";
import {
  createFaux,
  FAUX_MODEL as MODEL,
  FAUX_SYSTEM as SYSTEM,
  stopScript,
  toolScript,
} from "../../src/testing/index.ts";

function harness(
  name: string,
  onExecute?: (
    input: unknown,
    ctx: { sessionId: string; toolUseId: string; signal?: AbortSignal },
  ) => void,
) {
  return {
    def: { name, description: name, inputSchema: { type: "object" } },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    async execute(
      input: unknown,
      ctx: { sessionId: string; toolUseId: string; signal?: AbortSignal },
    ) {
      onExecute?.(input, ctx);
      return { content: `ran:${name}`, isError: false };
    },
  };
}

function toolResultContent(
  store: MemoryLogStore,
  sessionId: string,
): unknown[] {
  const blocks: unknown[] = [];
  for (const entry of store.path(sessionId)) {
    if (entry.kind !== "message") continue;
    const payload = entry.payload as { content?: unknown[] };
    for (const block of payload.content ?? []) {
      if (
        typeof block === "object" &&
        block !== null &&
        (block as { type?: string }).type === "tool_result"
      ) {
        blocks.push(block);
      }
    }
  }
  return blocks;
}

describe("the tool pipeline applies hooks around execute", () => {
  it("turns a tool_pre deny into an error result and does not execute", async () => {
    const store = new MemoryLogStore();
    const sessionId = "deny";
    let ran = false;
    const { adapter } = createFaux([
      toolScript("c1", "echo", '{"n":1}'),
      stopScript("stop"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [
        harness("echo", () => {
          ran = true;
        }),
      ],
    });
    session.hooks.on("tool_pre", () => ({ deny: "x" }));
    session.hooks.guard(() => undefined);

    const summary = await session.run("go");
    expect(summary.outcome).toBe("done");
    expect(ran).toBe(false);
    expect(toolResultContent(store, sessionId)).toEqual([
      {
        type: "tool_result",
        toolUseId: "c1",
        content: "Denied: x",
        isError: true,
      },
    ]);
  });

  it("rejects when a guard returns a reason and still cannot override a pre deny", async () => {
    const store = new MemoryLogStore();
    const sessionId = "guard";
    let ran = false;
    const { adapter } = createFaux([
      toolScript("g1", "echo"),
      stopScript("stop"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [
        harness("echo", () => {
          ran = true;
        }),
      ],
    });
    session.hooks.guard(() => "nope");

    await session.run("go");
    expect(ran).toBe(false);
    expect(toolResultContent(store, sessionId)[0]).toMatchObject({
      content: "Denied: nope",
      isError: true,
    });

    const store2 = new MemoryLogStore();
    const sessionId2 = "guard-pre";
    let ran2 = false;
    const faux2 = createFaux([toolScript("g2", "echo"), stopScript("stop")]);
    const session2 = new AgentSession({
      store: store2,
      sessionId: sessionId2,
      adapter: faux2.adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [
        harness("echo", () => {
          ran2 = true;
        }),
      ],
    });
    session2.hooks.on("tool_pre", () => ({ deny: "x" }));
    session2.hooks.guard(() => undefined);
    await session2.run("go");
    expect(ran2).toBe(false);
    expect(toolResultContent(store2, sessionId2)[0]).toMatchObject({
      content: "Denied: x",
      isError: true,
    });
  });

  it("lets tool_post replace the result and keeps going when tool_result observe throws", async () => {
    const store = new MemoryLogStore();
    const sessionId = "post";
    const { adapter } = createFaux([
      toolScript("p1", "echo"),
      stopScript("stop"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [harness("echo")],
    });
    session.hooks.on("tool_post", ({ result }) => ({
      ...result,
      content: "replaced",
      isError: false,
    }));
    session.hooks.observe("tool_result", () => {
      throw new Error("watch failed");
    });

    const summary = await session.run("go");
    expect(summary.outcome).toBe("done");
    expect(summary.observerErrors).toEqual([
      { event: "tool_result", error: "watch failed" },
    ]);
    expect(toolResultContent(store, sessionId)[0]).toMatchObject({
      content: "replaced",
      isError: false,
    });
  });

  it("applies updatedInput, wraps execute with around, and reports an unknown tool", async () => {
    const store = new MemoryLogStore();
    const sessionId = "around";
    let seen: unknown;
    const { adapter } = createFaux([
      toolScript("a1", "echo", '{"n":1}'),
      toolScript("a2", "missing"),
      stopScript("stop"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [
        harness("echo", (input) => {
          seen = input;
        }),
      ],
    });
    session.hooks.on("tool_pre", () => ({ updatedInput: { n: 2 } }));
    session.hooks.around(async (_call, next) => {
      const result = await next();
      return { ...result, content: `wrapped:${result.content}` };
    });

    await session.run("go");
    expect(seen).toEqual({ n: 2 });
    const blocks = toolResultContent(store, sessionId);
    expect(blocks[0]).toMatchObject({
      toolUseId: "a1",
      content: "wrapped:ran:echo",
    });
    expect(blocks[1]).toMatchObject({
      toolUseId: "a2",
      content: "Unknown tool: missing",
      isError: true,
    });
  });

  it("packs every tool result from one assistant turn into a single user message", async () => {
    const store = new MemoryLogStore();
    const sessionId = "batch";
    const { adapter } = createFaux([
      [
        {
          type: "block_start",
          index: 0,
          blockType: "tool_use",
          id: "a",
          name: "echo",
        },
        { type: "tool_input_delta", index: 0, partialJson: "{}" },
        { type: "block_end", index: 0 },
        {
          type: "block_start",
          index: 1,
          blockType: "tool_use",
          id: "b",
          name: "echo",
        },
        { type: "tool_input_delta", index: 1, partialJson: "{}" },
        { type: "block_end", index: 1 },
        {
          type: "usage",
          usage: {
            inputOther: 1,
            inputCacheRead: 0,
            inputCacheCreation: 0,
            output: 1,
          },
        },
        { type: "finish", kind: "tool_use" },
      ],
      stopScript("stop"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [harness("echo")],
    });

    await session.run("go");
    const userMessages = store.path(sessionId).filter((entry) => {
      if (entry.kind !== "message") return false;
      const payload = entry.payload as { role?: string; content?: unknown[] };
      return (
        payload.role === "user" &&
        (payload.content ?? []).some(
          (block) =>
            typeof block === "object" &&
            block !== null &&
            (block as { type?: string }).type === "tool_result",
        )
      );
    });
    expect(userMessages).toHaveLength(1);
    const packed = userMessages[0];
    if (!packed) throw new Error("missing tool result message");
    const packedPayload = packed.payload as {
      content: { toolUseId: string }[];
    };
    expect(packedPayload.content.map((block) => block.toolUseId)).toEqual([
      "a",
      "b",
    ]);
  });

  it("passes the tool use id on execute context", async () => {
    const store = new MemoryLogStore();
    const sessionId = "tool-id";
    const seen: string[] = [];
    const { adapter } = createFaux([
      toolScript("c1", "echo"),
      stopScript("stop"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [
        harness("echo", (_input, ctx) => {
          seen.push(ctx.toolUseId);
        }),
      ],
    });

    const summary = await session.run("go");
    expect(summary.outcome).toBe("done");
    expect(seen).toEqual(["c1"]);
  });
});

describe("a throwing observer does not stop the others", () => {
  it("runs later observers after one throws and records the error on the summary", async () => {
    const store = new MemoryLogStore();
    const { adapter } = createFaux([stopScript("ok")]);
    const session = new AgentSession({
      store,
      sessionId: "s6",
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [],
    });
    const order: string[] = [];
    session.hooks.observe("turn_start", () => {
      order.push("a");
    });
    session.hooks.observe("turn_start", () => {
      order.push("b");
      throw new Error("boom");
    });
    session.hooks.observe("turn_start", () => {
      order.push("c");
    });

    const summary = await session.run("hi");
    expect(summary.outcome).toBe("done");
    expect(order).toEqual(["a", "b", "c"]);
    expect(summary.observerErrors).toEqual([
      { event: "turn_start", error: "boom" },
    ]);
  });
});
