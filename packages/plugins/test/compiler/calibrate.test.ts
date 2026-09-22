import { InvalidEntryError, MemoryLogStore } from "@sage/core";
import type { Message } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { calibrationFor, measuredInput } from "../../src/compiler/calibrate.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";

const MODEL = { provider: "anthropic", model: "claude-sonnet-5" };

function context(store: MemoryLogStore, turn: number) {
  return {
    sessionId: "s",
    turn,
    path: store.path("s"),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: 1_000_000,
      pricing: {
        inputPerMTok: 2,
        outputPerMTok: 10,
        cacheWritePerMTok: 2.5,
        cacheReadPerMTok: 0.2,
        source: "https://platform.claude.com/docs/en/about-claude/pricing",
      },
      adapterDefaults: {},
    },
    head: { system: "You are sage.", tools: [] },
    tools: [],
  };
}

function user(text: string): Message {
  return { role: "user", content: [{ type: "text", text }] };
}

it("reads the previous assistant input total", () => {
  const store = new MemoryLogStore();
  expect(measuredInput(store.path("s"))).toBeUndefined();
  store.append("s", {
    kind: "message",
    payload: user("hi"),
  });
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [{ type: "text", text: "ok" }],
    },
    details: {
      usage: {
        inputOther: 20000,
        inputCacheRead: 100000,
        inputCacheCreation: 0,
        output: 12,
      },
    },
  });
  expect(measuredInput(store.path("s"))).toBe(120000);
});

it("records a fixed head and projects old and new tokens as head plus used", async () => {
  const compiler = contextCompiler({
    budget: 8000,
    claims: [],
    expectedTurns: 1,
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("hello") });
  const first = await compiler.view(context(store, 1));
  expect(first.manifest.calibration).toBeUndefined();
  const estimated = first.manifest.budget;
  if (typeof estimated === "number" || estimated === undefined) {
    throw new Error("expected a budget report");
  }
  store.append("s", { kind: "manifest", payload: first.manifest });
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [{ type: "text", text: "ok" }],
    },
    details: {
      usage: {
        inputOther: 20000,
        inputCacheRead: 100000,
        inputCacheCreation: 0,
        output: 4,
      },
    },
  });
  store.append("s", { kind: "message", payload: user("next") });

  const second = await compiler.view(context(store, 2));
  const calibration = second.manifest.calibration;
  const head = 120000 - estimated.used;
  expect(calibration?.measured).toBe(120000);
  expect(calibration?.estimated).toBe(estimated.used);
  expect(calibration?.head).toBe(head);
  expect(calibration?.ratio).toBe(1);
  const decision = second.manifest.epochDecision;
  const secondUsed =
    typeof second.manifest.budget === "object" && second.manifest.budget
      ? second.manifest.budget.used
      : 0;
  expect(decision?.newTokens).toBeCloseTo(head + secondUsed, 5);
  expect(decision?.oldTokens).toBeCloseTo(head + secondUsed, 5);

  store.append("s", { kind: "manifest", payload: second.manifest });
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [{ type: "text", text: "next-ok" }],
    },
    details: {
      usage: {
        inputOther: head + secondUsed + 4,
        inputCacheRead: 0,
        inputCacheCreation: 0,
        output: 4,
      },
    },
  });
  store.append("s", { kind: "message", payload: user("after") });
  const third = await compiler.view(context(store, 3));
  const later = third.manifest.calibration;
  const laterRatio = 4 / secondUsed + 1;
  expect(later?.head).toBe(head);
  expect(later?.ratio).toBeCloseTo(laterRatio, 5);
  expect(laterRatio).toBeGreaterThan(1);
  const thirdUsed =
    typeof third.manifest.budget === "object" && third.manifest.budget
      ? third.manifest.budget.used
      : 0;
  expect(third.manifest.epochDecision?.oldTokens).toBeCloseTo(
    head + thirdUsed,
    5,
  );
});

it("floors head at 0 when the counter already exceeds the measured input", () => {
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 1,
      budget: { total: 100, used: 100, overrun: 0 },
      claims: [],
      chunks: [],
      anomalies: [],
    },
  });
  store.append("s", {
    kind: "message",
    payload: { role: "assistant", content: [{ type: "text", text: "ok" }] },
    details: {
      usage: {
        inputOther: 20,
        inputCacheRead: 0,
        inputCacheCreation: 0,
        output: 1,
      },
    },
  });
  const calibration = calibrationFor(store.path("s"));
  expect(calibration?.head).toBe(0);
  expect(calibration?.measured).toBe(20);
  expect(calibration?.estimated).toBe(100);
  expect(calibration?.ratio).toBe(0.2);
});

it("rejects a calibration head that is not a finite number", () => {
  const store = new MemoryLogStore();
  expect(() =>
    store.append("s", {
      kind: "manifest",
      payload: {
        epoch: 0,
        turn: 1,
        claims: [],
        chunks: [],
        anomalies: [],
        calibration: { measured: 1, estimated: 1, ratio: 1, head: "nope" },
      },
    }),
  ).toThrow(InvalidEntryError);
});
