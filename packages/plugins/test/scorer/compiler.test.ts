import { MemoryLogStore, type ViewContext } from "@sage/core";
import type {
  ProviderAdapter,
  Request,
  StreamChunk,
  Usage,
} from "@sage/provider/contract";
import { expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { rebuildFromLog } from "../../src/compiler/rebuild.ts";
import { modelScorer } from "../../src/scorer/model.ts";
import type { Scorer } from "../../src/scorer/types.ts";

const MODEL = { provider: "anthropic", model: "claude-test" };
const HEAD = {
  system: "You are sage.",
  tools: [
    {
      name: "read",
      description: "read a file",
      inputSchema: { type: "object" },
    },
  ],
};

const USAGE: Usage = {
  inputOther: 4,
  inputCacheRead: 9,
  inputCacheCreation: 0,
  output: 3,
};

function toolMessage(id: string, path: string) {
  return {
    role: "assistant" as const,
    content: [
      {
        type: "tool_use" as const,
        id,
        name: "read",
        input: { path },
        rawInput: JSON.stringify({ path }),
      },
    ],
  };
}

function resultMessage(id: string, content: string) {
  return {
    role: "user" as const,
    content: [
      {
        type: "tool_result" as const,
        toolUseId: id,
        content,
        isError: false,
      },
    ],
  };
}

function seed(store: MemoryLogStore, body = "AAA"): void {
  store.append("s", {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "find the bug" }],
    },
  });
  store.append("s", { kind: "message", payload: toolMessage("t1", "a.ts") });
  store.append("s", { kind: "message", payload: resultMessage("t1", body) });
  store.append("s", { kind: "message", payload: toolMessage("t2", "b.ts") });
  store.append("s", { kind: "message", payload: resultMessage("t2", "BBB") });
  store.append("s", { kind: "message", payload: toolMessage("t3", "c.ts") });
  store.append("s", { kind: "message", payload: resultMessage("t3", "CCC") });
}

function context(
  store: MemoryLogStore,
  turn: number,
  contextWindow: number,
): ViewContext {
  return {
    sessionId: "s",
    turn,
    path: store.path("s"),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow,
      pricing: null,
      adapterDefaults: {},
    },
    head: {
      system: HEAD.system,
      tools: HEAD.tools.map((tool) => ({ ...tool })),
    },
    tools: [
      {
        def: HEAD.tools[0] ?? {
          name: "read",
          description: "read a file",
          inputSchema: { type: "object" },
        },
        isReadOnly: true,
        isDestructive: false,
        isOpenWorld: false,
        accesses(input: unknown) {
          if (
            typeof input !== "object" ||
            input === null ||
            !("path" in input)
          ) {
            return [];
          }
          const key = (input as { path?: unknown }).path;
          return typeof key === "string"
            ? [{ key, mode: "read" as const }]
            : [];
        },
        async execute() {
          return { content: "", isError: false };
        },
      },
    ],
  };
}

function resultContent(
  messages: {
    content: { type: string; toolUseId?: string; content?: string }[];
  }[],
  id: string,
): string | undefined {
  for (const message of messages) {
    for (const block of message.content) {
      if (block.type === "tool_result" && block.toolUseId === id)
        return block.content;
    }
  }
  return undefined;
}

it("does not call the scorer when the epoch stays closed", async () => {
  let calls = 0;
  const scorer: Scorer = {
    name: "model",
    async score() {
      calls += 1;
      return { decisions: [] };
    },
  };
  const store = new MemoryLogStore();
  seed(store);
  const view = await contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
    scorer,
  }).view(context(store, 1, 1_000_000));
  expect(view.manifest.epochDecision?.rebuild).toBe(false);
  expect(calls).toBe(0);
});

it("calls the scorer once when the epoch rebuilds and keeps recent over a none score", async () => {
  let calls = 0;
  const scorer: Scorer = {
    name: "model",
    async score() {
      calls += 1;
      return {
        decisions: [
          { id: "seq:1#t1", tier: "summary", summary: "file a" },
          { id: "seq:3#t2", tier: "none" },
          { id: "seq:5#t3", tier: "none" },
        ],
      };
    },
  };
  const store = new MemoryLogStore();
  seed(store);
  const view = await contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
    scorer,
  }).view(context(store, 1, 1));

  expect(view.manifest.epochDecision?.rebuild).toBe(true);
  expect(calls).toBe(1);
  expect(view.manifest.chunks).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        id: "seq:1#t1",
        tier: "summary",
        reason: "scorer:model",
        summary: "file a",
      }),
      expect.objectContaining({
        id: "seq:3#t2",
        tier: "none",
        reason: "scorer:model",
      }),
      expect.objectContaining({
        id: "seq:5#t3",
        tier: "full",
        reason: "recent",
      }),
    ]),
  );
  expect(resultContent(view.messages, "t1")).toBe(
    [
      '[#seq:1#t1 · read · {"path":"a.ts"} · 0.0k tokens · omitted · expand("seq:1#t1") to retrieve]',
      "summary (model notes, not evidence): file a",
    ].join("\n"),
  );
  expect(resultContent(view.messages, "t2")).toBe(
    '[#seq:3#t2 · read · {"path":"b.ts"} · 0.0k tokens · omitted · expand("seq:3#t2") to retrieve]',
  );
  expect(resultContent(view.messages, "t3")).toBe("CCC");
  expect(view.manifest.epochDecision?.reason).not.toContain("scorer:fallback");
});

it("leaves kept reasons unchanged when the default rule scorer rebuilds", async () => {
  const store = new MemoryLogStore();
  seed(store);
  const view = await contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
  }).view(context(store, 1, 1));
  expect(view.manifest.epochDecision?.rebuild).toBe(true);
  const tools = view.manifest.chunks.filter((chunk) => chunk.id.includes("#"));
  expect(tools.map((chunk) => chunk.reason)).toEqual([
    "kept",
    "kept",
    "recent",
  ]);
  expect(tools.map((chunk) => chunk.tier)).toEqual(["full", "full", "full"]);
  expect(view.manifest).not.toHaveProperty("scorer");
  expect(resultContent(view.messages, "t1")).toBe("AAA");
});

function scripted(
  chunks: StreamChunk[],
): ProviderAdapter & { requests: Request[] } {
  const requests: Request[] = [];
  return {
    api: "anthropic-messages",
    requests,
    prepare(model) {
      return {
        model,
        contextWindow: 8000,
        pricing: null,
        adapterDefaults: {},
      };
    },
    stream(request) {
      requests.push(structuredClone(request));
      return (async function* () {
        for (const chunk of chunks) yield chunk;
      })();
    },
  };
}

it("records model usage and renders the summary from the score JSON", async () => {
  const store = new MemoryLogStore();
  seed(store);
  const quiet = contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
  });
  const first = await quiet.view(context(store, 1, 1_000_000));
  expect(first.manifest.epochDecision?.rebuild).toBe(false);
  store.append("s", {
    kind: "request_header",
    payload: {
      provider: MODEL.provider,
      model: MODEL.model,
      systemPromptHash: "s",
      toolsHash: "t",
      sampling: { temperature: 0 },
      adapterDefaults: {},
      system: HEAD.system,
      tools: HEAD.tools,
    },
  });
  store.append("s", { kind: "manifest", payload: first.manifest });

  const adapter = scripted([
    { type: "block_start", index: 0, blockType: "text" },
    {
      type: "text_delta",
      index: 0,
      text: JSON.stringify({
        decisions: [
          { id: "seq:1#t1", tier: "summary", summary: "file a" },
          { id: "seq:3#t2", tier: "full" },
        ],
      }),
    },
    { type: "block_end", index: 0 },
    { type: "usage", usage: USAGE },
    { type: "finish", kind: "stop" },
  ]);
  const second = await contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
    scorer: modelScorer({ adapters: { anthropic: adapter }, model: MODEL }),
  }).view(context(store, 2, 1));

  expect(adapter.requests).toHaveLength(1);
  const scoreReq = adapter.requests[0];
  if (!scoreReq) throw new Error("missing score request");
  const prev = rebuildFromLog(store.path("s"), 1);
  expect(scoreReq.head).toEqual(prev.head);
  expect(scoreReq.messages.slice(0, prev.messages.length)).toEqual(
    prev.messages,
  );
  expect(scoreReq.messages).toHaveLength(prev.messages.length + 1);
  expect(scoreReq.head.tools).toEqual(prev.head.tools);
  const manifest = second.manifest as typeof second.manifest & {
    scorer?: { calls: number; usage: Usage; fallback: boolean };
  };
  expect(manifest.scorer).toEqual({ calls: 1, usage: USAGE, fallback: false });
  expect(resultContent(second.messages, "t1")).toContain(
    "summary (model notes, not evidence): file a",
  );
  expect(resultContent(second.messages, "t2")).toBe("BBB");
  const t2 = second.manifest.chunks.find((chunk) => chunk.id === "seq:3#t2");
  expect(t2).toMatchObject({ tier: "full", reason: "scorer:model" });
});

it("falls back to rule tiers when the model returns invalid JSON", async () => {
  const store = new MemoryLogStore();
  seed(store);
  const first = await contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
  }).view(context(store, 1, 1_000_000));
  store.append("s", {
    kind: "request_header",
    payload: {
      provider: MODEL.provider,
      model: MODEL.model,
      systemPromptHash: "s",
      toolsHash: "t",
      sampling: {},
      adapterDefaults: {},
      system: HEAD.system,
      tools: HEAD.tools,
    },
  });
  store.append("s", { kind: "manifest", payload: first.manifest });
  const adapter = scripted([
    { type: "block_start", index: 0, blockType: "text" },
    { type: "text_delta", index: 0, text: "not json" },
    { type: "block_end", index: 0 },
    { type: "usage", usage: USAGE },
    { type: "finish", kind: "stop" },
  ]);
  const view = await contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
    scorer: modelScorer({ adapters: { anthropic: adapter }, model: MODEL }),
  }).view(context(store, 2, 1));
  expect(view.manifest.epochDecision?.reason).toContain(
    ";scorer:fallback:invalid-json",
  );
  expect(resultContent(view.messages, "t1")).toBe("AAA");
  expect(view.messages.length).toBeGreaterThan(0);
  const manifest = view.manifest as typeof view.manifest & {
    scorer?: { calls: number; usage: Usage; fallback: boolean };
  };
  expect(manifest.scorer).toEqual({ calls: 1, usage: USAGE, fallback: true });
});

it("keeps a summary note on the next turn of the same epoch without scoring again", async () => {
  let calls = 0;
  const scorer: Scorer = {
    name: "model",
    async score() {
      calls += 1;
      return {
        decisions: [{ id: "seq:1#t1", tier: "summary", summary: "file a" }],
      };
    },
  };
  const store = new MemoryLogStore();
  seed(store, "A".repeat(2000));
  const compiler = contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
    scorer,
  });
  const opened = await compiler.view(context(store, 1, 1));
  expect(calls).toBe(1);
  store.append("s", {
    kind: "view_epoch",
    payload: { epoch: opened.epoch, reason: opened.epochStart?.reason ?? "" },
    details: { epochHead: structuredClone(opened.messages[0]) },
  });
  store.append("s", { kind: "manifest", payload: opened.manifest });
  const stayed = await compiler.view(context(store, 2, 1_000_000));
  expect(stayed.manifest.epochDecision?.rebuild).toBe(false);
  expect(calls).toBe(1);
  expect(resultContent(stayed.messages, "t1")).toContain(
    "summary (model notes, not evidence): file a",
  );
  expect(
    stayed.manifest.chunks.find((chunk) => chunk.id === "seq:1#t1"),
  ).toMatchObject({
    tier: "summary",
    reason: "scorer:model",
    summary: "file a",
  });
});

it("rebuilds a logged summary tier as the same note the model saw", () => {
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "find the bug" }],
    },
  });
  store.append("s", { kind: "message", payload: toolMessage("t1", "a.ts") });
  store.append("s", { kind: "message", payload: resultMessage("t1", "AAA") });
  store.append("s", {
    kind: "request_header",
    payload: {
      provider: MODEL.provider,
      model: MODEL.model,
      systemPromptHash: "s",
      toolsHash: "t",
      sampling: {},
      adapterDefaults: {},
      system: HEAD.system,
      tools: HEAD.tools,
    },
  });
  store.append("s", {
    kind: "manifest",
    payload: {
      epoch: 1,
      turn: 1,
      claims: [],
      chunks: [
        {
          id: "seq:1#t1",
          tier: "summary",
          reason: "scorer:model",
          tokens: 8,
          originalTokens: 2,
          summary: "file a",
        },
      ],
      anomalies: [],
    },
  });
  const rebuilt = rebuildFromLog(store.path("s"), 1);
  expect(resultContent(rebuilt.messages, "t1")).toBe(
    [
      '[#seq:1#t1 · read · {"path":"a.ts"} · 0.0k tokens · omitted · expand("seq:1#t1") to retrieve]',
      "summary (model notes, not evidence): file a",
    ].join("\n"),
  );
});
