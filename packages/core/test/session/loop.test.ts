import { describe, expect, it } from "vitest";
import { AgentSession, MemoryLogStore } from "../../src/index.ts";
import {
  createFaux,
  type FauxChunk,
  MODEL,
  SYSTEM,
  toolScript,
} from "../support/faux.ts";

function tool() {
  return {
    def: { name: "echo", description: "echo", inputSchema: { type: "object" } },
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

describe("scenario 8 abort", () => {
  it("records aborted with the partial assistant already in the log", async () => {
    const store = new MemoryLogStore();
    const sessionId = "s8";
    const controller = new AbortController();
    let markReady: (() => void) | undefined;
    const ready = new Promise<void>((resolve) => {
      markReady = resolve;
    });
    const { adapter } = createFaux([
      toolScript("c1", "echo"),
      ({ signal }) =>
        (async function* (): AsyncIterable<FauxChunk> {
          yield { type: "block_start", index: 0, blockType: "text" };
          yield { type: "text_delta", index: 0, text: "partial" };
          yield { type: "block_end", index: 0 };
          markReady?.();
          await new Promise<void>((resolve) => {
            if (signal?.aborted) {
              resolve();
              return;
            }
            signal?.addEventListener("abort", () => resolve(), { once: true });
          });
          yield {
            type: "usage",
            usage: {
              inputOther: 1,
              inputCacheRead: 0,
              inputCacheCreation: 0,
              output: 1,
            },
          };
          yield { type: "finish", kind: "aborted" };
        })(),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [tool()],
    });

    const pending = session.run("go", { signal: controller.signal });
    await ready;
    controller.abort();
    const summary = await pending;

    expect(summary.outcome).toBe("aborted");
    const path = store.path(sessionId);
    expect(path.at(-1)?.payload).toMatchObject({ outcome: "aborted" });
    const assistants = path.filter((entry) => {
      if (entry.kind !== "message") return false;
      return (entry.payload as { role?: string }).role === "assistant";
    });
    expect(JSON.stringify(assistants.at(-1)?.payload)).toContain("partial");
  });
});

describe("scenario 9 max turns", () => {
  it("stops with max_turns after exactly two request headers", async () => {
    const store = new MemoryLogStore();
    const sessionId = "s9";
    const { adapter, requests } = createFaux([
      toolScript("c1", "echo"),
      toolScript("c2", "echo"),
      toolScript("c3", "echo"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [tool()],
    });

    const summary = await session.run("go", { maxTurns: 2 });
    expect(summary.outcome).toBe("max_turns");
    expect(summary.turns).toBe(2);
    expect(requests).toHaveLength(2);
    const headers = store
      .path(sessionId)
      .filter((entry) => entry.kind === "request_header");
    expect(headers).toHaveLength(2);
    expect(store.path(sessionId).at(-1)?.payload).toMatchObject({
      outcome: "max_turns",
    });
  });
});
