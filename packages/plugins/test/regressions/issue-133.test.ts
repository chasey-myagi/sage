import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  type HarnessTool,
  isRecord,
  JsonlLogStore,
  MemoryLogStore,
  type StoredEntry,
} from "@sage/core";
import { listModels } from "@sage/provider";
import type { ToolDef } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { caseDir, REPLAY_CASES } from "../../../core/test/replay/cases.ts";
import { replayOffline } from "../../src/replay/offline.ts";

const fixtureDir = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "../replay/fixtures/issue-63",
);
const SESSION = "budget8000-vue10874-36turns";

it("replays a recorded deepseek session to the same rebuild flag and deciding rule on every turn", async () => {
  const recorded = new JsonlLogStore({ dir: fixtureDir }).path(SESSION);
  const head = recordedHead(recorded);
  // The recording predates the adaptive horizon: its remainingTurns is
  // max(1, expectedTurns - started), which alpha 0 reproduces.
  const diffs = await replayOffline(
    new JsonlLogStore({ dir: fixtureDir }),
    SESSION,
    { budget: 8000, keepRecent: 6, expectedTurns: 20, alpha: 0 },
    { tools: head.tools.map(pathTool), system: head.system },
  );
  // Payback numbers in the reason come from the cost model in force when
  // the session was recorded, so only the rule name before ":" is compared.
  const replayed = diffs.map((diff) => ({
    turn: diff.turn,
    rebuild: diff.rebuild,
    rule: ruleOf(diff.decisionReason),
  }));
  expect(replayed).toEqual(recordedDecisions(recorded));
});

it("writes system and tools only on the first replayed request_header when the head is unchanged", async () => {
  const spec = REPLAY_CASES.find((item) => item.name === "tool-roundtrip");
  if (!spec) throw new Error("missing tool-roundtrip case");
  const shadow = new MemoryLogStore();
  await replayOffline(
    new JsonlLogStore({ dir: caseDir(spec.name) }),
    "session",
    { budget: 8000 },
    { tools: spec.tools, system: spec.system, shadow },
  );
  const headers = shadow
    .path("offline")
    .filter((entry) => entry.kind === "request_header")
    .map((entry) => entry.payload);
  expect(headers).toHaveLength(2);
  const [first, second] = headers;
  expect(first).toMatchObject({
    system: spec.system,
    tools: spec.tools.map((tool) => tool.def),
  });
  expect(second).not.toHaveProperty("system");
  expect(second).not.toHaveProperty("tools");
  expect(isRecord(first) && isRecord(second)).toBe(true);
  if (!isRecord(first) || !isRecord(second)) return;
  expect(second.systemPromptHash).toBe(first.systemPromptHash);
  expect(second.toolsHash).toBe(first.toolsHash);
});

it("copies the recorded context window and pricing onto the replayed request_header", async () => {
  const shadow = new MemoryLogStore();
  await replayOffline(
    new JsonlLogStore({ dir: fixtureDir }),
    SESSION,
    { budget: 8000 },
    { tools: [], system: "You are sage.", shadow },
  );
  const replayed = shadow
    .path("offline")
    .filter((entry) => entry.kind === "request_header")
    .map((entry) => windowAndPricing(entry.payload));
  const recorded = new JsonlLogStore({ dir: fixtureDir })
    .path(SESSION)
    .filter((entry) => entry.kind === "request_header")
    .map((entry) => windowAndPricing(entry.payload));
  const flash = listModels("deepseek").find(
    (entry) => entry.model === "deepseek-flash",
  );
  expect(replayed).toHaveLength(36);
  expect(replayed[0]).toEqual({
    contextWindow: flash?.contextWindow,
    pricing: flash?.pricing,
  });
  expect(replayed).toEqual(recorded);
});

function windowAndPricing(payload: unknown): {
  contextWindow: unknown;
  pricing: unknown;
} {
  if (!isRecord(payload)) throw new Error("request_header is not an object");
  return { contextWindow: payload.contextWindow, pricing: payload.pricing };
}

type DecisionRow = { turn: unknown; rebuild: unknown; rule: unknown };

function ruleOf(reason: unknown): unknown {
  return typeof reason === "string" ? reason.split(/[:;]/)[0] : reason;
}

function recordedDecisions(recorded: readonly StoredEntry[]): DecisionRow[] {
  const rows: DecisionRow[] = [];
  for (const entry of recorded) {
    if (entry.kind !== "manifest" || !isRecord(entry.payload)) continue;
    const decision = entry.payload.epochDecision;
    if (!isRecord(decision)) throw new Error("manifest has no epochDecision");
    rows.push({
      turn: entry.payload.turn,
      rebuild: decision.rebuild,
      rule: ruleOf(decision.reason),
    });
  }
  return rows;
}

function recordedHead(recorded: readonly StoredEntry[]): {
  system: string;
  tools: ToolDef[];
} {
  const first = recorded.find((entry) => entry.kind === "request_header");
  if (!first || !isRecord(first.payload)) {
    throw new Error("recording has no request_header");
  }
  const { system, tools } = first.payload;
  if (typeof system !== "string" || !Array.isArray(tools)) {
    throw new Error("first request_header lacks system or tools text");
  }
  // `tools` is `unknown` on the header and its validator does not check the
  // schema shape; the fixture was recorded from a real run, so trust it.
  return { system, tools: tools as ToolDef[] };
}

/**
 * A stub with the recorded schema whose accesses are its `path` input, the
 * key the coding tools report.
 */
function pathTool(def: ToolDef): HarnessTool {
  const writes = def.name === "edit" || def.name === "write";
  return {
    def,
    isReadOnly: !writes,
    isDestructive: writes,
    isOpenWorld: false,
    accesses(input: unknown) {
      if (!isRecord(input) || typeof input.path !== "string") return [];
      if (input.path.length === 0) return [];
      return [{ key: input.path, mode: writes ? "write" : "read" }];
    },
    execute() {
      return Promise.resolve({ content: "", isError: false });
    },
  };
}
