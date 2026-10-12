import { MemoryLogStore } from "@sage/core";
import type { Message, Pricing } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import {
  classifyEpoch,
  DEFAULT_READ_RATE,
  DEFAULT_WRITE_RATE,
  type ExpandEstimate,
  expandEstimateFor,
  PRIOR_EXPAND_SHARE,
  type Rates,
  ratesFor,
} from "../../src/compiler/decide.ts";
import { chooseEpoch } from "../../src/compiler/epoch.ts";
import { EXPAND_TOOL_NAME } from "../../src/compiler/expand.ts";
import { epochCommitment } from "../../src/compiler/freeze.ts";
import { expandResultTokens } from "../../src/compiler/log-read.ts";

// Observed zero expands: the cost rule without any expand term.
const NO_EXPANDS: ExpandEstimate = {
  source: "observed",
  expandsPerEpoch: 0,
  tokensPerExpand: 0,
};

const RATES: Rates = { read: 0.1, write: 1.25, source: "pricing" };

function classify(over: Partial<Parameters<typeof classifyEpoch>[0]> = {}) {
  return classifyEpoch({
    oldTokens: 150_000,
    newTokens: 40_000,
    remainingTurns: 20,
    rates: RATES,
    minSavingsRatio: 0.2,
    contextWindow: 1_000_000,
    overContext: false,
    expands: NO_EXPANDS,
    turnsSinceEpochOpen: 100,
    committedPaybackTurns: 0,
    ...over,
  });
}

function pricing(over: Partial<Pricing>): Pricing {
  return {
    inputPerMTok: 2,
    outputPerMTok: 10,
    cacheWritePerMTok: 2.5,
    cacheReadPerMTok: 0.2,
    source: "https://example.test/pricing",
    ...over,
  };
}

it("takes both cache rates from the pricing", () => {
  expect(ratesFor(pricing({}))).toEqual({
    read: 0.1,
    write: 1.25,
    source: "pricing",
  });
});

it("falls back to the default rates only when pricing is missing", () => {
  expect(ratesFor(null)).toEqual({
    read: DEFAULT_READ_RATE,
    write: DEFAULT_WRITE_RATE,
    source: "default",
  });
  expect(ratesFor(pricing({ inputPerMTok: 0 })).source).toBe("default");
});

it("keeps a user's zero write price as the input price, sourced from the pricing", () => {
  expect(
    ratesFor(
      pricing({
        inputPerMTok: 6.5,
        cacheReadPerMTok: 1.3,
        cacheWritePerMTok: 0,
      }),
    ),
  ).toEqual({ read: 0.2, write: 1, source: "pricing" });
});

it("never prices a cache write below the input price", () => {
  const rates = ratesFor(pricing({ cacheWritePerMTok: 1 }));
  expect(rates.write).toBe(1);
  expect(rates.source).toBe("pricing");
});

it("a write rate above the read rate raises the rebuild cost and can turn a rebuild down", () => {
  const cheap = classify({ rates: { ...RATES, write: 1 }, remainingTurns: 3 });
  const dear = classify({ rates: { ...RATES, write: 2 }, remainingTurns: 3 });
  expect(dear.decision.extraCost).toBeGreaterThan(cheap.decision.extraCost);
  expect(cheap.decision.extraCost).toBe(40_000 * 1 - 150_000 * 0.1);
  expect(dear.decision.extraCost).toBe(40_000 * 2 - 150_000 * 0.1);
  expect(cheap.decision.savingsPerTurn).toBe(dear.decision.savingsPerTurn);
  expect(cheap.decision.paybackTurns).toBe(3);
  expect(cheap.decision.rebuild).toBe(true);
  expect(dear.decision.paybackTurns).toBe(6);
  expect(dear.decision.rebuild).toBe(false);
});

it("records the rate source on the decision", () => {
  expect(classify({ rates: ratesFor(null) }).decision.rateSource).toBe(
    "default",
  );
  expect(classify().decision.rateSource).toBe("pricing");
});

it("holds a payback rebuild until the epoch has lasted the payback it was opened on", () => {
  const held = classify({ turnsSinceEpochOpen: 5, committedPaybackTurns: 9 });
  expect(held.decision.rebuild).toBe(false);
  expect(held.trigger).toBe("no-rebuild");
  expect(held.decision.reason).toBe("payback-pending:5<9");
  expect(held.decision.paybackTurns).toBe(4);
  expect(held.decision.turnsSinceEpochOpen).toBe(5);
  expect(held.decision.committedPaybackTurns).toBe(9);

  const due = classify({ turnsSinceEpochOpen: 9, committedPaybackTurns: 9 });
  expect(due.decision.rebuild).toBe(true);
  expect(due.decision.reason).toBe("payback:4<=20");
});

it("an epoch with no committed payback does not hold a payback rebuild", () => {
  const free = classify({ turnsSinceEpochOpen: 1, committedPaybackTurns: 0 });
  expect(free.decision.rebuild).toBe(true);
  expect(free.trigger).toBe("payback");
});

it("does not hold an immediate rebuild: it is cheaper on its own turn", () => {
  const immediate = classify({
    oldTokens: 400_000,
    newTokens: 30_000,
    turnsSinceEpochOpen: 1,
    committedPaybackTurns: 9,
  });
  expect(immediate.decision.extraCost).toBeLessThanOrEqual(0);
  expect(immediate.decision.rebuild).toBe(true);
  expect(immediate.trigger).toBe("immediate");
});

it("the context-window guard still fires before the committed payback", () => {
  const guarded = classify({
    turnsSinceEpochOpen: 0,
    committedPaybackTurns: 9,
    overContext: true,
  });
  expect(guarded.decision.rebuild).toBe(true);
  expect(guarded.trigger).toBe("context-window-guard");
  expect(guarded.decision.reason).toBe("context-window-guard");
});

it("a guard-opened epoch commits to no payback", () => {
  const project = (used: number) => used;
  const input = {
    frozenUsed: 150_000,
    recompiledUsed: 40_000,
    hasFrozen: true,
    project,
    rates: RATES,
    remainingTurns: 20,
    minSavingsRatio: 0.2,
    contextWindow: 1_000_000,
    expands: NO_EXPANDS,
    turnsSinceEpochOpen: 2,
    committedPaybackTurns: 9,
  };
  const cost = chooseEpoch({ ...input, epochOpenReason: "cost:9" });
  expect(cost.decision.rebuild).toBe(false);
  expect(cost.decision.committedPaybackTurns).toBe(9);
  const guard = chooseEpoch({
    ...input,
    epochOpenReason: "context-window-guard",
  });
  expect(guard.decision.rebuild).toBe(true);
  expect(guard.decision.committedPaybackTurns).toBe(0);
});

it("uses the named prior share when the session has no expand observations", () => {
  expect(expandEstimateFor([], 3)).toEqual({
    source: "prior",
    share: PRIOR_EXPAND_SHARE,
  });
});

it("estimates expands per epoch and tokens per expand from observations", () => {
  expect(expandEstimateFor([300, 500, 1000], 2)).toEqual({
    source: "observed",
    expandsPerEpoch: 1.5,
    tokensPerExpand: 600,
  });
});

it("adds the expected expand cost to the rebuild cost, at the write rate", () => {
  const without = classify();
  const prior = classify({ expands: expandEstimateFor([], 1) });
  const observed = classify({ expands: expandEstimateFor([4000, 4000], 1) });
  const priorCost = PRIOR_EXPAND_SHARE * (150_000 - 40_000) * RATES.write;
  expect(without.decision.postRebuildCost).toBe(0);
  expect(prior.decision.postRebuildCost).toBe(priorCost);
  expect(prior.decision.extraCost).toBe(without.decision.extraCost + priorCost);
  expect(observed.decision.postRebuildCost).toBe(2 * 4000 * RATES.write);
  expect(observed.decision.expandEstimate.source).toBe("observed");
  expect(prior.decision.expandEstimate.source).toBe("prior");
  expect(observed.decision.paybackTurns).toBeGreaterThan(
    without.decision.paybackTurns ?? 0,
  );
});

it("never expects expands to bring back more than the rebuild drops", () => {
  const capped = classify({
    oldTokens: 10_000,
    newTokens: 7_000,
    expands: expandEstimateFor([50_000], 1),
  });
  expect(capped.decision.postRebuildCost).toBe(3_000 * RATES.write);
});

it("an expected expand cost can turn a marginal rebuild down", () => {
  const marginal = { remainingTurns: 4 };
  expect(classify(marginal).decision.rebuild).toBe(true);
  const costly = classify({
    ...marginal,
    expands: expandEstimateFor([10_000, 10_000], 2),
  });
  expect(costly.decision.rebuild).toBe(false);
  expect(costly.decision.reason).toBe("payback:5>4");
});

function append(log: MemoryLogStore, message: Message): void {
  log.append("s", { kind: "message", payload: message });
}

function expandCall(id: string, target: string): Message {
  return {
    role: "assistant",
    content: [
      {
        type: "tool_use",
        id,
        name: EXPAND_TOOL_NAME,
        input: { id: target },
        rawInput: JSON.stringify({ id: target }),
      },
    ],
  };
}

function result(id: string, content: string, isError = false): Message {
  return {
    role: "user",
    content: [{ type: "tool_result", toolUseId: id, content, isError }],
  };
}

it("reads the token size of every successful expand result on the path", () => {
  const log = new MemoryLogStore();
  append(log, { role: "user", content: [{ type: "text", text: "task" }] });
  append(log, expandCall("e1", "chunk-a"));
  append(log, result("e1", "x".repeat(40)));
  append(log, expandCall("e2", "missing"));
  append(log, result("e2", "unknown id", true));
  append(log, {
    role: "assistant",
    content: [
      { type: "tool_use", id: "b1", name: "bash", input: {}, rawInput: "{}" },
    ],
  });
  append(log, result("b1", "y".repeat(400)));
  const count = (text: string) => text.length;
  expect(expandResultTokens(log.path("s"), count)).toEqual([40]);
});

function manifest(epoch: number, turn: number, decision?: object) {
  return {
    kind: "manifest" as const,
    payload: {
      epoch,
      turn,
      claims: [],
      chunks: [],
      anomalies: [],
      ...(decision
        ? { epochDecision: { remainingTurns: 20, ...decision } }
        : {}),
    },
  };
}

it("reads the epoch's age and committed payback from its first manifest", () => {
  const log = new MemoryLogStore();
  expect(epochCommitment(log.path("s"), 0, 1)).toEqual({
    turnsSinceEpochOpen: 0,
    committedPaybackTurns: 0,
  });
  log.append("s", manifest(0, 1, { rebuild: false, paybackTurns: 30 }));
  log.append("s", manifest(0, 2));
  expect(epochCommitment(log.path("s"), 0, 3)).toEqual({
    turnsSinceEpochOpen: 2,
    committedPaybackTurns: 0,
  });
  log.append("s", {
    kind: "view_epoch",
    payload: { epoch: 1, reason: "cost:7" },
  });
  expect(epochCommitment(log.path("s"), 1, 3)).toEqual({
    turnsSinceEpochOpen: 0,
    committedPaybackTurns: 0,
  });
  log.append("s", manifest(1, 3, { rebuild: true, paybackTurns: 7 }));
  log.append("s", manifest(1, 4, { rebuild: true, paybackTurns: 2 }));
  expect(epochCommitment(log.path("s"), 1, 5)).toEqual({
    turnsSinceEpochOpen: 2,
    committedPaybackTurns: 7,
  });
});

it("the compiler copies the epoch's age and committed payback from the Log onto the manifest", async () => {
  const log = new MemoryLogStore();
  append(log, { role: "user", content: [{ type: "text", text: "task" }] });
  log.append("s", {
    kind: "view_epoch",
    payload: { epoch: 1, reason: "cost:7" },
  });
  log.append("s", manifest(1, 3, { rebuild: true, paybackTurns: 7 }));
  const compiler = contextCompiler({ budget: 1_000_000, claims: [] });
  const view = await compiler.view({
    sessionId: "s",
    turn: 6,
    path: log.path("s"),
    model: { provider: "anthropic", model: "claude-sonnet-5" },
    prepared: {
      model: { provider: "anthropic", model: "claude-sonnet-5" },
      contextWindow: 1_000_000,
      pricing: pricing({}),
      adapterDefaults: {},
    },
    head: { system: "You are sage.", tools: [] },
    tools: [],
  });
  expect(view.manifest.epochDecision).toMatchObject({
    turnsSinceEpochOpen: 3,
    committedPaybackTurns: 7,
    rebuild: false,
  });
});
