import { MemoryLogStore } from "@sage/core";
import type { Message, Pricing } from "@sage/provider/contract";
import { expect, it } from "vitest";
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
import { EXPAND_TOOL_NAME } from "../../src/compiler/expand.ts";
import { turnsSinceEpochOpen } from "../../src/compiler/freeze.ts";
import { expandResultTokens } from "../../src/compiler/log-read.ts";

// Observed zero expands: the cost rule without any expand term.
const NO_EXPANDS: ExpandEstimate = {
  source: "observed",
  expandsPerEpoch: 0,
  tokensPerExpand: 0,
};

const RATES: Rates = {
  read: 0.1,
  write: 1.25,
  readSource: "pricing",
  writeSource: "pricing",
};

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
    minRebuildInterval: 8,
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

it("takes both cache rates from the pricing when it lists a write price", () => {
  expect(ratesFor(pricing({}))).toEqual({
    read: 0.1,
    write: 1.25,
    readSource: "pricing",
    writeSource: "pricing",
  });
});

it("falls back to the default rates and marks both sources when pricing is missing", () => {
  const rates = ratesFor(null);
  expect(rates).toEqual({
    read: DEFAULT_READ_RATE,
    write: DEFAULT_WRITE_RATE,
    readSource: "default",
    writeSource: "default",
  });
  expect(ratesFor(pricing({ inputPerMTok: 0 })).readSource).toBe("default");
});

it("keeps the measured read rate but defaults the write rate when the pricing lists no write price", () => {
  expect(
    ratesFor(
      pricing({
        inputPerMTok: 6.5,
        cacheReadPerMTok: 1.3,
        cacheWritePerMTok: 0,
      }),
    ),
  ).toEqual({
    read: 0.2,
    write: DEFAULT_WRITE_RATE,
    readSource: "pricing",
    writeSource: "default",
  });
});

it("never prices a cache write below the input price", () => {
  const rates = ratesFor(pricing({ cacheWritePerMTok: 1 }));
  expect(rates.write).toBe(1);
  expect(rates.writeSource).toBe("pricing");
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

it("records the rate sources on the decision", () => {
  const { decision } = classify({ rates: ratesFor(null) });
  expect(decision.readRateSource).toBe("default");
  expect(decision.writeRateSource).toBe("default");
});

it("holds a rebuild the cost rule approves until the epoch is old enough", () => {
  const held = classify({ turnsSinceEpochOpen: 3, minRebuildInterval: 8 });
  expect(held.decision.rebuild).toBe(false);
  expect(held.trigger).toBe("no-rebuild");
  expect(held.decision.reason).toBe("min-interval:3<8");
  expect(held.decision.paybackTurns).toBe(4);
  expect(held.decision.turnsSinceEpochOpen).toBe(3);
  expect(held.decision.minRebuildInterval).toBe(8);

  const due = classify({ turnsSinceEpochOpen: 8, minRebuildInterval: 8 });
  expect(due.decision.rebuild).toBe(true);
  expect(due.decision.reason).toBe("payback:4<=20");
});

it("holds an immediate rebuild too while the epoch is young", () => {
  const held = classify({
    oldTokens: 400_000,
    newTokens: 30_000,
    turnsSinceEpochOpen: 1,
  });
  expect(held.decision.extraCost).toBeLessThanOrEqual(0);
  expect(held.decision.rebuild).toBe(false);
  expect(held.decision.reason).toBe("min-interval:1<8");
});

it("the context-window guard still fires inside the minimum interval", () => {
  const guarded = classify({
    turnsSinceEpochOpen: 0,
    minRebuildInterval: 8,
    overContext: true,
  });
  expect(guarded.decision.rebuild).toBe(true);
  expect(guarded.trigger).toBe("context-window-guard");
  expect(guarded.decision.reason).toBe("context-window-guard");
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

function store(): MemoryLogStore {
  return new MemoryLogStore();
}

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
  const log = store();
  append(log, { role: "user", content: [{ type: "text", text: "task" }] });
  append(log, expandCall("e1", "chunk-a"));
  append(log, result("e1", "x".repeat(40)));
  append(log, expandCall("e2", "missing"));
  append(log, result("e2", "unknown id", true));
  append(log, {
    role: "assistant",
    content: [
      {
        type: "tool_use",
        id: "b1",
        name: "bash",
        input: {},
        rawInput: "{}",
      },
    ],
  });
  append(log, result("b1", "y".repeat(400)));
  const count = (text: string) => text.length;
  expect(expandResultTokens(log.path("s"), count)).toEqual([40]);
});

it("counts turns since the current epoch's first manifest", () => {
  const log = store();
  expect(turnsSinceEpochOpen(log.path("s"), 0, 1)).toBe(0);
  log.append("s", {
    kind: "manifest",
    payload: { epoch: 0, turn: 1, claims: [], chunks: [], anomalies: [] },
  });
  log.append("s", {
    kind: "manifest",
    payload: { epoch: 0, turn: 2, claims: [], chunks: [], anomalies: [] },
  });
  expect(turnsSinceEpochOpen(log.path("s"), 0, 3)).toBe(2);
  log.append("s", {
    kind: "view_epoch",
    payload: { epoch: 1, reason: "cost:4" },
  });
  expect(turnsSinceEpochOpen(log.path("s"), 1, 3)).toBe(0);
  log.append("s", {
    kind: "manifest",
    payload: { epoch: 1, turn: 3, claims: [], chunks: [], anomalies: [] },
  });
  expect(turnsSinceEpochOpen(log.path("s"), 1, 5)).toBe(2);
});
