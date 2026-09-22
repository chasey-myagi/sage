import path from "node:path";
import { fileURLToPath } from "node:url";
import { type HarnessTool, JsonlLogStore, MemoryLogStore } from "@sage/core";
import { renderLlmInput } from "@sage/core/testing";
import type { Message, Request } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { frozenDecisions } from "../../src/compiler/freeze.ts";
import { replayOffline } from "../../src/replay/offline.ts";

const MODEL = { provider: "anthropic", model: "claude-sonnet-5" };
const HEAD = { system: "You are sage.", tools: [] as const };
const PRICING = {
  inputPerMTok: 2,
  outputPerMTok: 10,
  cacheWritePerMTok: 2.5,
  cacheReadPerMTok: 0.2,
  source: "https://platform.claude.com/docs/en/about-claude/pricing",
};
const PREFIX = "system: <same>\ntools: <same>\nmessages:\n<last>\n";

const fixtureDir = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "../replay/fixtures/issue-46",
);

it("accumulates chunk tiers inside one epoch and does not override the first", () => {
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 1,
      claims: [],
      chunks: [{ id: "a", tier: "full", reason: "recent" }],
      anomalies: [],
    },
  });
  store.append("s", {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 2,
      claims: [],
      chunks: [
        { id: "a", tier: "none", reason: "superseded_by:b" },
        { id: "b", tier: "full", reason: "recent" },
      ],
      anomalies: [],
    },
  });
  const frozen = frozenDecisions(store.path("s"), 0);
  expect(frozen?.get("a")).toEqual({
    id: "a",
    tier: "full",
    reason: "recent",
  });
  expect(frozen?.get("b")).toEqual({
    id: "b",
    tier: "full",
    reason: "recent",
  });
});

it("keeps each epoch's freeze on its own side of view_epoch", () => {
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 1,
      claims: [],
      chunks: [{ id: "a", tier: "full", reason: "kept" }],
      anomalies: [],
    },
  });
  store.append("s", {
    kind: "view_epoch",
    payload: { epoch: 1, reason: "cost:immediate" },
  });
  store.append("s", {
    kind: "manifest",
    payload: {
      epoch: 1,
      turn: 2,
      claims: [],
      chunks: [{ id: "a", tier: "none", reason: "superseded_by:b" }],
      anomalies: [],
    },
  });
  expect(frozenDecisions(store.path("s"), 0)?.get("a")?.tier).toBe("full");
  expect(frozenDecisions(store.path("s"), 1)?.get("a")?.tier).toBe("none");
});

it("extends the frozen prefix for 20 turns, including a covering edit", async () => {
  const tools = [pathTool("read", "read"), pathTool("edit", "write")];
  const compiler = contextCompiler({
    budget: 1_000_000,
    claims: [],
    keepRecent: 6,
    expectedTurns: 20,
    minSavingsRatio: 0.2,
    count: (text) => text.length,
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("task") });
  const views = [];
  for (let turn = 1; turn <= 20; turn += 1) {
    if (turn === 2)
      appendTool(store, "read", "readA", "a.ts", "A".repeat(2000));
    if (turn >= 3 && turn <= 8) {
      appendTool(store, "read", `f${turn}`, `f${turn}.ts`, "f");
    }
    if (turn === 9) {
      appendTool(store, "edit", "editA", "a.ts", "edited");
    }
    if (turn === 10) appendTool(store, "read", "readA2", "a.ts", "A2");
    if (turn >= 11) {
      store.append("s", { kind: "message", payload: user(`n${turn}`) });
    }
    const view = await compiler.view(context(store, turn, tools));
    const previous = views.at(-1);
    if (previous) {
      const rendered = renderLlmInput(
        requestOf(view.messages, view.cacheBreakpoints, tools),
        requestOf(previous.messages, previous.cacheBreakpoints, tools),
      );
      const opened = view.epoch !== previous.epoch;
      if (!opened) {
        expect(rendered.startsWith(PREFIX), rendered.slice(0, 160)).toBe(true);
        expect(rendered.includes("\n+ "), `turn ${turn}`).toBe(true);
      }
    }
    views.push(view);
    if (view.epochStart) {
      store.append("s", {
        kind: "view_epoch",
        payload: { epoch: view.epoch, reason: view.epochStart.reason },
      });
    }
    store.append("s", { kind: "manifest", payload: view.manifest });
  }

  expect(views).toHaveLength(20);
  expect(tierFlips(views)).toBe(0);
  const reopenAt = views.findIndex((view, index) => {
    const previous = views[index - 1];
    return previous !== undefined && view.epoch !== previous.epoch;
  });
  expect(reopenAt).toBeGreaterThan(0);
  const reopen = views[reopenAt];
  const followed = views[reopenAt + 1];
  if (!reopen || !followed) throw new Error("missing rebuild turn");
  expect(reopen.epochStart).toBeDefined();
  expect(chunkTier(reopen, "readA")).toMatchObject({ tier: "none" });
  expect(chunkTier(reopen, "readA")?.reason.startsWith("superseded_by:")).toBe(
    true,
  );
  expect(followed.epoch).toBe(reopen.epoch);
  expect(followed.epochStart).toBeUndefined();
  expect(chunkTier(followed, "readA")).toEqual(chunkTier(reopen, "readA"));
  const budget = reopen.manifest.budget;
  if (typeof budget !== "object" || budget === undefined) {
    throw new Error("expected a budget report");
  }
  expect(budget.frozenTokens).toBeGreaterThan(0);
  expect(budget.staleOmittable).toBeGreaterThan(0);
  expect(budget.frozenTokens).toBeGreaterThanOrEqual(budget.staleOmittable);
});

it("replays the long deepseek trace with a frozen prefix and a real rebuild", async () => {
  const store = new JsonlLogStore({ dir: fixtureDir });
  const diffs = await replayOffline(
    store,
    "long-deepseek-flash",
    { budget: 40_000, keepRecent: 6, expectedTurns: 20 },
    {
      tools: [
        pathTool("read", "read"),
        pathTool("edit", "write"),
        pathTool("write", "write"),
        pathTool("ls", "read"),
        pathTool("bash", "read"),
      ],
      system: "You are sage.",
    },
  );
  const rows = diffs.map((diff) => ({
    turn: diff.turn,
    epoch: diff.epoch,
    saved: diff.saved,
    oldTokens: diff.oldTokens,
    newTokens: diff.newTokens,
    rebuild: diff.rebuild,
    decisionReason: diff.decisionReason,
  }));
  const detail = JSON.stringify(rows);
  expect(tierFlips(diffs), detail).toBe(0);
  expect(
    diffs.findIndex((diff) => diff.newTokens < diff.oldTokens),
    detail,
  ).toBe(7);
  for (const diff of diffs.slice(7)) {
    expect(diff.newTokens, `turn ${diff.turn} ${detail}`).toBeLessThan(
      diff.oldTokens,
    );
  }
  expect(
    diffs.find((diff) => diff.turn === 3)?.decisionReason ?? "",
    detail,
  ).not.toContain("context-window-guard");
  expect(
    diffs
      .filter((diff) => diff.decisionReason.includes("context-window-guard"))
      .map((diff) => diff.turn),
    detail,
  ).toEqual([4]);
  expect(
    diffs.some(
      (diff) =>
        !diff.rebuild &&
        diff.newTokens < diff.oldTokens &&
        diff.decisionReason.includes("payback:"),
    ),
    detail,
  ).toBe(true);
});

function tierFlips(
  views: readonly {
    epoch: number;
    manifest?: { chunks: { id: string; tier: string }[] };
    chunkTiers?: { id: string; tier: string }[];
  }[],
): number {
  const seen = new Map<string, string>();
  let flips = 0;
  for (const view of views) {
    const chunks =
      view.chunkTiers ??
      view.manifest?.chunks.map((chunk) => ({
        id: chunk.id,
        tier: chunk.tier,
      })) ??
      [];
    for (const chunk of chunks) {
      const key = `${view.epoch}\0${chunk.id}`;
      const prev = seen.get(key);
      if (prev !== undefined && prev !== chunk.tier) flips += 1;
      seen.set(key, chunk.tier);
    }
  }
  return flips;
}

function chunkTier(
  view: {
    manifest: { chunks: { id: string; tier: string; reason: string }[] };
  },
  toolUseId: string,
): { id: string; tier: string; reason: string } | undefined {
  return view.manifest.chunks.find((chunk) =>
    chunk.id.endsWith(`#${toolUseId}`),
  );
}

function context(store: MemoryLogStore, turn: number, tools: HarnessTool[]) {
  return {
    sessionId: "s",
    turn,
    path: store.path("s"),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: 1_000_000,
      pricing: PRICING,
      adapterDefaults: {},
    },
    head: { system: HEAD.system, tools: tools.map((tool) => tool.def) },
    tools,
    depth: 0,
  };
}

function user(text: string): Message {
  return { role: "user", content: [{ type: "text", text }] };
}

function appendTool(
  store: MemoryLogStore,
  name: string,
  id: string,
  filePath: string,
  output: string,
): void {
  const input = { path: filePath };
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id,
          name,
          input,
          rawInput: JSON.stringify(input),
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
          toolUseId: id,
          content: output,
          isError: false,
        },
      ],
    },
  });
}

function requestOf(
  messages: Message[],
  cacheBreakpoints: Request["cacheBreakpoints"],
  tools: HarnessTool[],
): Request {
  return {
    model: MODEL,
    head: { system: HEAD.system, tools: tools.map((tool) => tool.def) },
    messages,
    cacheBreakpoints,
    sampling: {},
  };
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
