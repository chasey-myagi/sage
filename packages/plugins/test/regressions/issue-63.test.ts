import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  appendRequestEntries,
  type HarnessTool,
  InvalidEntryError,
  JsonlLogStore,
  MemoryLogStore,
} from "@sage/core";
import { pathlessTool, pathTool } from "@sage/core/testing";
import { expect, it } from "vitest";
import { systemReminder } from "../../src/claims/system-reminder.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import {
  buildViewContext,
  copyMessages,
  sliceTurns,
} from "../../src/replay/offline.ts";

const SESSION = "budget8000-vue10874-36turns";
const SYSTEM = "You are sage.";
const DEEPSEEK = {
  inputPerMTok: 0.3,
  outputPerMTok: 1.2,
  cacheWritePerMTok: 0,
  cacheReadPerMTok: 0.006,
  source: "https://api-docs.deepseek.com/quick_start/pricing",
};
const fixtureDir = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "../replay/fixtures/issue-63",
);

type Row = {
  turn: number;
  remainingTurns: number;
  paybackTurns: number | null;
  rebuild: boolean;
  reason: string;
};

it("keeps a growing horizon on the 36-turn vue trace", async () => {
  const rows = await replayRows();
  expect(rows).toHaveLength(36);
  for (const row of rows) {
    expect(row.remainingTurns, `turn ${row.turn}`).toEqual(expect.any(Number));
    expect(Number.isFinite(row.remainingTurns), `turn ${row.turn}`).toBe(true);
  }
  expect(rows.find((row) => row.turn === 30)?.remainingTurns).toBe(15);
  expect(rows.find((row) => row.turn === 36)?.remainingTurns).toBe(18);
});

it("rejects a manifest whose epoch decision omits remainingTurns", () => {
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
        epochDecision: { rebuild: false },
      },
    }),
  ).toThrow(InvalidEntryError);
});

it("lets alpha raise the horizon above the fixed expectedTurns remainder", async () => {
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "message",
    payload: { role: "user", content: [{ type: "text", text: "hi" }] },
  });
  const compiler = contextCompiler({
    budget: 8000,
    claims: [],
    expectedTurns: 20,
    alpha: 1,
  });
  const view = await compiler.view({
    sessionId: "s",
    turn: 36,
    path: store.path("s"),
    model: { provider: "deepseek", model: "deepseek-flash" },
    prepared: {
      model: { provider: "deepseek", model: "deepseek-flash" },
      contextWindow: 1_000_000,
      pricing: DEEPSEEK,
      adapterDefaults: {},
    },
    head: { system: SYSTEM, tools: [] },
    tools: [],
    depth: 0,
  });
  expect(view.manifest.epochDecision?.remainingTurns).toBe(35);
});

async function replayRows(): Promise<Row[]> {
  const recorded = new JsonlLogStore({ dir: fixtureDir }).path(SESSION);
  const shadow = new MemoryLogStore();
  const shadowId = "offline";
  const tools = traceTools();
  const provider = contextCompiler({
    budget: 8000,
    claims: [systemReminder({ text: "Work in small verified steps." })],
    keepRecent: 6,
    expectedTurns: 20,
  });
  let copiedThrough = 0;
  const rows: Row[] = [];
  for (const slice of sliceTurns(recorded)) {
    copyMessages(recorded, copiedThrough, slice.prefixEnd, shadow, shadowId);
    copiedThrough = slice.prefixEnd;
    const ctx = buildViewContext(
      slice.turn,
      { ...slice.header, contextWindow: 1_000_000, pricing: DEEPSEEK },
      shadow.path(shadowId),
      { tools, system: SYSTEM },
    );
    const view = await provider.view(ctx);
    const decision = view.manifest.epochDecision;
    rows.push({
      turn: slice.turn,
      remainingTurns: decision?.remainingTurns ?? Number.NaN,
      paybackTurns: decision?.paybackTurns ?? null,
      rebuild: decision?.rebuild ?? false,
      reason: decision?.reason ?? "",
    });
    appendRequestEntries(shadow, shadowId, {
      view,
      model: ctx.model,
      sampling: {},
      prepared: ctx.prepared,
    });
  }
  return rows;
}

function traceTools(): HarnessTool[] {
  return [
    pathTool("read", "read"),
    pathTool("edit", "write"),
    pathTool("write", "write"),
    pathTool("ls", "read"),
    pathlessTool("bash"),
    pathTool("grep", "search"),
    pathTool("find", "search"),
    pathlessTool("update_ledger"),
    pathlessTool("expand"),
    pathlessTool("spawn_subtask"),
    pathlessTool("spawn_subtasks"),
  ];
}
