import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  type HarnessTool,
  isRecord,
  JsonlLogStore,
  MemoryLogStore,
} from "@sage/core";
import { FAUX_CONTEXT_WINDOW } from "@sage/core/testing";
import { expect, it } from "vitest";
import { caseDir, REPLAY_CASES } from "../../../core/test/replay/cases.ts";
import {
  DEFAULT_WRITE_RATE,
  PRIOR_EXPAND_SHARE,
} from "../../src/compiler/decide.ts";
import { DEFAULT_MIN_REBUILD_INTERVAL } from "../../src/compiler/epoch.ts";
import { compareOffline, replayOffline } from "../../src/replay/offline.ts";
import type { Scorer } from "../../src/scorer/types.ts";

for (const spec of REPLAY_CASES) {
  it(`replays ${spec.name} with passthrough: nothing saved, prefix stays stable`, async () => {
    const store = new JsonlLogStore({ dir: caseDir(spec.name) });
    const diffs = await replayOffline(
      store,
      "session",
      { passthrough: true },
      { tools: spec.tools, system: spec.system },
    );
    expect(diffs.map((diff) => diff.recordedTokens)).toEqual(
      spec.name === "text-only" ? [4] : [4, 7],
    );
    for (const diff of diffs) {
      expect(diff.saved).toBe(0);
      expect(diff.prefixStable).toBe(true);
      expect(diff.policyTokens).toBe(diff.recordedTokens);
      expect(diff.omittedIds).toEqual([]);
      expect(diff.likelyNeededOmissions).toEqual([]);
    }
  });
}

it("reports an omitted tool chunk when a later assistant cites its access key", async () => {
  const store = new MemoryLogStore();
  const sessionId = "cited";
  appendSession(store, sessionId, {
    toolUses: [
      { id: "call_read", name: "read", input: { path: "src/app.ts" } },
      { id: "call_other", name: "read", input: { path: "src/other.ts" } },
    ],
    result: "OMITTED_BODY_SHOULD_NOT_LEAK",
    later: "please reopen src/app.ts",
  });
  const diffs = await replayOffline(
    store,
    sessionId,
    { budget: 0, keepRecent: 0 },
    { tools: [readTool()], system: "You are sage." },
  );
  const cited = diffs.flatMap((diff) => diff.likelyNeededOmissions);
  expect(cited.some((id) => id.endsWith("#call_read"))).toBe(true);
  expect(cited.some((id) => id.endsWith("#call_other"))).toBe(false);
  expect(JSON.stringify(diffs)).not.toContain("OMITTED_BODY_SHOULD_NOT_LEAK");
});

it("reports an omitted tool chunk when a later tool_use input cites its id", async () => {
  const store = new MemoryLogStore();
  const sessionId = "cited-id";
  appendSession(store, sessionId, {
    toolUses: [
      { id: "call_read", name: "read", input: { path: "src/app.ts" } },
    ],
    result: "OMITTED_BODY_SHOULD_NOT_LEAK",
    later: "nothing about the path",
    laterTool: {
      id: "call_next",
      name: "grep",
      input: { pattern: "call_read" },
    },
  });
  const diffs = await replayOffline(
    store,
    sessionId,
    { budget: 0, keepRecent: 0 },
    { tools: [readTool()], system: "You are sage." },
  );
  const cited = diffs.flatMap((diff) => diff.likelyNeededOmissions);
  expect(cited.some((id) => id.endsWith("#call_read"))).toBe(true);
});

it("compares two policies as numbers and ids, without chunk text", async () => {
  const store = new MemoryLogStore();
  const sessionId = "compare";
  appendSession(store, sessionId, {
    toolUses: [
      { id: "call_read", name: "read", input: { path: "src/app.ts" } },
    ],
    result: "OMITTED_BODY_SHOULD_NOT_LEAK",
    later: "please reopen src/app.ts",
  });
  const passthrough = await replayOffline(
    store,
    sessionId,
    { passthrough: true },
    { tools: [readTool()], system: "You are sage." },
  );
  const tight = await replayOffline(
    store,
    sessionId,
    { budget: 0, keepRecent: 0 },
    { tools: [readTool()], system: "You are sage." },
  );
  const report = compareOffline(passthrough, tight);
  const json = report.json as {
    turns: { turn: number; a: number; b: number }[];
    saved: { a: number; b: number };
    prefixStableTurns: { a: number; b: number };
    likelyNeededOmissions: { a: number; b: number };
    notes: string[];
  };
  expect(json.turns.length).toBe(passthrough.length);
  expect(json.saved.a).toBe(0);
  expect(json.saved.b).toBeGreaterThan(0);
  expect(json.prefixStableTurns.a).toBe(passthrough.length);
  expect(json.likelyNeededOmissions.a).toBe(0);
  expect(json.likelyNeededOmissions.b).toBeGreaterThan(0);
  expect(report.markdown).toContain("saved A 0");
  expect(report.markdown).toContain("likelyNeededOmissions");
  const rendered = `${report.markdown}\n${JSON.stringify(report.json)}`;
  expect(rendered).not.toContain("OMITTED_BODY_SHOULD_NOT_LEAK");
  expect(rendered).not.toContain("please reopen");
  expect(rendered).not.toContain("investigate");
});

it("replays a model scorer as the rule scorer and labels the report", async () => {
  const store = new JsonlLogStore({ dir: caseDir("tool-roundtrip") });
  let calls = 0;
  const scorer: Scorer = {
    name: "model",
    score() {
      calls += 1;
      return Promise.resolve({ decisions: [] });
    },
  };
  const diffs = await replayOffline(
    store,
    "session",
    { scorer, budget: 0, keepRecent: 0 },
    {
      tools: REPLAY_CASES[1]?.tools ?? [],
      system: REPLAY_CASES[1]?.system ?? "",
    },
  );
  expect(calls).toBe(0);
  const report = compareOffline(diffs, diffs);
  expect(report.markdown).toContain("scorer:offline-rule");
  const json = report.json as { notes: string[] };
  expect(json.notes).toEqual(["scorer:offline-rule"]);
  expect(report.markdown).not.toContain("ping");
  expect(JSON.stringify(report.json)).not.toContain("ping");
});

it("records every epoch decision field on each manifest of a replayed session", async () => {
  // A recorded deepseek-flash session: its pricing lists a read price and
  // no cache-write price, so the two rate sources differ.
  const dir = path.join(
    path.dirname(fileURLToPath(import.meta.url)),
    "fixtures/issue-63",
  );
  const shadow = new MemoryLogStore();
  await replayOffline(
    new JsonlLogStore({ dir }),
    "budget8000-vue10874-36turns",
    { budget: 8000 },
    { tools: [], system: "You are sage.", shadow },
  );
  const decisions = shadow
    .path("offline")
    .filter((entry) => entry.kind === "manifest")
    .map((entry) =>
      isRecord(entry.payload) ? entry.payload.epochDecision : undefined,
    );
  expect(decisions).toHaveLength(36);
  for (const decision of decisions) {
    expect(decision).toEqual({
      rebuild: expect.any(Boolean),
      oldTokens: expect.any(Number),
      newTokens: expect.any(Number),
      remainingTurns: expect.any(Number),
      readRate: 0.02,
      writeRate: DEFAULT_WRITE_RATE,
      readRateSource: "pricing",
      writeRateSource: "default",
      savingsPerTurn: expect.any(Number),
      extraCost: expect.any(Number),
      postRebuildCost: expect.any(Number),
      expandEstimate: { source: "prior", share: PRIOR_EXPAND_SHARE },
      turnsSinceEpochOpen: expect.any(Number),
      minRebuildInterval: DEFAULT_MIN_REBUILD_INTERVAL,
      paybackTurns: expect.toSatisfy(
        (value: unknown) => value === null || typeof value === "number",
      ),
      reason: expect.any(String),
    });
  }
});

it("scales compiler policy tokens by that turn's calibration ratio", async () => {
  const store = new JsonlLogStore({ dir: caseDir("text-only") });
  const diffs = await replayOffline(
    store,
    "session",
    { budget: 8000 },
    { tools: [], system: "You are sage." },
  );
  expect(diffs).toEqual([
    {
      turn: 1,
      recordedTokens: 4,
      policyTokens: 2,
      saved: 2,
      prefixStable: true,
      omittedIds: [],
      likelyNeededOmissions: [],
      epoch: 0,
      rebuild: false,
      oldTokens: 2,
      newTokens: 2,
      decisionReason: "savings-too-small;pricing:fallback",
      chunkTiers: [{ id: "seq:0", tier: "full" }],
    },
  ]);
});

function readTool(): HarnessTool {
  return {
    def: {
      name: "read",
      description: "Read a file",
      inputSchema: { type: "object" },
    },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses(input: unknown) {
      if (!isRecord(input) || typeof input.path !== "string") return [];
      if (input.path.length === 0) return [];
      return [{ key: input.path, mode: "read" }];
    },
    async execute() {
      return { content: "", isError: false };
    },
  };
}

function appendSession(
  store: MemoryLogStore,
  sessionId: string,
  spec: {
    toolUses: { id: string; name: string; input: { path: string } }[];
    result: string;
    later: string;
    laterTool?: { id: string; name: string; input: { pattern: string } };
  },
): void {
  const usage = {
    inputOther: 10,
    inputCacheRead: 0,
    inputCacheCreation: 0,
    output: 1,
  };
  store.append(sessionId, {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "investigate" }],
    },
  });
  appendTurn(store, sessionId, 1, {
    role: "assistant",
    content: spec.toolUses.map((call) => ({
      type: "tool_use" as const,
      id: call.id,
      name: call.name,
      input: call.input,
      rawInput: JSON.stringify(call.input),
    })),
    usage,
    finish: "tool_use",
  });
  store.append(sessionId, {
    kind: "message",
    payload: {
      role: "user",
      content: spec.toolUses.map((call) => ({
        type: "tool_result" as const,
        toolUseId: call.id,
        content: spec.result,
        isError: false,
      })),
    },
  });
  appendTurn(store, sessionId, 2, {
    role: "assistant",
    content: [{ type: "text", text: "looking" }],
    usage,
    finish: "stop",
  });
  appendTurn(store, sessionId, 3, {
    role: "assistant",
    content: [
      { type: "text", text: spec.later },
      ...(spec.laterTool
        ? [
            {
              type: "tool_use" as const,
              id: spec.laterTool.id,
              name: spec.laterTool.name,
              input: spec.laterTool.input,
              rawInput: JSON.stringify(spec.laterTool.input),
            },
          ]
        : []),
    ],
    usage,
    finish: spec.laterTool ? "tool_use" : "stop",
  });
}

function appendTurn(
  store: MemoryLogStore,
  sessionId: string,
  turn: number,
  assistant: {
    role: "assistant";
    content: (
      | { type: "text"; text: string }
      | {
          type: "tool_use";
          id: string;
          name: string;
          input: { path: string } | { pattern: string };
          rawInput: string;
        }
    )[];
    usage: {
      inputOther: number;
      inputCacheRead: number;
      inputCacheCreation: number;
      output: number;
    };
    finish: "tool_use" | "stop";
  },
): void {
  store.append(sessionId, {
    kind: "request_header",
    payload: {
      provider: "anthropic",
      model: "claude-test",
      systemPromptHash: "abc",
      toolsHash: "def",
      sampling: { maxTokens: 64 },
      adapterDefaults: { maxTokens: 64 },
      contextWindow: FAUX_CONTEXT_WINDOW,
      pricing: null,
      system: "You are sage.",
      tools: [],
    },
  });
  store.append(sessionId, {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn,
      claims: [],
      chunks: [],
      anomalies: [],
    },
  });
  store.append(sessionId, {
    kind: "message",
    payload: { role: assistant.role, content: assistant.content },
    details: {
      usage: assistant.usage,
      finish: { kind: assistant.finish },
    },
  });
}
