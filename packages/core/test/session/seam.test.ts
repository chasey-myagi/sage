import { describe, expect, it } from "vitest";
import {
  AgentSession,
  HeadChangedMidEpochError,
  MemoryLogStore,
  passthroughView,
  UnpairedToolCallError,
} from "../../src/index.ts";
import {
  createFaux,
  MODEL,
  SYSTEM,
  stopScript,
  toolScript,
} from "../support/faux.ts";

function tool(name = "echo") {
  return {
    def: { name, description: name, inputSchema: { type: "object" } },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    async execute() {
      return { content: "ok", isError: false };
    },
  };
}

describe("scenario 1 unpaired view", () => {
  it("throws before stream and appends a terminal error", async () => {
    const store = new MemoryLogStore();
    const { adapter, requests } = createFaux([stopScript("nope")]);
    const session = new AgentSession({
      store,
      sessionId: "s1",
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [],
      viewProvider: {
        view() {
          return {
            epoch: 0,
            head: { system: SYSTEM, tools: [] },
            messages: [
              {
                role: "assistant",
                content: [
                  {
                    type: "tool_use",
                    id: "u1",
                    name: "missing",
                    input: {},
                    rawInput: "{}",
                  },
                ],
              },
            ],
            cacheBreakpoints: ["head"],
            manifest: {
              epoch: 0,
              turn: 1,
              claims: [],
              chunks: [],
              anomalies: [],
            },
          };
        },
      },
    });

    await expect(session.run("hello")).rejects.toBeInstanceOf(
      UnpairedToolCallError,
    );
    expect(requests).toHaveLength(0);
    const path = store.path("s1");
    expect(path.at(-1)?.kind).toBe("terminal");
    expect(path.at(-1)?.payload).toMatchObject({ outcome: "error" });
  });
});

describe("scenario 2 head frozen inside an epoch", () => {
  it("throws on turn 3 when tools change without a new view_epoch and does not send", async () => {
    const store = new MemoryLogStore();
    const sessionId = "s2";
    const extra = {
      name: "extra",
      description: "extra",
      inputSchema: { type: "object" },
    };
    let turn = 0;
    const { adapter, requests } = createFaux([
      toolScript("c1", "echo"),
      toolScript("c2", "echo"),
      stopScript("nope"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [tool()],
      viewProvider: {
        async view(ctx) {
          turn += 1;
          const base = await passthroughView.view(ctx);
          if (turn < 3) return base;
          return {
            ...base,
            head: {
              system: ctx.head.system,
              tools: [...ctx.head.tools, extra],
            },
          };
        },
      },
    });

    let caught: unknown;
    try {
      await session.run("go", { maxTurns: 3 });
    } catch (error) {
      caught = error;
    }
    expect(caught).toBeInstanceOf(HeadChangedMidEpochError);
    expect((caught as Error).message).toMatch(/epoch/i);
    expect(requests).toHaveLength(2);
    const path = store.path(sessionId);
    expect(path.at(-1)?.payload).toMatchObject({ outcome: "error" });
    expect(path.some((entry) => entry.kind === "view_epoch")).toBe(false);
  });

  it("sends the changed head after a view_epoch is appended", async () => {
    const store = new MemoryLogStore();
    const sessionId = "s2b";
    const extra = {
      name: "extra",
      description: "extra",
      inputSchema: { type: "object" },
    };
    let turn = 0;
    const { adapter, requests } = createFaux([
      toolScript("c1", "echo"),
      toolScript("c2", "echo"),
      stopScript("ok"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [tool()],
      viewProvider: {
        async view(ctx) {
          turn += 1;
          const base = await passthroughView.view(ctx);
          if (turn < 3) return base;
          store.append(sessionId, {
            kind: "view_epoch",
            payload: { epoch: 2, reason: "add tool" },
          });
          return {
            ...base,
            epoch: 2,
            manifest: { ...base.manifest, epoch: 2 },
            head: {
              system: ctx.head.system,
              tools: [...ctx.head.tools, extra],
            },
          };
        },
      },
    });

    const summary = await session.run("go", { maxTurns: 3 });
    expect(summary.outcome).toBe("done");
    expect(requests).toHaveLength(3);
    expect(requests[2]?.head.tools.map((item) => item.name)).toEqual([
      "echo",
      "extra",
    ]);
  });
});

describe("scenario 7 view handed to hooks is a copy", () => {
  it("ignores mutations the hook makes to the view it received", async () => {
    const store = new MemoryLogStore();
    const { adapter, requests } = createFaux([stopScript("plain")]);
    const session = new AgentSession({
      store,
      sessionId: "s7",
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [tool()],
      viewProvider: {
        async view(ctx) {
          const base = await passthroughView.view(ctx);
          return { ...base, cacheBreakpoints: [{ afterMessage: 0 }] };
        },
      },
    });
    session.hooks.observe("view_compiled", ({ view }) => {
      view.head.system = "hacked";
      view.head.tools.push({
        name: "hack",
        description: "hack",
        inputSchema: {},
      });
      view.messages.push({
        role: "user",
        content: [{ type: "text", text: "hacked" }],
      });
      view.cacheBreakpoints.push("head");
    });

    await session.run("hello");
    expect(requests).toHaveLength(1);
    expect(requests[0]?.head.system).toBe(SYSTEM);
    expect(requests[0]?.head.tools.map((item) => item.name)).toEqual(["echo"]);
    expect(requests[0]?.messages).toEqual([
      { role: "user", content: [{ type: "text", text: "hello" }] },
    ]);
    expect(requests[0]?.cacheBreakpoints).toEqual([{ afterMessage: 0 }]);
  });
});
