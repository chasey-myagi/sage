import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  appendRequestEntries,
  type HarnessTool,
  InvalidEntryError,
  isRecord,
  JsonlLogStore,
  MemoryLogStore,
  type RequestHeaderPayload,
  requestHeaderOf,
  type StoredEntry,
} from "@sage/core";
import type { PreparedCall } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { systemReminder } from "../../src/claims/system-reminder.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { finite } from "../../src/guards.ts";

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
    const ctx = viewContext(slice, shadow.path(shadowId), tools);
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

type Slice = {
  turn: number;
  prefixEnd: number;
  assistant: StoredEntry;
  header: RequestHeaderPayload;
};

function sliceTurns(path: readonly StoredEntry[]): Slice[] {
  const slices: Slice[] = [];
  for (let index = 0; index < path.length; index += 1) {
    const entry = path[index];
    const header = entry && requestHeaderOf(entry);
    if (!header) continue;
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
    slices.push({ turn, prefixEnd, assistant, header });
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

function viewContext(
  slice: Slice,
  prefix: readonly StoredEntry[],
  tools: HarnessTool[],
) {
  const { header } = slice;
  const model = { provider: header.provider, model: header.model };
  const prepared: PreparedCall = {
    model,
    contextWindow: 1_000_000,
    pricing: DEEPSEEK,
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
    pathTool("update_ledger", "read"),
    pathTool("expand", "read"),
    pathTool("spawn_subtask", "read"),
    pathTool("spawn_subtasks", "read"),
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
      if (
        name === "bash" ||
        name === "update_ledger" ||
        name === "expand" ||
        name === "spawn_subtask" ||
        name === "spawn_subtasks"
      ) {
        return [];
      }
      if (!isRecord(input) || typeof input.path !== "string") return [];
      if (input.path.length === 0) return [];
      return [{ key: input.path, mode }];
    },
    execute() {
      return Promise.resolve({ content: "", isError: false });
    },
  };
}
