import type { HarnessTool } from "@sage/core";
import { MemoryLogStore } from "@sage/core";
import {
  FAUX_CONTEXT_WINDOW,
  FAUX_UNIT_USAGE,
  stopScript,
} from "@sage/core/testing";
import type { Request } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { subtaskTools } from "../../src/index.ts";

const SYSTEM = "You are sage.";

function user(text: string) {
  return {
    role: "user" as const,
    content: [{ type: "text" as const, text }],
  };
}

function seed(store: MemoryLogStore, sessionId: string) {
  store.append(sessionId, { kind: "message", payload: user("seed") });
  store.append(sessionId, {
    kind: "request_header",
    payload: {
      provider: "anthropic",
      model: "claude-test",
      systemPromptHash: "h",
      toolsHash: "t",
      sampling: {},
      adapterDefaults: {},
      contextWindow: FAUX_CONTEXT_WINDOW,
      pricing: null,
    },
  });
  store.append(sessionId, {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 1,
      claims: [],
      chunks: [{ id: "seq:0", tier: "full", reason: "kept", tokens: 1 }],
      anomalies: [],
    },
  });
}

function poolAdapter(delayMs: number) {
  let inflight = 0;
  let maxInflight = 0;
  const starts: number[] = [];
  const ends: number[] = [];
  let childCalls = 0;
  const adapter = {
    api: "anthropic-messages" as const,
    prepare(model: { provider: string; model: string }) {
      return {
        model,
        contextWindow: 8000,
        pricing: null,
        adapterDefaults: {},
      };
    },
    stream(request: Request) {
      const text = messageText(request);
      const child = text.includes("Begin the subtask");
      const query = child ? queryOf(text) : "parent";
      if (query === "boom") throw new Error("child blew up");
      if (child) {
        childCalls += 1;
        inflight += 1;
        maxInflight = Math.max(maxInflight, inflight);
        starts.push(performance.now());
      }
      return (async function* () {
        if (child && delayMs > 0) {
          await new Promise((resolve) => setTimeout(resolve, delayMs));
        }
        for (const chunk of stopScript(query, FAUX_UNIT_USAGE)) yield chunk;
        if (child) {
          inflight -= 1;
          ends.push(performance.now());
        }
      })();
    },
  };
  return {
    adapter,
    stats: () => ({
      maxInflight,
      starts: [...starts],
      ends: [...ends],
      childCalls,
    }),
  };
}

function messageText(request: Request): string {
  let text = "";
  for (const message of request.messages) {
    for (const block of message.content) {
      if (block.type === "text") text += block.text;
    }
  }
  return text;
}

function queryOf(text: string): string {
  const match =
    /<claim id="subtask-query" tier="full">\n([\s\S]*?)\n<\/claim>/.exec(text);
  return match?.[1] ?? "";
}

function toolsFor(
  store: MemoryLogStore,
  adapter: ReturnType<typeof poolAdapter>["adapter"],
  opts?: { maxConcurrent?: number },
) {
  const tools: HarnessTool[] = [];
  const created = subtaskTools({
    store,
    adapters: { anthropic: adapter },
    tools,
    system: SYSTEM,
    makeCompiler: ({ budget, query }) =>
      contextCompiler({
        budget,
        claims: [],
        subtask: { query },
        expectedTurns: 1,
      }),
    ...(opts?.maxConcurrent !== undefined
      ? { maxConcurrent: opts.maxConcurrent }
      : {}),
  });
  return created.spawnMany;
}

it("runs read-only subtasks concurrently up to the default cap", async () => {
  const store = new MemoryLogStore();
  seed(store, "parent");
  const pooled = poolAdapter(40);
  const spawnMany = toolsFor(store, pooled.adapter);
  const result = await spawnMany.execute(
    {
      tasks: ["alpha", "beta", "gamma"].map((query) => ({
        query,
        readOnly: true,
        budget: 200,
      })),
    },
    { sessionId: "parent", toolUseId: "pool" },
  );
  expect(result.isError).toBe(false);
  expect(result.content).toContain("alpha");
  expect(result.content).toContain("beta");
  expect(result.content).toContain("gamma");
  const stats = pooled.stats();
  expect(stats.starts).toHaveLength(3);
  expect(stats.maxInflight).toBe(3);
  expect(Math.max(...stats.starts)).toBeLessThan(Math.min(...stats.ends));
  const results = result.details?.childRuns;
  expect(Array.isArray(results)).toBe(true);
  if (!Array.isArray(results)) return;
  expect(results).toHaveLength(3);
  expect(results.every((item) => item.outcome === "done")).toBe(true);
});

it("keeps at most maxConcurrent children in flight", async () => {
  const store = new MemoryLogStore();
  seed(store, "parent");
  const pooled = poolAdapter(40);
  const spawnMany = toolsFor(store, pooled.adapter, { maxConcurrent: 2 });
  const result = await spawnMany.execute(
    {
      tasks: ["a", "b", "c"].map((query) => ({ query, readOnly: true })),
    },
    { sessionId: "parent", toolUseId: "pool" },
  );
  expect(result.isError).toBe(false);
  const stats = pooled.stats();
  expect(stats.maxInflight).toBe(2);
  expect(stats.starts).toHaveLength(3);
  expect(Math.max(...stats.starts)).toBeGreaterThanOrEqual(
    Math.min(...stats.ends),
  );
});

it("rejects a pair that writes the same key and names the conflict", async () => {
  const store = new MemoryLogStore();
  seed(store, "parent");
  const pooled = poolAdapter(0);
  const spawnMany = toolsFor(store, pooled.adapter);
  const result = await spawnMany.execute(
    {
      tasks: [
        {
          query: "taskA",
          accesses: [{ key: "src/a.ts", mode: "write" }],
        },
        {
          query: "taskB",
          accesses: [{ key: "src/a.ts", mode: "write" }],
        },
      ],
    },
    { sessionId: "parent", toolUseId: "pool" },
  );
  expect(result.isError).toBe(true);
  expect(result.content).toContain("(taskA, taskB, src/a.ts)");
  expect(pooled.stats().childCalls).toBe(0);

  const mixed = await spawnMany.execute(
    {
      tasks: [
        {
          query: "writer",
          accesses: [{ key: "src/a.ts", mode: "write" }],
        },
        {
          query: "reader",
          accesses: [{ key: "src/a.ts", mode: "read" }],
        },
      ],
    },
    { sessionId: "parent", toolUseId: "pool-mixed" },
  );
  expect(mixed.isError).toBe(true);
  expect(mixed.content).toContain("(writer, reader, src/a.ts)");
  expect(pooled.stats().childCalls).toBe(0);

  const disjoint = await spawnMany.execute(
    {
      tasks: [
        {
          query: "left",
          accesses: [{ key: "src/a.ts", mode: "write" }],
        },
        {
          query: "right",
          accesses: [{ key: "src/b.ts", mode: "write" }],
        },
      ],
    },
    { sessionId: "parent", toolUseId: "pool-ok" },
  );
  expect(disjoint.isError).toBe(false);
  expect(pooled.stats().childCalls).toBe(2);

  const reads = await spawnMany.execute(
    {
      tasks: [
        {
          query: "read-a",
          accesses: [{ key: "src/a.ts", mode: "read" }],
        },
        {
          query: "read-b",
          accesses: [{ key: "src/a.ts", mode: "search" }],
        },
      ],
    },
    { sessionId: "parent", toolUseId: "pool-read" },
  );
  expect(reads.isError).toBe(false);

  const unknown = await spawnMany.execute(
    {
      tasks: [{ query: "safe", readOnly: true }, { query: "writes" }],
    },
    { sessionId: "parent", toolUseId: "pool-unknown" },
  );
  expect(unknown.isError).toBe(true);
  expect(unknown.content).toContain("(safe, writes, unknown)");

  const ignored = await spawnMany.execute(
    {
      tasks: [
        {
          query: "ro-a",
          readOnly: true,
          accesses: [{ key: "src/a.ts", mode: "write" }],
        },
        {
          query: "ro-b",
          readOnly: true,
          accesses: [{ key: "src/a.ts", mode: "write" }],
        },
      ],
    },
    { sessionId: "parent", toolUseId: "pool-ro" },
  );
  expect(ignored.isError).toBe(false);
});

it("rejects an undeclared write beside any other task", async () => {
  const store = new MemoryLogStore();
  seed(store, "parent");
  const pooled = poolAdapter(0);
  const spawnMany = toolsFor(store, pooled.adapter);
  const result = await spawnMany.execute(
    {
      tasks: [
        { query: "safe", readOnly: true },
        { query: "writes", readOnly: false },
      ],
    },
    { sessionId: "parent", toolUseId: "pool" },
  );
  expect(result.isError).toBe(true);
  expect(result.content).toContain("(safe, writes, unknown)");
  expect(pooled.stats().childCalls).toBe(0);
});

it("keeps going when one child throws", async () => {
  const store = new MemoryLogStore();
  seed(store, "parent");
  const pooled = poolAdapter(20);
  const spawnMany = toolsFor(store, pooled.adapter);
  const result = await spawnMany.execute(
    {
      tasks: [
        { query: "alpha", readOnly: true },
        { query: "boom", readOnly: true },
        { query: "gamma", readOnly: true },
      ],
    },
    { sessionId: "parent", toolUseId: "pool" },
  );
  const results = result.details?.childRuns;
  if (!Array.isArray(results)) throw new Error("missing results");
  expect(results.map((item) => item.outcome)).toEqual([
    "done",
    "error",
    "done",
  ]);
  expect(result.content).toContain("alpha");
  expect(result.content).toContain("gamma");
  expect(result.content).toContain("child blew up");
});

it("caps the default pool at 4", async () => {
  const store = new MemoryLogStore();
  seed(store, "parent");
  const pooled = poolAdapter(30);
  const spawnMany = toolsFor(store, pooled.adapter);
  const result = await spawnMany.execute(
    {
      tasks: ["a", "b", "c", "d", "e"].map((query) => ({
        query,
        readOnly: true,
      })),
    },
    { sessionId: "parent", toolUseId: "pool" },
  );
  expect(result.isError).toBe(false);
  expect(pooled.stats().maxInflight).toBe(4);
  expect(pooled.stats().starts).toHaveLength(5);
});
