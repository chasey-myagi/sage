import { describe, expect, it } from "vitest";
import { AgentSession, MemoryLogStore } from "../../src/index.ts";
import {
  createFaux,
  type FauxChunk,
  MODEL,
  SYSTEM,
  stopScript,
  USAGE,
} from "../support/faux.ts";

function cut(
  text: string,
  error: { message: string; status?: number; transport?: boolean },
): FauxChunk[] {
  return [
    { type: "block_start", index: 0, blockType: "text" },
    { type: "text_delta", index: 0, text },
    { type: "finish", kind: "error", error },
  ];
}

function recordingSleep() {
  const waits: number[] = [];
  return {
    waits,
    sleep: async (ms: number) => {
      waits.push(ms);
    },
  };
}

function assistants(store: MemoryLogStore, sessionId: string) {
  return store.path(sessionId).filter((entry) => {
    if (entry.kind !== "message") return false;
    return (entry.payload as { role?: string }).role === "assistant";
  });
}

function retries(store: MemoryLogStore, sessionId: string) {
  return store.path(sessionId).filter((entry) => entry.kind === "turn_retry");
}

describe("issue 85 turn retry", () => {
  it("retries one terminated stream and keeps a single assistant", async () => {
    const store = new MemoryLogStore();
    const sessionId = "retry-once";
    const { waits, sleep } = recordingSleep();
    const { adapter, requests } = createFaux([
      cut("half-terminated-text", { message: "terminated" }),
      stopScript("done"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [],
      sleep,
    });

    const summary = await session.run("go");
    const path = store.path(sessionId);
    const assistantAt = path.findIndex((entry) => {
      if (entry.kind !== "message") return false;
      return (entry.payload as { role?: string }).role === "assistant";
    });
    const retryAt = path.findIndex((entry) => entry.kind === "turn_retry");

    expect(summary.outcome).toBe("done");
    expect(summary.health.turnRetries).toBe(1);
    expect(summary.turns).toBe(1);
    expect(summary.usage).toEqual(USAGE);
    expect(waits).toEqual([2000]);
    expect(requests).toHaveLength(2);
    expect(requests[1]?.messages).toEqual(requests[0]?.messages);
    expect(JSON.stringify(requests)).not.toContain("half-terminated-text");
    expect(assistants(store, sessionId)).toHaveLength(1);
    expect(retries(store, sessionId)).toEqual([
      expect.objectContaining({
        payload: { turn: 1, attempt: 1, reason: "terminated" },
      }),
    ]);
    expect(retryAt).toBeGreaterThan(-1);
    expect(assistantAt).toBeGreaterThan(retryAt);
    expect(JSON.stringify(path)).not.toContain("half-terminated-text");
    expect(path.at(-1)?.payload).toMatchObject({
      outcome: "done",
      health: { turnRetries: 1 },
    });
    expect(
      path.filter((entry) => entry.kind === "request_header"),
    ).toHaveLength(1);
  });

  it("stops after two retries of terminated", async () => {
    const store = new MemoryLogStore();
    const sessionId = "retry-exhausted";
    const { waits, sleep } = recordingSleep();
    const { adapter, requests } = createFaux([
      cut("partial-1", { message: "terminated" }),
      cut("partial-2", { message: "terminated" }),
      cut("partial-3", { message: "terminated" }),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [],
      sleep,
    });

    const summary = await session.run("go");
    const path = store.path(sessionId);
    const logged = JSON.stringify(path);

    expect(summary.outcome).toBe("error");
    expect(summary.health.turnRetries).toBe(2);
    expect(waits).toEqual([2000, 8000]);
    expect(requests).toHaveLength(3);
    expect(retries(store, sessionId).map((entry) => entry.payload)).toEqual([
      { turn: 1, attempt: 1, reason: "terminated" },
      { turn: 1, attempt: 2, reason: "terminated" },
    ]);
    expect(logged).not.toContain("partial-1");
    expect(logged).not.toContain("partial-2");
    expect(assistants(store, sessionId)).toHaveLength(1);
    expect(logged).toContain("partial-3");
    expect(path.at(-1)?.payload).toMatchObject({
      outcome: "error",
      health: { turnRetries: 2 },
    });
  });

  it("does not retry a 400 finish error", async () => {
    const store = new MemoryLogStore();
    const sessionId = "no-retry-400";
    const { waits, sleep } = recordingSleep();
    const { adapter, requests } = createFaux([
      cut("bad-request-text", { message: "bad request", status: 400 }),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [],
      sleep,
    });

    const summary = await session.run("go");

    expect(summary.outcome).toBe("error");
    expect(summary.health.turnRetries).toBe(0);
    expect(waits).toEqual([]);
    expect(requests).toHaveLength(1);
    expect(retries(store, sessionId)).toHaveLength(0);
    expect(assistants(store, sessionId)).toHaveLength(1);
    expect(JSON.stringify(store.path(sessionId))).toContain("bad-request-text");
  });

  it("retries provider-marked transport and the other transport messages", async () => {
    const cases: {
      error: { message: string; transport?: boolean };
      reason: string;
    }[] = [
      {
        error: { message: "upstream reset", transport: true },
        reason: "upstream reset",
      },
      { error: { message: "fetch failed" }, reason: "fetch failed" },
      { error: { message: "read ECONNRESET" }, reason: "read ECONNRESET" },
      { error: { message: "socket hang up" }, reason: "socket hang up" },
      { error: { message: "aborted by peer" }, reason: "aborted by peer" },
    ];

    for (const [index, item] of cases.entries()) {
      const store = new MemoryLogStore();
      const sessionId = `transport-${index}`;
      const { waits, sleep } = recordingSleep();
      const { adapter, requests } = createFaux([
        cut("dropped", item.error),
        stopScript("done"),
      ]);
      const session = new AgentSession({
        store,
        sessionId,
        adapter,
        model: MODEL,
        system: SYSTEM,
        tools: [],
        sleep,
      });

      const summary = await session.run("go");
      expect(summary.outcome).toBe("done");
      expect(summary.health.turnRetries).toBe(1);
      expect(waits).toEqual([2000]);
      expect(requests).toHaveLength(2);
      expect(retries(store, sessionId).map((entry) => entry.payload)).toEqual([
        { turn: 1, attempt: 1, reason: item.reason },
      ]);
      expect(JSON.stringify(store.path(sessionId))).not.toContain("dropped");
    }
  });
});
