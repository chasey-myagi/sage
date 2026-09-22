import { MemoryLogStore } from "@sage/core";
import type { Message } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { measuredInput } from "../../src/compiler/calibrate.ts";
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

it("records calibration and scales later newTokens by the ratio", async () => {
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
  expect(calibration?.measured).toBe(120000);
  expect(calibration?.estimated).toBe(estimated.used);
  expect(calibration?.ratio).toBe(120000 / estimated.used);
  const decision = second.manifest.epochDecision;
  expect(calibration?.ratio).not.toBe(1);
  expect(decision?.newTokens).toBeCloseTo(
    (typeof second.manifest.budget === "object" && second.manifest.budget
      ? second.manifest.budget.used
      : 0) * (calibration?.ratio ?? 1),
    5,
  );
});
