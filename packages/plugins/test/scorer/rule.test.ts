import {
  appendRequestEntries,
  type CompiledView,
  type HarnessTool,
  MemoryLogStore,
  type ViewContext,
} from "@sage/core";
import { pathlessTool, pathTool } from "@sage/core/testing";
import { expect, it } from "vitest";
import { rebuildFromLog } from "../../src/compiler/rebuild.ts";
import { contextCompiler, ruleScorer } from "../../src/index.ts";
import type { Scorer } from "../../src/scorer/types.ts";

const MODEL = { provider: "anthropic", model: "claude-test" };
const BODY = "R".repeat(400);

const TOOLS: HarnessTool[] = [
  pathTool("read", "read"),
  pathTool("edit", "write"),
  pathTool("write", "write"),
  pathlessTool("bash"),
  pathlessTool("grep"),
  pathlessTool("find"),
  pathlessTool("update_ledger"),
];

type Call = {
  id: string;
  name: string;
  input: unknown;
  isError?: boolean;
  /** The tool_result text; defaults to `BODY`. */
  result?: string;
};

/**
 * One call per rule category. Echo tools carry the one-line results the
 * real tools return; the others carry a result large enough to score.
 */
const CALLS: Call[] = [
  { id: "edit", name: "edit", input: { path: "e.ts" }, result: "Edited e.ts" },
  { id: "write", name: "write", input: { path: "w.ts" }, result: "Wrote w.ts" },
  {
    id: "ledger",
    name: "update_ledger",
    input: { goal: "fix" },
    result: "Ledger updated.",
  },
  { id: "read", name: "read", input: { path: "r.ts" } },
  { id: "grep", name: "grep", input: { pattern: "foo" } },
  { id: "cat", name: "bash", input: { command: "cd /repo && cat a.ts" } },
  { id: "test", name: "bash", input: { command: "pnpm test" } },
  {
    id: "failed",
    name: "bash",
    input: { command: "pnpm build" },
    isError: true,
  },
  { id: "mixed", name: "bash", input: { command: "cat a.ts && make" } },
  {
    id: "piped",
    name: "bash",
    input: { command: "pnpm test 2>&1 | tail -50" },
  },
  { id: "find", name: "find", input: { glob: "**/*.ts" } },
];

const KEPT_IDS = [
  "edit",
  "write",
  "ledger",
  "read",
  "grep",
  "cat",
  "failed",
  "mixed",
  "piped",
  "find",
];

it("summarizes only a successful runner and keeps every other tool kind in entry mode", async () => {
  const store = seeded(CALLS);
  const scored = await compileEntry(store, ruleScorer());
  const baseline = await compileEntry(store, undefined);

  expect(row(scored, "test")).toMatchObject({
    tier: "summary",
    reason: "scorer:entry:summary;scorer:rule:bash-runner",
  });
  for (const kept of KEPT_IDS) {
    expect(row(scored, kept)).toEqual(row(baseline, kept));
    expect(result(scored, kept)).toBe(result(baseline, kept));
  }
  expect(result(scored, "edit")).toBe("Edited e.ts");
  expect(scored.manifest.entryScored).toEqual({
    calls: 0,
    none: 0,
    summary: 1,
    full: 0,
    summaryRejected: 0,
  });
});

it("shows a summarized runner as the stub line alone", async () => {
  const store = seeded(CALLS);
  const view = await compileEntry(store, ruleScorer());
  const id = row(view, "test")?.id ?? "";
  expect(result(view, "test")).toBe(
    `[#${id} · bash · {"command":"pnpm test"} · 0.4k tokens · omitted · expand("${id}") to retrieve]`,
  );
});

it("keeps the stub-only summary byte-identical on the next turn and in the Log", async () => {
  const store = seeded(CALLS);
  const compiler = entryCompiler(ruleScorer());
  const first = await compiler.view(context(store, 1));
  commit(store, first);
  store.append("s", {
    kind: "message",
    payload: { role: "assistant", content: [{ type: "text", text: "ok" }] },
  });
  store.append("s", {
    kind: "message",
    payload: { role: "user", content: [{ type: "text", text: "go on" }] },
  });
  const second = await compiler.view(context(store, 2));
  commit(store, second);

  expect(row(second, "test")?.tier).toBe("summary");
  expect(result(second, "test")).toBe(result(first, "test"));
  const rebuilt = rebuildFromLog(store.path("s"), 1);
  expect(resultIn(rebuilt.messages, "test")).toBe(result(first, "test"));
});

it("lets options override the tier of each category", async () => {
  const store = seeded(CALLS);
  const baseline = await compileEntry(store, undefined);
  const view = await compileEntry(
    store,
    ruleScorer({ tiers: { "bash-runner": "keep", "bash-mixed": "none" } }),
  );
  expect(row(view, "test")).toEqual(row(baseline, "test"));
  expect(row(view, "mixed")).toMatchObject({
    tier: "none",
    reason: "scorer:entry:none;scorer:rule:bash-mixed",
  });
});

it("applies the same tiers when an epoch rebuilds", async () => {
  const store = seeded([
    {
      id: "edit",
      name: "edit",
      input: { path: "e.ts" },
      result: "Edited e.ts",
    },
    { id: "test", name: "bash", input: { command: "pnpm test" } },
    { id: "read", name: "read", input: { path: "r.ts" } },
  ]);
  const view = await contextCompiler({
    budget: 1_000_000,
    claims: [],
    keepRecent: 1,
    scorer: ruleScorer(),
    count: (text) => text.length,
  }).view(context(store, 1, 1));

  expect(view.manifest.epochDecision?.rebuild).toBe(true);
  expect(row(view, "edit")).toMatchObject({ tier: "full", reason: "kept" });
  expect(result(view, "edit")).toBe("Edited e.ts");
  expect(row(view, "test")).toMatchObject({
    tier: "summary",
    reason: "scorer:rule:bash-runner",
  });
  expect(result(view, "test")).not.toContain("\n");
  expect(row(view, "read")).toMatchObject({ tier: "full", reason: "recent" });
  expect(view.manifest).not.toHaveProperty("scorer");
});

function seeded(calls: readonly Call[]): MemoryLogStore {
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "message",
    payload: { role: "user", content: [{ type: "text", text: "fix it" }] },
  });
  for (const call of calls) {
    store.append("s", {
      kind: "message",
      payload: {
        role: "assistant",
        content: [
          {
            type: "tool_use",
            id: call.id,
            name: call.name,
            input: call.input,
            rawInput: JSON.stringify(call.input),
          },
        ],
      },
    });
    store.append("s", {
      kind: "message",
      payload: {
        role: "user",
        content: [
          {
            type: "tool_result",
            toolUseId: call.id,
            content: call.result ?? BODY,
            isError: call.isError ?? false,
          },
        ],
      },
    });
  }
  return store;
}

function entryCompiler(scorer: Scorer | undefined) {
  return contextCompiler({
    budget: 1_000_000,
    claims: [],
    ...(scorer ? { scorer } : {}),
    scorerApply: "entry",
    count: (text) => text.length,
  });
}

function compileEntry(
  store: MemoryLogStore,
  scorer: Scorer | undefined,
): Promise<CompiledView> {
  return entryCompiler(scorer).view(context(store, 1));
}

function commit(store: MemoryLogStore, view: CompiledView): void {
  appendRequestEntries(store, "s", {
    view,
    model: MODEL,
    sampling: {},
    prepared: preparedFor(1_000_000),
  });
}

function preparedFor(contextWindow: number) {
  return { model: MODEL, contextWindow, pricing: null, adapterDefaults: {} };
}

function context(
  store: MemoryLogStore,
  turn: number,
  contextWindow = 1_000_000,
): ViewContext {
  return {
    sessionId: "s",
    turn,
    path: store.path("s"),
    model: MODEL,
    prepared: preparedFor(contextWindow),
    head: { system: "You are sage.", tools: TOOLS.map((tool) => tool.def) },
    tools: TOOLS,
    depth: 0,
  };
}

function row(view: CompiledView, toolUseId: string) {
  return view.manifest.chunks.find((chunk) =>
    chunk.id.endsWith(`#${toolUseId}`),
  );
}

function result(view: CompiledView, toolUseId: string): string | undefined {
  return resultIn(view.messages, toolUseId);
}

function resultIn(
  messages: CompiledView["messages"],
  toolUseId: string,
): string | undefined {
  for (const message of messages) {
    for (const block of message.content) {
      if (block.type === "tool_result" && block.toolUseId === toolUseId) {
        return block.content;
      }
    }
  }
  return undefined;
}
