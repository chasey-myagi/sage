import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  type HarnessTool,
  JsonlLogStore,
  MemoryLogStore,
  type StoredEntry,
  systemHash,
  toolsHash,
} from "@sage/core";
import type { Head, Message, PreparedCall } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { replayOffline } from "../../src/replay/offline.ts";

const SESSION = "long-deepseek-flash";
const SYSTEM = "You are sage.";
const fixtureDir = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "../replay/fixtures/issue-50",
);

type Row = {
  turn: number;
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

it("does not open context-window-guard on turn 3 of the 8000-window replay", async () => {
  const diffs = await replayOffline(loadStore(), SESSION, policy(), deps());
  const turn3 = diffs.find((diff) => diff.turn === 3);
  expect(turn3?.decisionReason ?? "").not.toContain("context-window-guard");
  const turn2 = diffs.find((diff) => diff.turn === 2);
  expect(turn2?.decisionReason ?? "").not.toContain("context-window-guard");
});

function relative(estimate: number, actual: number): number {
  if (actual === 0) return estimate === 0 ? 0 : Number.POSITIVE_INFINITY;
  return Math.abs(estimate - actual) / actual;
}

function policy() {
  return { budget: 40_000, keepRecent: 6, expectedTurns: 20 };
}

function deps() {
  return { tools: traceTools(), system: SYSTEM };
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
    const ctx = viewContext(slice, prefix, tools);
    const view = await provider.view(ctx);
    const decision = view.manifest.epochDecision;
    rows.push({
      turn: slice.turn,
      recorded: inputTotal(slice.assistant),
      oldTokens: decision?.oldTokens ?? 0,
      reason: decision?.reason ?? "",
      ratio: view.manifest.calibration?.ratio,
      expected: view.manifest.cache?.expectedReadTokens ?? 0,
      measuredCache: view.manifest.cache?.measuredReadTokens ?? 0,
    });
    commitView(shadow, shadowId, view, ctx);
  }
  return rows;
}

type Slice = {
  turn: number;
  prefixEnd: number;
  assistant: StoredEntry;
  header: StoredEntry;
};

function sliceTurns(path: readonly StoredEntry[]): Slice[] {
  const slices: Slice[] = [];
  for (let index = 0; index < path.length; index += 1) {
    const entry = path[index];
    if (entry?.kind !== "request_header") continue;
    let prefixEnd = index;
    while (prefixEnd > 0) {
      const prev = path[prefixEnd - 1];
      if (prev?.kind !== "view_epoch" && prev?.kind !== "model_change") break;
      prefixEnd -= 1;
    }
    let turn = slices.length + 1;
    let assistant: StoredEntry | undefined;
    for (let cursor = index + 1; cursor < path.length; cursor += 1) {
      const next = path[cursor];
      if (!next || next.kind === "request_header") break;
      if (next.kind === "manifest" && isRecord(next.payload)) {
        const value = finite(next.payload.turn);
        if (value !== undefined) turn = value;
      }
      if (isAssistantEntry(next)) {
        assistant = next;
        break;
      }
    }
    if (!assistant) continue;
    slices.push({ turn, prefixEnd, assistant, header: entry });
  }
  return slices;
}

function copyMessages(
  recorded: readonly StoredEntry[],
  from: number,
  to: number,
  shadow: MemoryLogStore,
  sessionId: string,
): void {
  for (let index = from; index < to; index += 1) {
    const entry = recorded[index];
    if (entry?.kind !== "message") continue;
    shadow.append(sessionId, {
      kind: "message",
      payload: entry.payload,
      ...(entry.details ? { details: entry.details } : {}),
    });
  }
}

function commitView(
  shadow: MemoryLogStore,
  sessionId: string,
  view: {
    epoch: number;
    epochStart?: { reason: string };
    messages: Message[];
    manifest: unknown;
    details?: Record<string, unknown>;
  },
  ctx: {
    model: { provider: string; model: string };
    prepared: PreparedCall;
    head: Head;
  },
): void {
  const path = shadow.path(sessionId);
  let latest = 0;
  let seen = false;
  for (const entry of path) {
    if (entry.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    const epoch = finite(entry.payload.epoch);
    if (epoch === undefined) continue;
    latest = epoch;
    seen = true;
  }
  const baseline = seen ? latest : 0;
  if (view.epochStart || view.epoch > baseline) {
    const epochHead = view.messages[0];
    shadow.append(sessionId, {
      kind: "view_epoch",
      payload: {
        epoch: view.epoch,
        reason: view.epochStart?.reason ?? "epoch-advanced",
      },
      ...(epochHead ? { details: { epochHead } } : {}),
    });
  }
  shadow.append(sessionId, {
    kind: "request_header",
    payload: {
      provider: ctx.model.provider,
      model: ctx.model.model,
      systemPromptHash: systemHash(ctx.head.system),
      toolsHash: toolsHash(ctx.head.tools),
      sampling: {},
      adapterDefaults: { ...ctx.prepared.adapterDefaults },
      system: ctx.head.system,
      tools: ctx.head.tools,
    },
  });
  shadow.append(sessionId, {
    kind: "manifest",
    payload: view.manifest,
    ...(view.details ? { details: view.details } : {}),
  });
}

function viewContext(
  slice: Slice,
  prefix: readonly StoredEntry[],
  tools: HarnessTool[],
) {
  const header = headerOf(slice.header);
  const model = { provider: header.provider, model: header.model };
  const prepared: PreparedCall = {
    model,
    contextWindow: 8000,
    pricing: null,
    adapterDefaults: header.adapterDefaults,
  };
  const head = {
    system: SYSTEM,
    tools: tools.map((tool) => tool.def),
  };
  return {
    sessionId: "offline",
    turn: slice.turn,
    path: prefix,
    model,
    prepared,
    head,
    tools,
    depth: 0 as const,
  };
}

function headerOf(entry: StoredEntry): {
  provider: string;
  model: string;
  adapterDefaults: Record<string, unknown>;
} {
  if (!isRecord(entry.payload)) {
    throw new Error("request_header payload is not an object");
  }
  const provider = entry.payload.provider;
  const model = entry.payload.model;
  if (typeof provider !== "string" || typeof model !== "string") {
    throw new Error("request_header is missing provider or model");
  }
  const defaults = entry.payload.adapterDefaults;
  return {
    provider,
    model,
    adapterDefaults: isRecord(defaults) ? defaults : {},
  };
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

function isAssistantEntry(entry: StoredEntry): boolean {
  return (
    entry.kind === "message" &&
    isRecord(entry.payload) &&
    entry.payload.role === "assistant"
  );
}

function traceTools(): HarnessTool[] {
  return [
    pathTool("read", "read"),
    pathTool("edit", "write"),
    pathTool("write", "write"),
    pathTool("ls", "read"),
    pathTool("bash", "read"),
    pathTool("grep", "search"),
    pathTool("find", "search"),
  ];
}

function pathTool(
  name: string,
  mode: "read" | "write" | "search",
): HarnessTool {
  return {
    def: {
      name,
      description: name,
      inputSchema: { type: "object" },
    },
    isReadOnly: mode !== "write",
    isDestructive: mode === "write",
    isOpenWorld: false,
    accesses(input: unknown) {
      if (name === "bash") return [];
      if (!isRecord(input) || typeof input.path !== "string") return [];
      if (input.path.length === 0) return [];
      return [{ key: input.path, mode }];
    },
    execute() {
      return Promise.resolve({ content: "", isError: false });
    },
  };
}

function finite(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
