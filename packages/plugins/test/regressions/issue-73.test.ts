import {
  isRecord,
  MemoryLogStore,
  runHealth,
  type ViewContext,
} from "@sage/core";
import { renderLlmInput } from "@sage/core/testing";
import type { Message, Request } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { expandTool } from "../../src/compiler/expand.ts";
import { modelScorer } from "../../src/scorer/model.ts";
import type { Scorer } from "../../src/scorer/types.ts";

const MODEL = { provider: "anthropic", model: "claude-sonnet-5" };
const HEAD = { system: "You are sage.", tools: [] as const };
const PREFIX = "system: <same>\ntools: <same>\nmessages:\n<last>\n";
const BODY = `ENTRY73_${"Q".repeat(400)}`;

const readTool = {
  def: {
    name: "read",
    description: "read",
    inputSchema: { type: "object" },
  },
  isReadOnly: true,
  isDestructive: false,
  isOpenWorld: false,
  accesses(input: unknown) {
    if (!isRecord(input) || typeof input.path !== "string") return [];
    if (input.path.length === 0) return [];
    return [{ key: input.path, mode: "read" as const }];
  },
  execute() {
    return Promise.resolve({ content: "", isError: false });
  },
};

it("stubs a new block of at least 300 tokens on entry and keeps the prefix", async () => {
  const shadowCalls: string[][] = [];
  const scorer: Scorer = {
    name: "fake",
    async score(input) {
      return {
        decisions: input.candidates
          .filter((chunk) => chunk.tokens >= 300)
          .map((chunk) => ({ id: chunk.id, tier: "none" as const })),
      };
    },
  };
  const shadow: Scorer = {
    name: "shadow",
    async score(input) {
      shadowCalls.push(input.candidates.map((chunk) => chunk.id));
      return { decisions: [] };
    },
  };
  const compiler = contextCompiler({
    budget: 1_000_000,
    claims: [],
    keepRecent: 6,
    expectedTurns: 20,
    scorer,
    scorerApply: "entry",
    minScoreTokens: 300,
    shadowScorer: shadow,
    count: (text) => text.length,
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("task") });
  const views = [];
  for (let turn = 1; turn <= 8; turn += 1) {
    if (turn === 2) appendTool(store, "big", "big.ts", BODY);
    if (turn >= 3 && turn <= 7) {
      store.append("s", { kind: "message", payload: user(`n${turn}`) });
    }
    if (turn === 8) {
      const id = chunkOf(views[1], "big")?.id;
      if (!id) throw new Error("missing entry chunk");
      const retrieved = await expandTool({ store, sessionId: "s" }).execute(
        { id },
        { sessionId: "s", toolUseId: "ex" },
      );
      expect(retrieved).toEqual({ content: BODY, isError: false });
      appendTool(store, "ex", id, retrieved.content);
    }
    const view = await compiler.view(context(store, turn));
    const previous = views.at(-1);
    if (previous) {
      const rendered = renderLlmInput(
        requestOf(view.messages, view.cacheBreakpoints),
        requestOf(previous.messages, previous.cacheBreakpoints),
      );
      expect(rendered.startsWith(PREFIX), rendered.slice(0, 200)).toBe(true);
      expect(rendered.includes("\n+ "), `turn ${turn}`).toBe(true);
    }
    views.push(view);
    store.append("s", { kind: "manifest", payload: view.manifest });
  }

  const entered = chunkOf(views[1], "big");
  expect(entered, JSON.stringify(entered)).toMatchObject({
    tier: "none",
    reason: "scorer:entry:none",
  });
  expect(resultOf(views[1]?.messages ?? [], "big")).not.toContain("ENTRY73");
  expect(resultOf(views[1]?.messages ?? [], "big")).toContain("omitted");
  for (const view of views.slice(2)) {
    expect(chunkOf(view, "big")?.tier).toBe("none");
    expect(resultOf(view.messages, "big")).not.toContain("ENTRY73");
  }
  const last = views[7];
  const before = views[6];
  expect(expandsOf(last)).toBe(expandsOf(before) + 1);
  expect(resultOf(last?.messages ?? [], "ex")).toBe(BODY);
  expect(shadowCalls).toEqual([]);
  expect(runHealth(store.path("s")).entryScored).toEqual({
    calls: 1,
    none: 1,
    summary: 0,
    full: 0,
    summaryRejected: 0,
    failedRequests: 0,
    usage: {
      input: 0,
      cacheRead: 0,
      cacheCreation: 0,
      output: 0,
      total: 0,
    },
  });
});

it("does not score a new block under 300 tokens and leaves it recent", async () => {
  const seen: string[][] = [];
  const scorer: Scorer = {
    name: "fake",
    async score(input) {
      seen.push(input.candidates.map((chunk) => chunk.id));
      return {
        decisions: input.candidates.map((chunk) => ({
          id: chunk.id,
          tier: "none" as const,
        })),
      };
    },
  };
  const compiler = contextCompiler({
    budget: 1_000_000,
    claims: [],
    keepRecent: 6,
    scorer,
    scorerApply: "entry",
    count: (text) => text.length,
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("task") });
  appendTools(store, [
    { id: "big", path: "big.ts", output: BODY },
    { id: "small", path: "s.ts", output: "ok" },
  ]);
  const view = await compiler.view(context(store, 1));
  expect(seen).toHaveLength(1);
  expect(seen[0]?.some((id) => id.endsWith("#small"))).toBe(false);
  expect(seen[0]?.some((id) => id.endsWith("#big"))).toBe(true);
  expect(chunkOf(view, "small")).toMatchObject({
    tier: "full",
    reason: "recent",
  });
  expect(resultOf(view.messages, "small")).toBe("ok");
});

it("keeps the block full when the scorer throws and still counts the call", async () => {
  const view = await compileBig({
    name: "fake",
    async score() {
      throw new Error("boom");
    },
  });
  expect(chunkOf(view.view, "big")).toMatchObject({
    tier: "full",
    reason: "recent",
  });
  expect(resultOf(view.view.messages, "big")).toBe(BODY);
  expect(runHealth(view.store.path("s")).entryScored.calls).toBe(1);
});

it("keeps an entry summary on the next turn and does not score again", async () => {
  let calls = 0;
  const note = "src/big.ts: keptListing (L1-40)";
  const scorer: Scorer = {
    name: "fake",
    async score(input) {
      calls += 1;
      return {
        decisions: input.candidates.map((chunk) => ({
          id: chunk.id,
          tier: "summary" as const,
          summary: `${note}${"x".repeat(250)}`,
        })),
      };
    },
  };
  const compiler = contextCompiler({
    budget: 1_000_000,
    claims: [],
    keepRecent: 6,
    scorer,
    scorerApply: "entry",
    count: (text) => text.length,
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("task") });
  appendTool(store, "big", "big.ts", BODY);
  const first = await compiler.view(context(store, 1));
  const shown = resultOf(first.messages, "big");
  expect(calls).toBe(1);
  expect(chunkOf(first, "big")).toMatchObject({
    tier: "summary",
    reason: "scorer:entry:summary",
  });
  expect(shown).toContain(
    `summary (model notes, not evidence): ${note}${"x".repeat(200 - note.length)}`,
  );
  expect(shown).not.toContain("x".repeat(201));
  store.append("s", { kind: "manifest", payload: first.manifest });
  store.append("s", { kind: "message", payload: user("next") });
  const second = await compiler.view(context(store, 2));
  expect(calls).toBe(1);
  expect(resultOf(second.messages, "big")).toBe(shown);
  expect(
    renderLlmInput(
      requestOf(second.messages, second.cacheBreakpoints),
      requestOf(first.messages, first.cacheBreakpoints),
    ).startsWith(PREFIX),
  ).toBe(true);
});

it("stays full when a summary decision has no note", async () => {
  const view = await compileBig({
    name: "fake",
    async score(input) {
      return {
        decisions: input.candidates.map((chunk) => ({
          id: chunk.id,
          tier: "summary" as const,
        })),
      };
    },
  });
  expect(chunkOf(view.view, "big")).toMatchObject({
    tier: "full",
    reason: "recent",
  });
  expect(resultOf(view.view.messages, "big")).toBe(BODY);
  expect(runHealth(view.store.path("s")).entryScored.calls).toBe(1);
  expect(runHealth(view.store.path("s")).entryScored.summary).toBe(0);
});

it("keeps the text and records scorer:entry:full when the scorer chooses full", async () => {
  const view = await compileBig({
    name: "fake",
    async score(input) {
      return {
        usage: {
          inputOther: 2,
          inputCacheRead: 0,
          inputCacheCreation: 0,
          output: 1,
        },
        decisions: input.candidates.map((chunk) => ({
          id: chunk.id,
          tier: "full" as const,
        })),
      };
    },
  });
  expect(chunkOf(view.view, "big")).toMatchObject({
    tier: "full",
    reason: "scorer:entry:full",
  });
  expect(resultOf(view.view.messages, "big")).toBe(BODY);
  expect(runHealth(view.store.path("s")).entryScored).toMatchObject({
    calls: 1,
    none: 0,
    summary: 0,
    full: 1,
    usage: { input: 2, cacheRead: 0, cacheCreation: 0, output: 1, total: 3 },
  });
});

it("keeps the block full when the scorer returns fallback and still counts the call", async () => {
  const view = await compileBig({
    name: "fake",
    async score() {
      return {
        decisions: [{ id: "seq:2#big", tier: "none" as const }],
        fallback: "invalid-json",
      };
    },
  });
  expect(chunkOf(view.view, "big")).toMatchObject({
    tier: "full",
    reason: "recent",
  });
  expect(resultOf(view.view.messages, "big")).toBe(BODY);
  expect(runHealth(view.store.path("s")).entryScored.calls).toBe(1);
  expect(runHealth(view.store.path("s")).entryScored.none).toBe(0);
});

it("keeps the block full when the scorer omits its id and still counts the call", async () => {
  const view = await compileBig({
    name: "fake",
    async score() {
      return { decisions: [{ id: "missing", tier: "none" as const }] };
    },
  });
  expect(chunkOf(view.view, "big")).toMatchObject({
    tier: "full",
    reason: "recent",
  });
  expect(resultOf(view.view.messages, "big")).toBe(BODY);
  expect(runHealth(view.store.path("s")).entryScored.calls).toBe(1);
});

it("records zero entry calls when the model scorer has no adapter, as rebuild does", async () => {
  const view = await compileBig(modelScorer({ adapters: {}, model: MODEL }));
  expect(chunkOf(view.view, "big")).toMatchObject({
    tier: "full",
    reason: "recent",
  });
  expect(resultOf(view.view.messages, "big")).toBe(BODY);
  expect(view.view.manifest.entryScored).toEqual({
    calls: 0,
    none: 0,
    summary: 0,
    full: 0,
    summaryRejected: 0,
  });
  expect(runHealth(view.store.path("s")).entryScored.calls).toBe(0);
});

async function compileBig(scorer: Scorer): Promise<{
  store: MemoryLogStore;
  view: Awaited<ReturnType<ReturnType<typeof contextCompiler>["view"]>>;
}> {
  const compiler = contextCompiler({
    budget: 1_000_000,
    claims: [],
    keepRecent: 6,
    scorer,
    scorerApply: "entry",
    count: (text) => text.length,
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("task") });
  appendTool(store, "big", "big.ts", BODY);
  const view = await compiler.view(context(store, 1));
  store.append("s", { kind: "manifest", payload: view.manifest });
  return { store, view };
}

function context(store: MemoryLogStore, turn: number): ViewContext {
  return {
    sessionId: "s",
    turn,
    path: store.path("s"),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: 1_000_000,
      pricing: null,
      adapterDefaults: {},
    },
    head: { system: HEAD.system, tools: [readTool.def] },
    tools: [readTool],
    depth: 0,
  };
}

function user(text: string): Message {
  return { role: "user", content: [{ type: "text", text }] };
}

function appendTool(
  store: MemoryLogStore,
  id: string,
  filePath: string,
  output: string,
): void {
  appendTools(store, [{ id, path: filePath, output }]);
}

function appendTools(
  store: MemoryLogStore,
  tools: { id: string; path: string; output: string }[],
): void {
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: tools.map((tool) => ({
        type: "tool_use" as const,
        id: tool.id,
        name: tool.id === "ex" ? "expand" : "read",
        input: tool.id === "ex" ? { id: tool.path } : { path: tool.path },
        rawInput: JSON.stringify(
          tool.id === "ex" ? { id: tool.path } : { path: tool.path },
        ),
      })),
    },
  });
  store.append("s", {
    kind: "message",
    payload: {
      role: "user",
      content: tools.map((tool) => ({
        type: "tool_result" as const,
        toolUseId: tool.id,
        content: tool.output,
        isError: false,
      })),
    },
  });
}

function chunkOf(
  view:
    | {
        manifest: {
          chunks: { id: string; tier: string; reason: string }[];
        };
      }
    | undefined,
  toolUseId: string,
): { id: string; tier: string; reason: string } | undefined {
  return view?.manifest.chunks.find((chunk) =>
    chunk.id.endsWith(`#${toolUseId}`),
  );
}

function resultOf(
  messages: {
    content: { type: string; toolUseId?: string; content?: string }[];
  }[],
  toolUseId: string,
): string {
  for (const message of messages) {
    for (const block of message.content) {
      if (block.type === "tool_result" && block.toolUseId === toolUseId) {
        return block.content ?? "";
      }
    }
  }
  return "";
}

function expandsOf(
  view: { manifest: { budget?: number | { expands?: number } } } | undefined,
): number {
  const budget = view?.manifest.budget;
  if (typeof budget !== "object" || budget === undefined) return 0;
  return budget.expands ?? 0;
}

function requestOf(
  messages: Message[],
  cacheBreakpoints: Request["cacheBreakpoints"],
): Request {
  return {
    model: MODEL,
    head: { system: HEAD.system, tools: [readTool.def] },
    messages,
    cacheBreakpoints,
    sampling: {},
  };
}
