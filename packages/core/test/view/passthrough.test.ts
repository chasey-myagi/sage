import { describe, expect, it } from "vitest";
import {
  MemoryLogStore,
  passthroughView,
  synthesizeToolResult,
} from "../../src/index.ts";
import { MODEL } from "../support/faux.ts";

const head = {
  system: "sys",
  tools: [
    {
      name: "bash",
      description: "run",
      inputSchema: { type: "object" },
    },
  ],
};

const prepared = {
  model: MODEL,
  contextWindow: 100,
  pricing: null,
  adapterDefaults: {},
};

function ctx(store: MemoryLogStore, sessionId: string) {
  return {
    sessionId,
    turn: 1,
    path: store.path(sessionId),
    model: MODEL,
    prepared,
    head,
  };
}

describe("passthrough projection", () => {
  it("synthesizes TOOL_OUTCOME_UNKNOWN for a dangling tool_use and records the anomaly", async () => {
    const store = new MemoryLogStore();
    const sessionId = "crash";
    store.append(sessionId, {
      kind: "message",
      payload: {
        role: "user",
        content: [{ type: "text", text: "go" }],
      },
    });
    store.append(sessionId, {
      kind: "message",
      payload: {
        role: "assistant",
        content: [
          {
            type: "tool_use",
            id: "call_9",
            name: "bash",
            input: { cmd: "ls" },
            rawInput: '{"cmd":"ls"}',
          },
        ],
      },
    });

    const view = await passthroughView.view(ctx(store, sessionId));

    expect(view.messages).toEqual([
      {
        role: "user",
        content: [{ type: "text", text: "go" }],
      },
      {
        role: "assistant",
        content: [
          {
            type: "tool_use",
            id: "call_9",
            name: "bash",
            input: { cmd: "ls" },
            rawInput: '{"cmd":"ls"}',
          },
        ],
      },
      {
        role: "user",
        content: [synthesizeToolResult("call_9", "TOOL_OUTCOME_UNKNOWN")],
      },
    ]);
    expect(view.manifest.anomalies).toEqual([
      { kind: "dangling_tool_use", id: "call_9" },
    ]);
    expect(view.cacheBreakpoints).toEqual(["head"]);
    expect(view.epoch).toBe(0);
    expect(view.head).toEqual(head);
    expect(view.manifest.chunks).toHaveLength(view.messages.length);
    for (const chunk of view.manifest.chunks) {
      expect(chunk.reason).toBe("passthrough");
      expect(chunk.tier).toBe("full");
    }
  });

  it("drops an orphan tool_result, keeps sibling text, and records anomalies", async () => {
    const store = new MemoryLogStore();
    const sessionId = "orphan";
    store.append(sessionId, {
      kind: "message",
      payload: {
        role: "user",
        content: [
          { type: "text", text: "kept" },
          {
            type: "tool_result",
            toolUseId: "missing",
            content: "nope",
            isError: true,
          },
        ],
      },
    });
    store.append(sessionId, {
      kind: "message",
      payload: {
        role: "user",
        content: [
          {
            type: "tool_result",
            toolUseId: "also-missing",
            content: "gone",
            isError: false,
          },
        ],
      },
    });

    const view = await passthroughView.view(ctx(store, sessionId));

    expect(view.messages).toEqual([
      {
        role: "user",
        content: [{ type: "text", text: "kept" }],
      },
    ]);
    expect(view.manifest.anomalies).toEqual([
      { kind: "orphan_tool_result", id: "missing" },
      { kind: "orphan_tool_result", id: "also-missing" },
      { kind: "empty_message" },
    ]);
  });

  it("leaves a closed tool exchange untouched and reads epoch from the path", async () => {
    const store = new MemoryLogStore();
    const sessionId = "closed";
    store.append(sessionId, {
      kind: "view_epoch",
      payload: { epoch: 4, reason: "phase" },
    });
    store.append(sessionId, {
      kind: "message",
      payload: {
        role: "assistant",
        content: [
          {
            type: "tool_use",
            id: "ok",
            name: "bash",
            input: {},
            rawInput: "{}",
          },
        ],
      },
    });
    store.append(sessionId, {
      kind: "message",
      payload: {
        role: "user",
        content: [
          {
            type: "tool_result",
            toolUseId: "ok",
            content: "done",
            isError: false,
          },
        ],
      },
    });

    const view = await passthroughView.view(ctx(store, sessionId));

    expect(view.epoch).toBe(4);
    expect(view.manifest.epoch).toBe(4);
    expect(view.manifest.anomalies).toEqual([]);
    expect(view.messages).toHaveLength(2);
    expect(view.manifest.claims).toEqual([]);
  });
});
