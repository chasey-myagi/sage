import { expect, it } from "vitest";
import { runHealth } from "../src/health.ts";
import { AgentSession, MemoryLogStore } from "../src/index.ts";
import type { StoredEntry } from "../src/log/types.ts";
import {
  createFaux,
  MODEL,
  SYSTEM,
  stopScript,
  toolScript,
} from "./support/faux.ts";

const LEAK = "LEAK_SENTINEL_claim_/tmp/secret.txt";

function entry(
  seq: number,
  kind: string,
  payload: unknown,
  details?: Record<string, unknown>,
): StoredEntry {
  return {
    id: `00000000-0000-4000-8000-${String(seq).padStart(12, "0")}` as StoredEntry["id"],
    parentId: null,
    seq,
    at: "2026-09-22T00:00:00.000Z",
    kind,
    payload,
    ...(details ? { details } : {}),
  };
}

function assistant(
  seq: number,
  usage: Record<string, unknown> | undefined,
  text = "ok",
): StoredEntry {
  return entry(
    seq,
    "message",
    { role: "assistant", content: [{ type: "text", text }] },
    usage ? { usage, responseModel: LEAK } : undefined,
  );
}

function manifest(seq: number, payload: Record<string, unknown>): StoredEntry {
  return entry(seq, "manifest", {
    epoch: 0,
    turn: seq,
    claims: [{ id: "c", tier: "full", reason: LEAK, text: LEAK }],
    chunks: [{ id: "k", tier: "none", reason: LEAK, text: LEAK }],
    anomalies: [],
    ...payload,
  });
}

it("sums child usage from spawn tool results and leaves parent rates alone", async () => {
  const store = new MemoryLogStore();
  const { adapter } = createFaux([
    toolScript("s1", "spawn_subtask", "{}"),
    toolScript("s2", "spawn_subtasks", "{}"),
    stopScript("done"),
  ]);
  const session = new AgentSession({
    store,
    sessionId: "parent",
    adapter,
    model: MODEL,
    system: SYSTEM,
    tools: [
      spawnTool("spawn_subtask", {
        sessionId: "c1",
        outcome: "done",
        usage: {
          inputOther: 40,
          inputCacheRead: 50,
          inputCacheCreation: 6,
          output: 7,
        },
        chunks: [],
      }),
      spawnTool("spawn_subtasks", {
        results: [
          {
            sessionId: "c2",
            outcome: "done",
            usage: {
              inputOther: 1,
              inputCacheRead: 2,
              inputCacheCreation: 0,
              output: 3,
            },
            chunks: [],
          },
          {
            sessionId: "c3",
            outcome: "done",
            usage: {
              inputOther: 4,
              inputCacheRead: 0,
              inputCacheCreation: 1,
              output: 1,
            },
            chunks: [],
          },
        ],
      }),
    ],
  });
  const summary = await session.run("go");
  const health = summary.health;
  expect(health.subtasks).toEqual({
    count: 3,
    tokens: {
      input: 45,
      cacheRead: 52,
      cacheCreation: 7,
      output: 11,
      total: 115,
    },
  });
  expect(health.tokens).toEqual({
    input: 9,
    cacheRead: 3,
    cacheCreation: 0,
    output: 6,
    total: 18,
  });
  expect(health.cacheHitRatio).toBe(3 / 12);
  expect(health.expandRate).toBeNull();
  expect(JSON.stringify(health)).not.toContain("c1");
  for (const entry of store.path("parent")) {
    if (entry.kind !== "message") continue;
    const payload = entry.payload as { content?: { type?: string }[] };
    for (const block of payload.content ?? []) {
      expect(block).not.toHaveProperty("details");
    }
  }
});

it("returns zeros and null rates for an empty path", () => {
  expect(runHealth([])).toEqual({
    turns: 0,
    tokens: { input: 0, cacheRead: 0, cacheCreation: 0, output: 0, total: 0 },
    cacheHitRatio: null,
    expandRate: null,
    omitted: 0,
    expands: 0,
    epochs: 1,
    modelChanges: 0,
    scorerCalls: 0,
    scorerFallbacks: 0,
    shadow: {
      calls: 0,
      usage: {
        input: 0,
        cacheRead: 0,
        cacheCreation: 0,
        output: 0,
        total: 0,
      },
    },
    entryScored: {
      calls: 0,
      none: 0,
      summary: 0,
      full: 0,
      usage: {
        input: 0,
        cacheRead: 0,
        cacheCreation: 0,
        output: 0,
        total: 0,
      },
    },
    subtasks: {
      count: 0,
      tokens: {
        input: 0,
        cacheRead: 0,
        cacheCreation: 0,
        output: 0,
        total: 0,
      },
    },
    toolErrors: 0,
  });
});

it("sums assistant usage and weights the cache hit by tokens", () => {
  const path = [
    entry(0, "message", {
      role: "user",
      content: [{ type: "text", text: LEAK }],
    }),
    assistant(1, {
      inputOther: 10,
      inputCacheRead: 0,
      inputCacheCreation: 0,
      output: 1,
      raw: LEAK,
    }),
    assistant(2, {
      inputOther: 0,
      inputCacheRead: 90,
      inputCacheCreation: 10,
      output: 4,
    }),
  ];
  const health = runHealth(path);
  expect(health.turns).toBe(2);
  expect(health.tokens).toEqual({
    input: 10,
    cacheRead: 90,
    cacheCreation: 10,
    output: 5,
    total: 115,
  });
  // Turn ratios would be 0 and 0.9; a turn average is 0.45.
  // Token weight is 90 / (90 + 10 + 10).
  expect(health.cacheHitRatio).toBe(90 / 110);
  expect(JSON.stringify(health)).not.toContain(LEAK);
  expect(valuesAreCounts(health)).toBe(true);
});

it("returns null cacheHitRatio when no assistant usage was recorded", () => {
  const path = [
    entry(0, "request_header", { provider: "anthropic", model: "m" }),
    assistant(1, undefined),
  ];
  expect(runHealth(path).cacheHitRatio).toBeNull();
  expect(runHealth(path).turns).toBe(1);
});

it("returns null cacheHitRatio when input tokens are all zero", () => {
  expect(
    runHealth([
      assistant(0, {
        inputOther: 0,
        inputCacheRead: 0,
        inputCacheCreation: 0,
        output: 3,
      }),
    ]).cacheHitRatio,
  ).toBeNull();
});

it("reads omitted and expands from the last manifest only", () => {
  const health = runHealth([
    manifest(0, {
      budget: { total: 1, used: 1, overrun: 0, omitted: 10, expands: 9 },
      scorer: { calls: 1, fallback: false },
    }),
    manifest(1, {
      budget: { total: 1, used: 1, overrun: 0, omitted: 4, expands: 1 },
      scorer: { calls: 2, fallback: true },
      cache: { expectedReadTokens: 1000, measuredReadTokens: 1000 },
    }),
  ]);
  expect(health.omitted).toBe(4);
  expect(health.expands).toBe(1);
  expect(health.expandRate).toBe(0.25);
  expect(health.scorerCalls).toBe(3);
  expect(health.scorerFallbacks).toBe(1);
  // Manifest cache measuredReadTokens must not replace usage.
  expect(health.cacheHitRatio).toBeNull();
  expect(JSON.stringify(health)).not.toContain(LEAK);
});

it("returns null expandRate when the last manifest omitted nothing", () => {
  const health = runHealth([
    manifest(0, {
      budget: { total: 1, used: 1, overrun: 0, omitted: 3, expands: 1 },
    }),
    manifest(1, {
      budget: 40000,
    }),
  ]);
  expect(health.omitted).toBe(0);
  expect(health.expands).toBe(0);
  expect(health.expandRate).toBeNull();
});

it("counts epochs from the highest view_epoch and model_change entries", () => {
  const health = runHealth([
    entry(0, "view_epoch", { epoch: 0, reason: LEAK }),
    entry(1, "view_epoch", { epoch: 2, reason: "rebuild" }),
    entry(2, "model_change", {
      from: { provider: "a", model: "b" },
      to: { provider: "a", model: "c" },
      reason: LEAK,
    }),
    entry(3, "model_change", {
      from: { provider: "a", model: "c" },
      to: { provider: "a", model: "d" },
      reason: "route:",
    }),
  ]);
  expect(health.epochs).toBe(3);
  expect(health.modelChanges).toBe(2);
  expect(JSON.stringify(health)).not.toContain(LEAK);
});

it("sums numeric scorer fallbacks and ignores non-finite usage", () => {
  const health = runHealth([
    assistant(0, {
      inputOther: Number.NaN,
      inputCacheRead: 4,
      inputCacheCreation: Number.POSITIVE_INFINITY,
      output: 2,
    }),
    manifest(1, { scorer: { calls: 1, fallback: 2 } }),
    manifest(2, { scorer: { calls: "nope", fallback: false } }),
  ]);
  expect(health.tokens).toEqual({
    input: 0,
    cacheRead: 4,
    cacheCreation: 0,
    output: 2,
    total: 6,
  });
  expect(health.cacheHitRatio).toBe(1);
  expect(health.scorerCalls).toBe(1);
  expect(health.scorerFallbacks).toBe(2);
});

it("counts each shadow manifest once and sums its usage quadruple", () => {
  const health = runHealth([
    manifest(0, {
      shadow: {
        scorer: "model",
        decisions: [{ id: "a", tier: "none", reason: LEAK }],
        usage: {
          inputOther: 2,
          inputCacheRead: 4,
          inputCacheCreation: 1,
          output: 3,
          raw: LEAK,
        },
      },
    }),
    manifest(1, {
      shadow: { scorer: "model", decisions: [], fallback: "boom" },
    }),
    manifest(2, {}),
  ]);
  expect(health.shadow).toEqual({
    calls: 2,
    usage: {
      input: 2,
      cacheRead: 4,
      cacheCreation: 1,
      output: 3,
      total: 10,
    },
  });
  expect(JSON.stringify(health)).not.toContain(LEAK);
});

it("sums entryScored calls, tiers, and usage across manifests", () => {
  const health = runHealth([
    manifest(0, {
      entryScored: {
        calls: 1,
        none: 2,
        summary: 1,
        full: 0,
        usage: {
          inputOther: 3,
          inputCacheRead: 4,
          inputCacheCreation: 1,
          output: 2,
          raw: LEAK,
        },
      },
    }),
    manifest(1, {
      entryScored: { calls: 1, none: 0, summary: 0, full: 1 },
    }),
    manifest(2, {}),
  ]);
  expect(health.entryScored).toEqual({
    calls: 2,
    none: 2,
    summary: 1,
    full: 1,
    usage: {
      input: 3,
      cacheRead: 4,
      cacheCreation: 1,
      output: 2,
      total: 10,
    },
  });
  expect(JSON.stringify(health)).not.toContain(LEAK);
});

function spawnTool(name: string, details: Record<string, unknown>) {
  return {
    def: { name, description: name, inputSchema: { type: "object" } },
    isReadOnly: false,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    async execute() {
      return { content: name, isError: false, details };
    },
  };
}

function valuesAreCounts(value: unknown): boolean {
  if (value === null) return true;
  if (typeof value === "number") return Number.isFinite(value);
  if (typeof value === "object" && value !== null && !Array.isArray(value)) {
    return Object.values(value).every(valuesAreCounts);
  }
  return false;
}
