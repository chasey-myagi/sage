import {
  AgentSession,
  type HarnessTool,
  MemoryLogStore,
  passthroughView,
  type ViewContext,
} from "@sage/core";
import { fauxAdapter, renderLlmInput } from "@sage/core/testing";
import type { StreamChunk } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { defaultTokenCount } from "../../src/compiler/budget.ts";
import { indexChunks } from "../../src/compiler/chunks.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { expandTool } from "../../src/compiler/expand.ts";
import { subtaskTools } from "../../src/index.ts";

const MODEL = { provider: "anthropic", model: "claude-test" };
const SYSTEM = "You are sage.";
const BEGIN = 'Begin the subtask described in <claim id="subtask-query">.';

function user(text: string) {
  return {
    role: "user" as const,
    content: [{ type: "text" as const, text }],
  };
}

function toolTurn(id: string, name: string, inputJson: string): StreamChunk[] {
  return [
    { type: "block_start", index: 0, blockType: "tool_use", id, name },
    { type: "tool_input_delta", index: 0, partialJson: inputJson },
    { type: "block_end", index: 0 },
    {
      type: "usage",
      usage: {
        inputOther: 3,
        inputCacheRead: 1,
        inputCacheCreation: 0,
        output: 2,
      },
    },
    { type: "finish", kind: "tool_use" },
  ];
}

function stopTurn(text: string, output = 2): StreamChunk[] {
  return [
    { type: "block_start", index: 0, blockType: "text" },
    { type: "text_delta", index: 0, text },
    { type: "block_end", index: 0 },
    {
      type: "usage",
      usage: {
        inputOther: 3,
        inputCacheRead: 1,
        inputCacheCreation: 0,
        output,
      },
    },
    { type: "finish", kind: "stop" },
  ];
}

function parentView(ctx: ViewContext) {
  const base = passthroughView.view(ctx);
  if (base instanceof Promise) {
    throw new Error("passthrough view must be synchronous");
  }
  const seqs: number[] = [];
  for (const entry of ctx.path) {
    if (entry.kind === "message") seqs.push(entry.seq);
  }
  const chunks = indexChunks(
    [...base.messages],
    seqs,
    ctx.tools,
    defaultTokenCount,
  );
  const messages = [user("EPOCH-HEAD"), ...base.messages];
  const end = Math.max(0, messages.length - 1);
  return {
    epoch: 2,
    ...(ctx.turn === 1 ? { epochStart: { reason: "test-epoch" } } : {}),
    head: ctx.head,
    messages,
    cacheBreakpoints: [
      "head" as const,
      { afterMessage: end },
      { afterMessage: end },
    ],
    manifest: {
      epoch: 2,
      turn: ctx.turn,
      claims: [],
      chunks: chunks.map((chunk) => ({
        id: chunk.id,
        tier: "full" as const,
        reason: "frozen",
        tokens: chunk.tokens,
      })),
      anomalies: base.manifest.anomalies,
    },
  };
}

function compilerFor(budget: number, query: string) {
  return contextCompiler({
    budget,
    claims: [],
    subtask: { query },
    expectedTurns: 1,
  });
}

function harness(
  name: string,
  isReadOnly: boolean,
  execute?: () => Promise<{ content: string; isError: boolean }>,
): HarnessTool {
  return {
    def: { name, description: name, inputSchema: { type: "object" } },
    isReadOnly,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    async execute() {
      if (execute) return execute();
      return { content: "ok", isError: false };
    },
  };
}

async function spawnFrozen(query = "find the needle") {
  const store = new MemoryLogStore();
  const parentId = "parent";
  for (let index = 0; index < 11; index += 1) {
    store.append(parentId, {
      kind: "message",
      payload: user(`frozen-${index}`),
    });
  }
  const adapter = fauxAdapter([
    toolTurn(
      "call-1",
      "spawn_subtask",
      JSON.stringify({ query, budget: 1000, readOnly: true }),
    ),
    stopTurn("needle-found", 50),
    stopTurn("parent-done", 2),
  ]);
  const captured: {
    content: string;
    isError: boolean;
    details?: Record<string, unknown>;
  }[] = [];
  const tools: HarnessTool[] = [];
  const { spawn } = subtaskTools({
    store,
    adapters: { anthropic: adapter },
    tools,
    system: SYSTEM,
    makeCompiler: ({ budget, query: task }) => compilerFor(budget, task),
  });
  tools.push({
    ...spawn,
    async execute(input, ctx) {
      const result = await spawn.execute(input, ctx);
      captured.push(result);
      return result;
    },
  });
  const session = new AgentSession({
    store,
    sessionId: parentId,
    adapter,
    model: MODEL,
    system: SYSTEM,
    tools,
    viewProvider: { view: parentView },
  });
  const summary = await session.run("frozen-11");
  const result = captured[0];
  if (!result) throw new Error("spawn_subtask did not run");
  return { store, parentId, requests: adapter.requests, summary, result };
}

it("extends the parent request with the subtask query after the frozen prefix", async () => {
  const { requests } = await spawnFrozen();
  const parentReq = requests[0];
  const childReq = requests[1];
  if (!parentReq || !childReq) throw new Error("missing requests");
  expect(parentReq.messages).toHaveLength(13);
  expect(parentReq.messages[0]).toEqual(user("EPOCH-HEAD"));
  expect(parentReq.messages.slice(1)).toHaveLength(12);
  expect(childReq.head).toEqual(parentReq.head);
  expect(childReq.messages.slice(0, 13)).toEqual(parentReq.messages);
  expect(renderLlmInput(childReq, parentReq)).toBe(`system: <same>
tools: <same>
messages:
<last>
+ [user] ${BEGIN}
+ <claim id="subtask-query" tier="full">
+ find the needle
+ </claim>`);
});

it("stores the child on its own session and does not copy parent entries", async () => {
  const { store, parentId, result } = await spawnFrozen();
  const parentIds = new Set(store.all(parentId).map((entry) => entry.id));
  const childId = result.details?.sessionId;
  if (typeof childId !== "string") throw new Error("missing child session");
  const child = store.all(childId);
  expect(child[0]?.kind).toBe("subtask");
  expect(child[0]?.payload).toMatchObject({
    parentSessionId: parentId,
    query: "find the needle",
    budget: 1000,
    readOnly: true,
  });
  expect(child.every((entry) => !parentIds.has(entry.id))).toBe(true);
  expect(JSON.stringify(child)).not.toContain("frozen-0");
  expect(JSON.stringify(store.all(parentId)).split("frozen-0")).toHaveLength(2);
  expect(child.length).toBeLessThan(store.all(parentId).length);
});

it("keeps the parent tool list and denies writes in a read-only subtask", async () => {
  const store = new MemoryLogStore();
  let wrote = false;
  const adapter = fauxAdapter([
    toolTurn(
      "call-1",
      "spawn_subtask",
      JSON.stringify({ query: "inspect", budget: 500, readOnly: true }),
    ),
    toolTurn("w1", "write", "{}"),
    stopTurn("child-done"),
    stopTurn("parent-done"),
  ]);
  const tools: HarnessTool[] = [
    harness("write", false, async () => {
      wrote = true;
      return { content: "wrote", isError: false };
    }),
    harness("read", true),
  ];
  const { spawn } = subtaskTools({
    store,
    adapters: { anthropic: adapter },
    tools,
    system: SYSTEM,
    makeCompiler: ({ budget, query }) => compilerFor(budget, query),
  });
  tools.push(spawn);
  const session = new AgentSession({
    store,
    sessionId: "parent",
    adapter,
    model: MODEL,
    system: SYSTEM,
    tools,
  });
  await session.run("go");
  const parentReq = adapter.requests[0];
  const childReq = adapter.requests[1];
  if (!parentReq || !childReq) throw new Error("missing requests");
  expect(childReq.head).toEqual(parentReq.head);
  expect(childReq.head.tools.map((tool) => tool.name)).toEqual([
    "write",
    "read",
    "spawn_subtask",
  ]);
  const childId = store
    .all("parent")
    .filter((entry) => entry.kind === "message")
    .map((entry) => JSON.stringify(entry.payload))
    .join("\n");
  expect(childId).toContain("Subtask ");
  const sessions = ["parent"];
  const match = /Subtask ([0-9a-f-]+) /.exec(
    JSON.stringify(store.all("parent")),
  );
  const sid = match?.[1];
  if (!sid) throw new Error("missing child id");
  sessions.push(sid);
  const denied = store
    .all(sid)
    .flatMap((entry) => {
      if (entry.kind !== "message") return [];
      const payload = entry.payload as {
        content?: { type?: string; content?: string }[];
      };
      return payload.content ?? [];
    })
    .find((block) => block.type === "tool_result");
  expect(denied).toMatchObject({
    content: "Denied: read-only subtask",
    isError: true,
  });
  expect(wrote).toBe(false);
});

it("returns chunk pointers and expand reads the same original", async () => {
  const { store, parentId, summary, result } = await spawnFrozen();
  expect(summary.usage.output).toBe(4);
  expect(result.isError).toBe(false);
  const sessionId = result.details?.sessionId;
  expect(typeof sessionId).toBe("string");
  expect(result.details?.outcome).toBe("done");
  expect(result.details?.usage).toMatchObject({ output: 50 });
  expect(result.content).toBe(
    `Subtask ${sessionId} done. needle-found\nChunks: ${((result.details?.chunks as string[]) ?? []).join(", ")}`,
  );
  const chunks = result.details?.chunks;
  if (!Array.isArray(chunks)) throw new Error("missing chunks");
  expect(chunks.some((id) => id === `child:${sessionId}#seq:0`)).toBe(true);
  expect(
    chunks.some(
      (id) =>
        typeof id === "string" && id.startsWith(`child:${sessionId}#own:`),
    ),
  ).toBe(true);
  const expand = expandTool({ store, sessionId: parentId });
  const ctx = { sessionId: parentId, toolUseId: "ex" };
  expect(await expand.execute({ id: `child:${sessionId}#seq:0` }, ctx)).toEqual(
    { content: "frozen-0", isError: false },
  );
  const own = chunks.find(
    (id) => typeof id === "string" && id.includes("#own:"),
  );
  expect(await expand.execute({ id: own }, ctx)).toEqual({
    content: BEGIN,
    isError: false,
  });
});

it("reuses parent chunk ids and does not rebuild the epoch", async () => {
  const { store, parentId, result } = await spawnFrozen();
  const sessionId = result.details?.sessionId;
  if (typeof sessionId !== "string") throw new Error("missing child");
  const parentChunks = chunkIds(firstManifest(store, parentId));
  const childChunks = chunkIds(lastManifest(store, sessionId));
  for (const id of parentChunks) {
    expect(childChunks).toContain(id);
    expect(id.startsWith("seq:")).toBe(true);
  }
  expect(childChunks.some((id) => id.startsWith("own:"))).toBe(true);
  const decision = lastManifest(store, sessionId)?.payload as {
    epoch?: number;
    epochDecision?: { rebuild?: boolean; reason?: string };
  };
  expect(decision.epoch).toBe(2);
  expect(decision.epochDecision).toMatchObject({
    rebuild: false,
    reason: "subtask:no-rebuild",
  });
  const reasons = store
    .all(sessionId)
    .filter((entry) => entry.kind === "view_epoch")
    .map((entry) => (entry.payload as { reason?: string }).reason ?? "");
  expect(reasons.some((reason) => reason.startsWith("cost:"))).toBe(false);
});

it("allows a grandchild and rejects the next depth, and caps fan-out", async () => {
  const store = new MemoryLogStore();
  seed(store, "parent");
  const adapter = fauxAdapter(
    Array.from({ length: 4 }, () => stopTurn("leaf")),
  );
  const tools: HarnessTool[] = [];
  const { spawn } = subtaskTools({
    store,
    adapters: { anthropic: adapter },
    tools,
    system: SYSTEM,
    makeCompiler: ({ budget, query }) => compilerFor(budget, query),
  });
  const ctx = { toolUseId: "t" };
  const child = await spawn.execute(
    { query: "child", readOnly: false },
    { ...ctx, sessionId: "parent" },
  );
  expect(child.isError).toBe(false);
  const childId = child.details?.sessionId;
  if (typeof childId !== "string") throw new Error("missing child");
  const grand = await spawn.execute(
    { query: "grand", readOnly: false },
    { ...ctx, sessionId: childId },
  );
  expect(grand.isError).toBe(false);
  const grandId = grand.details?.sessionId;
  if (typeof grandId !== "string") throw new Error("missing grandchild");
  const denied = await spawn.execute(
    { query: "too deep", readOnly: false },
    { ...ctx, sessionId: grandId },
  );
  expect(denied.isError).toBe(true);
  expect(denied.content).toContain("maxDepth=2");

  const capped = new MemoryLogStore();
  seed(capped, "parent");
  const capAdapter = fauxAdapter(
    Array.from({ length: 8 }, () => stopTurn("ok")),
  );
  const capTools: HarnessTool[] = [];
  const cap = subtaskTools({
    store: capped,
    adapters: { anthropic: capAdapter },
    tools: capTools,
    system: SYSTEM,
    makeCompiler: ({ budget, query }) => compilerFor(budget, query),
  });
  for (let index = 0; index < 8; index += 1) {
    const result = await cap.spawn.execute(
      { query: `q${index}` },
      { sessionId: "parent", toolUseId: `c${index}` },
    );
    expect(result.isError).toBe(false);
  }
  const ninth = await cap.spawn.execute(
    { query: "overflow" },
    { sessionId: "parent", toolUseId: "c9" },
  );
  expect(ninth.isError).toBe(true);
  expect(ninth.content).toContain("maxSubtasks=8");
});

function seed(store: MemoryLogStore, sessionId: string) {
  store.append(sessionId, { kind: "message", payload: user("seed") });
  store.append(sessionId, {
    kind: "request_header",
    payload: {
      provider: "anthropic",
      model: "claude-test",
      systemPromptHash: "h",
      toolsHash: "t",
      sampling: {},
      adapterDefaults: {},
      system: SYSTEM,
    },
  });
  store.append(sessionId, {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 1,
      claims: [],
      chunks: [{ id: "seq:0", tier: "full", reason: "kept", tokens: 1 }],
      anomalies: [],
    },
  });
}

function firstManifest(store: MemoryLogStore, sessionId: string) {
  return store.all(sessionId).find((entry) => entry.kind === "manifest");
}

function lastManifest(store: MemoryLogStore, sessionId: string) {
  const all = store.all(sessionId);
  for (let index = all.length - 1; index >= 0; index -= 1) {
    if (all[index]?.kind === "manifest") return all[index];
  }
  return undefined;
}

function chunkIds(entry: { payload: unknown } | undefined): string[] {
  const payload = entry?.payload as { chunks?: { id?: string }[] } | undefined;
  return (payload?.chunks ?? []).flatMap((chunk) =>
    typeof chunk.id === "string" ? [chunk.id] : [],
  );
}
