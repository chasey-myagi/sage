import { MemoryLogStore } from "@sage/core";
import { expect, it } from "vitest";
import { cacheHitRatio } from "../../src/compiler/cache.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";

it("reads per-turn cache hit ratios from manifest cache fields", async () => {
  const compiler = contextCompiler({
    budget: 8000,
    claims: [],
    expectedTurns: 1,
  });
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "message",
    payload: { role: "user", content: [{ type: "text", text: "hello" }] },
  });
  const first = await compiler.view({
    sessionId: "s",
    turn: 1,
    path: store.path("s"),
    model: { provider: "anthropic", model: "claude-sonnet-5" },
    prepared: {
      model: { provider: "anthropic", model: "claude-sonnet-5" },
      contextWindow: 1_000_000,
      pricing: null,
      adapterDefaults: {},
    },
    head: { system: "You are sage.", tools: [] },
    tools: [],
  });
  expect(first.manifest.cache?.measuredReadTokens).toBe(0);
  expect(first.manifest.cache?.expectedReadTokens).toBeGreaterThan(0);
  store.append("s", { kind: "manifest", payload: first.manifest });
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [{ type: "text", text: "ok" }],
    },
    details: {
      usage: {
        inputOther: 20,
        inputCacheRead: 80,
        inputCacheCreation: 0,
        output: 2,
      },
    },
  });
  const second = await compiler.view({
    sessionId: "s",
    turn: 2,
    path: store.path("s"),
    model: { provider: "anthropic", model: "claude-sonnet-5" },
    prepared: {
      model: { provider: "anthropic", model: "claude-sonnet-5" },
      contextWindow: 1_000_000,
      pricing: null,
      adapterDefaults: {},
    },
    head: { system: "You are sage.", tools: [] },
    tools: [],
  });
  store.append("s", { kind: "manifest", payload: second.manifest });
  expect(second.manifest.cache?.measuredReadTokens).toBe(80);

  const series = cacheHitRatio(store.path("s"));
  expect(series.map((row) => row.turn)).toEqual([1, 2]);
  expect(series[1]?.measured).toBe(80);
  expect(series[1]?.expected).toBe(second.manifest.cache?.expectedReadTokens);
  expect(series[1]?.ratio).toBe(
    (series[1]?.expected ?? 0) === 0
      ? 0
      : (series[1]?.measured ?? 0) / (series[1]?.expected ?? 1),
  );
});
