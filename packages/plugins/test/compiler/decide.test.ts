import {
  AgentSession,
  MemoryLogStore,
  passthroughView,
  type ViewProvider,
} from "@sage/core";
import { fauxAdapter, stopScript, toolScript } from "@sage/core/testing";
import type { Message, Pricing } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { indexChunks } from "../../src/compiler/chunks.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { decideEpoch, ratesFor } from "../../src/compiler/decide.ts";

const SONNET: Pricing = {
  inputPerMTok: 2,
  outputPerMTok: 10,
  cacheWritePerMTok: 2.5,
  cacheReadPerMTok: 0.2,
  source: "https://platform.claude.com/docs/en/about-claude/pricing",
};

const DEEPSEEK: Pricing = {
  inputPerMTok: 0.3,
  outputPerMTok: 1.2,
  cacheWritePerMTok: 0,
  cacheReadPerMTok: 0.006,
  source: "https://api-docs.deepseek.com/quick_start/pricing",
};

const RATES = { read: 0.1, write: 1.25, source: "catalog" as const };

function baseDecision(over: Partial<Parameters<typeof decideEpoch>[0]> = {}) {
  return decideEpoch({
    oldTokens: 150000,
    newTokens: 40000,
    remainingTurns: 20,
    rates: RATES,
    minSavingsRatio: 0.2,
    contextWindow: 1_000_000,
    overContext: false,
    ...over,
  });
}

it("derives read and write rates from catalog pricing", () => {
  expect(ratesFor(SONNET)).toEqual({
    read: 0.1,
    write: 1.25,
    source: "catalog",
  });
  expect(ratesFor(null)).toEqual({
    read: 0.1,
    write: 1.25,
    source: "fallback",
  });
  expect(
    ratesFor({
      inputPerMTok: 2.5,
      outputPerMTok: 15,
      cacheReadPerMTok: 0.25,
      cacheWritePerMTok: 0,
      source: "https://developers.openai.com/api/docs/pricing",
    }),
  ).toEqual({ read: 0.1, write: 1, source: "catalog" });
});

it("uses max(cacheWrite, input) so a zero write surcharge still pays the miss price", () => {
  expect(ratesFor(DEEPSEEK)).toEqual({
    read: 0.02,
    write: 1,
    source: "catalog",
  });
  expect(ratesFor(SONNET)).toEqual({
    read: 0.1,
    write: 1.25,
    source: "catalog",
  });
});

it("rebuilds a DeepSeek epoch when 20 turns remain and refuses when 10 remain", () => {
  const rates = ratesFor(DEEPSEEK);
  const open = decideEpoch({
    oldTokens: 150000,
    newTokens: 40000,
    remainingTurns: 20,
    rates,
    minSavingsRatio: 0.2,
    contextWindow: 1_000_000,
    overContext: false,
  });
  expect(open.savingsPerTurn).toBe(2200);
  expect(open.extraCost).toBe(37000);
  expect(open.paybackTurns).toBe(17);
  expect(open.rebuild).toBe(true);
  expect(open.reason).toBe("payback:17<=20");

  const hold = decideEpoch({
    oldTokens: 150000,
    newTokens: 40000,
    remainingTurns: 10,
    rates,
    minSavingsRatio: 0.2,
    contextWindow: 1_000_000,
    overContext: false,
  });
  expect(hold.extraCost).toBe(37000);
  expect(hold.paybackTurns).toBe(17);
  expect(hold.rebuild).toBe(false);
  expect(hold.reason).toBe("payback:17>10");
});

it("pays back in 4 turns when 20 remain and refuses when 3 remain", () => {
  const worth = baseDecision();
  expect(worth.savingsPerTurn).toBe(11000);
  expect(worth.extraCost).toBe(35000);
  expect(worth.paybackTurns).toBe(4);
  expect(worth.rebuild).toBe(true);
  expect(worth.reason).toBe("payback:4<=20");

  const late = baseDecision({ remainingTurns: 3 });
  expect(late.rebuild).toBe(false);
  expect(late.reason).toBe("payback:4>3");
  expect(late.paybackTurns).toBe(4);
});

it("does not rebuild when the token drop is below the savings ratio", () => {
  const decision = baseDecision({ newTokens: 130000 });
  expect(decision.rebuild).toBe(false);
  expect(decision.reason).toBe("savings-too-small");
});

it("rebuilds immediately when the extra cost is not positive", () => {
  const decision = baseDecision({
    oldTokens: 400000,
    newTokens: 30000,
    remainingTurns: 10,
  });
  expect(decision.extraCost).toBeLessThanOrEqual(0);
  expect(decision.rebuild).toBe(true);
  expect(decision.reason).toBe("immediate");
});

it("forces a rebuild on the context-window guard", () => {
  const decision = baseDecision({
    newTokens: 140000,
    overContext: true,
  });
  expect(decision.rebuild).toBe(true);
  expect(decision.reason).toBe("context-window-guard");
});

it("marks pricing fallback on the manifest when pricing is null", async () => {
  const compiler = contextCompiler({
    budget: 8000,
    claims: [],
    expectedTurns: 1,
  });
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "message",
    payload: { role: "user", content: [{ type: "text", text: "hi" }] },
  });
  const view = await compiler.view({
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
  expect(view.manifest.epochDecision?.readRate).toBe(0.1);
  expect(view.manifest.epochDecision?.writeRate).toBe(1.25);
  expect(view.manifest.epochDecision?.reason).toContain("pricing:fallback");
});

it("opens the next epoch when the inequality pays back", async () => {
  const count = (text: string) => (text.includes("HUGE") ? 10_000 : 0);
  const store = new MemoryLogStore();
  for (let index = 0; index < 20; index += 1) {
    const id = `t${index}`;
    store.append("seed", {
      kind: "message",
      payload: {
        role: "assistant",
        content: [
          {
            type: "tool_use",
            id,
            name: "bash",
            input: {},
            rawInput: "{}",
          },
        ],
      },
    });
    store.append("seed", {
      kind: "message",
      payload: {
        role: "user",
        content: [
          {
            type: "tool_result",
            toolUseId: id,
            content: "HUGE",
            isError: false,
          },
        ],
      },
    });
  }
  const seeded = passthroughView.view({
    sessionId: "seed",
    turn: 1,
    path: store.path("seed"),
    model: { provider: "anthropic", model: "claude-sonnet-5" },
    prepared: {
      model: { provider: "anthropic", model: "claude-sonnet-5" },
      contextWindow: 1_000_000,
      pricing: SONNET,
      adapterDefaults: {},
    },
    head: { system: "You are sage.", tools: [] },
    tools: [],
  });
  const seqs = store
    .path("seed")
    .filter((entry) => entry.kind === "message")
    .map((entry) => entry.seq);
  const chunks = indexChunks(seeded.messages, seqs, [], count);
  store.append("seed", {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 0,
      budget: { total: 1, used: 200_000, overrun: 0 },
      claims: [],
      chunks: chunks.map((chunk) => ({
        id: chunk.id,
        tier: "full",
        reason: "kept",
        tokens: chunk.tokens,
      })),
      anomalies: [],
    },
  });

  const seen: { epoch: number; reason?: string; payback: number | null }[] = [];
  const compiler = contextCompiler({
    budget: 1,
    claims: [],
    count,
    expectedTurns: 20,
    minSavingsRatio: 0.2,
  });
  const recording: ViewProvider = {
    async view(ctx) {
      const compiled = await compiler.view(ctx);
      seen.push({
        epoch: compiled.epoch,
        reason: compiled.epochStart?.reason,
        payback: compiled.manifest.epochDecision?.paybackTurns ?? null,
      });
      return compiled;
    },
  };
  const usage = {
    inputOther: 10,
    inputCacheRead: 10,
    inputCacheCreation: 0,
    output: 1,
  };
  const adapter = fauxAdapter(
    [toolScript("next", "bash", "{}", usage), stopScript("done", usage)],
    {
      prepared: { contextWindow: 1_000_000, pricing: SONNET },
    },
  );
  const session = new AgentSession({
    store,
    sessionId: "seed",
    adapter,
    model: { provider: "anthropic", model: "claude-sonnet-5" },
    system: "You are sage.",
    tools: [],
    viewProvider: recording,
  });
  await session.run("task");

  expect(seen[0]?.epoch).toBe(1);
  expect(seen[0]?.payback).toBe(4);
  expect(seen[0]?.reason).toBe("cost:4");
  expect(seen[1]?.epoch).toBe(1);
  expect(seen[1]?.reason).toBeUndefined();
  const epochs = store
    .path("seed")
    .filter((entry) => entry.kind === "view_epoch");
  expect(epochs.at(-1)?.payload).toEqual({ epoch: 1, reason: "cost:4" });
  const manifests = store
    .path("seed")
    .filter((entry) => entry.kind === "manifest");
  const opened = manifests.at(-2)?.payload as {
    chunks?: { id: string; tier: string }[];
  };
  const followed = manifests.at(-1)?.payload as {
    chunks?: { id: string; tier: string }[];
  };
  const frozenFull = (opened.chunks ?? []).filter(
    (chunk) => chunk.tier === "full",
  );
  for (const chunk of frozenFull) {
    expect(followed.chunks?.find((item) => item.id === chunk.id)?.tier).toBe(
      "full",
    );
  }
});

it("records overrun and does not reopen after a guard that is still over", async () => {
  const compiler = contextCompiler({
    budget: 1_000_000,
    claims: [],
    count: (text) => (text.includes("WIDE") ? 1000 : 1),
  });
  const store = new MemoryLogStore();
  const wide: Message = {
    role: "user",
    content: [{ type: "text", text: "WIDE" }],
  };
  store.append("s", { kind: "message", payload: wide });
  const prepared = {
    model: { provider: "anthropic", model: "claude-sonnet-5" },
    contextWindow: 100,
    pricing: SONNET,
    adapterDefaults: {},
  };
  const head = { system: "You are sage.", tools: [] as const };
  const first = await compiler.view({
    sessionId: "s",
    turn: 1,
    path: store.path("s"),
    model: prepared.model,
    prepared,
    head: { system: head.system, tools: [...head.tools] },
    tools: [],
  });
  expect(first.epoch).toBe(1);
  expect(first.epochStart).toEqual({ reason: "context-window-guard" });
  expect(first.manifest.epochDecision?.reason).toContain(
    "context-window-guard",
  );
  const budget = first.manifest.budget;
  if (typeof budget !== "object" || budget === undefined) {
    throw new Error("expected a budget report");
  }
  expect(budget.overrun).toBeGreaterThan(0);

  store.append("s", {
    kind: "view_epoch",
    payload: { epoch: 1, reason: "context-window-guard" },
    details: { prefixMessage: structuredClone(first.messages[0]) },
  });
  store.append("s", { kind: "manifest", payload: first.manifest });
  const second = await compiler.view({
    sessionId: "s",
    turn: 2,
    path: store.path("s"),
    model: prepared.model,
    prepared,
    head: { system: head.system, tools: [...head.tools] },
    tools: [],
  });
  expect(second.epoch).toBe(1);
  expect(second.epochStart).toBeUndefined();
  expect(second.manifest.epochDecision?.rebuild).toBe(false);
  const again = second.manifest.budget;
  if (typeof again !== "object" || again === undefined) {
    throw new Error("expected a budget report");
  }
  expect(again.overrun).toBeGreaterThan(0);
});
