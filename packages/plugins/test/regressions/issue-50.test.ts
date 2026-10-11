import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  appendRequestEntries,
  type HarnessTool,
  isRecord,
  JsonlLogStore,
  MemoryLogStore,
  type StoredEntry,
} from "@sage/core";
import { pathlessTool, pathTool } from "@sage/core/testing";
import { expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { finite } from "../../src/guards.ts";
import {
  buildViewContext,
  copyMessages,
  sliceTurns,
} from "../../src/replay/offline.ts";

const SESSION = "long-deepseek-flash";
const SYSTEM = "You are sage.";
const fixtureDir = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "../replay/fixtures/issue-50",
);

type Row = {
  turn: number;
  epoch: number;
  recorded: number;
  oldTokens: number;
  reason: string;
  ratio: number | undefined;
  expected: number;
  measuredCache: number;
};

it("keeps oldTokens within 15% of measured input and ratios at or below 2", async () => {
  const rows = await replayRows();
  const misses: string[] = [];
  for (const row of rows) {
    if (row.turn < 2) continue;
    const error = relative(row.oldTokens, row.recorded);
    if (error > 0.15) {
      misses.push(
        `turn ${row.turn} old ${row.oldTokens} measured ${row.recorded} error ${error.toFixed(3)}`,
      );
    }
  }
  expect(misses, misses.join("\n")).toEqual([]);
  for (const row of rows) {
    if (row.turn < 2 || row.turn > 5) continue;
    expect(row.ratio, `turn ${row.turn}`).toBeTypeOf("number");
    expect(row.ratio ?? Number.POSITIVE_INFINITY).toBeLessThanOrEqual(2);
  }
});

it("keeps cache.expectedReadTokens within 15% of measuredReadTokens after turn 3", async () => {
  const rows = await replayRows();
  const misses: string[] = [];
  for (const row of rows) {
    if (row.turn < 3) continue;
    const error = relative(row.expected, row.measuredCache);
    if (error > 0.15) {
      misses.push(
        `turn ${row.turn} expected ${row.expected} measured ${row.measuredCache} error ${error.toFixed(3)}`,
      );
    }
  }
  expect(misses, misses.join("\n")).toEqual([]);
});

it("opens context-window-guard only for a real over-window input, and on the first over-window old estimate of each epoch", async () => {
  const rows = await replayRows();
  const limit = 8000 * 0.9;
  const falseAlarms: string[] = [];
  for (const row of rows) {
    if (!row.reason.includes("context-window-guard")) continue;
    if (row.recorded > limit) continue;
    falseAlarms.push(
      `turn ${row.turn} guard opened measured ${row.recorded} limit ${limit}`,
    );
  }
  expect(falseAlarms, falseAlarms.join("\n")).toEqual([]);

  const missed: string[] = [];
  const epochs = new Map<number, Row[]>();
  for (const row of rows) {
    const group = epochs.get(row.epoch) ?? [];
    group.push(row);
    epochs.set(row.epoch, group);
  }
  for (const [epoch, group] of epochs) {
    const first = group.find((row) => row.oldTokens > limit);
    if (!first || first.reason.includes("context-window-guard")) continue;
    missed.push(
      `epoch ${epoch} turn ${first.turn} old ${first.oldTokens} measured ${first.recorded} limit ${limit} reason ${first.reason}`,
    );
  }
  expect(missed, missed.join("\n")).toEqual([]);

  const guarded = rows
    .filter((row) => row.reason.includes("context-window-guard"))
    .map((row) => row.turn);
  expect(guarded).toEqual([3]);
});

function relative(estimate: number, actual: number): number {
  if (actual === 0) return estimate === 0 ? 0 : Number.POSITIVE_INFINITY;
  return Math.abs(estimate - actual) / actual;
}

function loadStore(): JsonlLogStore {
  return new JsonlLogStore({ dir: fixtureDir });
}

async function replayRows(): Promise<Row[]> {
  const recorded = loadStore().path(SESSION);
  const shadow = new MemoryLogStore();
  const shadowId = "offline";
  const tools = traceTools();
  const provider = contextCompiler({
    budget: 40_000,
    claims: [],
    keepRecent: 6,
    expectedTurns: 20,
  });
  let copiedThrough = 0;
  const rows: Row[] = [];
  for (const slice of sliceTurns(recorded)) {
    copyMessages(recorded, copiedThrough, slice.prefixEnd, shadow, shadowId);
    copiedThrough = slice.prefixEnd;
    const prefix = shadow.path(shadowId);
    const ctx = buildViewContext(
      slice.turn,
      { ...slice.header, contextWindow: 8000, pricing: null },
      prefix,
      { tools, system: SYSTEM },
    );
    const view = await provider.view(ctx);
    const decision = view.manifest.epochDecision;
    rows.push({
      turn: slice.turn,
      epoch: view.epoch,
      recorded: inputTotal(slice.assistant),
      oldTokens: decision?.oldTokens ?? 0,
      reason: decision?.reason ?? "",
      ratio: view.manifest.calibration?.ratio,
      expected: view.manifest.cache?.expectedReadTokens ?? 0,
      measuredCache: view.manifest.cache?.measuredReadTokens ?? 0,
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
  ];
}

function inputTotal(entry: StoredEntry): number {
  const usage = entry.details?.usage;
  if (!isRecord(usage)) return 0;
  return (
    (finite(usage.inputOther) ?? 0) +
    (finite(usage.inputCacheRead) ?? 0) +
    (finite(usage.inputCacheCreation) ?? 0)
  );
}
