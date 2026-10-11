import {
  AgentSession,
  assertPaired,
  type HarnessTool,
  MemoryLogStore,
  type ViewContext,
} from "@sage/core";
import {
  FAUX_UNIT_USAGE,
  fauxAdapter,
  recordViews,
  stopScript,
  toolScript,
} from "@sage/core/testing";
import type { Message } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { applyDecisions } from "../../src/compiler/apply.ts";
import type { Chunk } from "../../src/compiler/chunks.ts";
import { indexChunks } from "../../src/compiler/chunks.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { rebuildFromLog } from "../../src/compiler/rebuild.ts";

const MODEL = { provider: "anthropic", model: "claude-test" };

function declared(
  name: string,
  mode: "read" | "write" | "search",
  contents: string[],
): HarnessTool {
  let cursor = 0;
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
      if (typeof input !== "object" || input === null || !("path" in input)) {
        return [];
      }
      const key = input.path;
      if (typeof key !== "string" || key.length === 0) return [];
      return [{ key, mode }];
    },
    async execute() {
      const content = contents[cursor] ?? "";
      cursor += 1;
      return { content, isError: false };
    },
  };
}

function resultsOf(messages: Message[], toolUseId: string) {
  return messages.flatMap((message) =>
    message.content.filter(
      (block) => block.type === "tool_result" && block.toolUseId === toolUseId,
    ),
  );
}

function usesOf(messages: Message[], id: string) {
  return messages.flatMap((message) =>
    message.content.filter(
      (block) => block.type === "tool_use" && block.id === id,
    ),
  );
}

it("replaces a none tool result with a stub and leaves the tool_use and isError in place", () => {
  const messages: Message[] = [
    {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "b1",
          name: "bash",
          input: { cmd: "ls" },
          rawInput: '{"cmd":"ls"}',
        },
      ],
    },
    {
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: "b1",
          content: "boom",
          isError: true,
        },
      ],
    },
  ];
  const chunks: Chunk[] = indexChunks(messages, [3, 4], [], () => 1);
  const next = applyDecisions(messages, chunks, [
    { id: "seq:3#b1", tier: "none", reason: "retry_intermediate" },
  ]);
  expect(usesOf(next, "b1")).toEqual(usesOf(messages, "b1"));
  expect(resultsOf(next, "b1")).toEqual([
    {
      type: "tool_result",
      toolUseId: "b1",
      content:
        '[#seq:3#b1 · bash · {"cmd":"ls"} · 0.0k tokens · omitted · expand("seq:3#b1") to retrieve]',
      isError: true,
    },
  ]);
  expect(JSON.stringify(next)).not.toContain("[omitted]");
  expect(resultsOf(messages, "b1")[0]).toMatchObject({ content: "boom" });
  assertPaired(next);
});

it("omits a superseded tool result in the compiled view and keeps anomalies empty", async () => {
  const inputJson = JSON.stringify({ path: "/repo/a.ts" });
  const read = declared("read", "read", ["v1", "v2"]);
  const adapter = fauxAdapter([
    toolScript("r1", "read", inputJson, FAUX_UNIT_USAGE),
    toolScript("r2", "read", inputJson, FAUX_UNIT_USAGE),
    stopScript("done", FAUX_UNIT_USAGE),
  ]);
  const store = new MemoryLogStore();
  const recorder = recordViews(
    contextCompiler({ budget: 100_000, claims: [], keepRecent: 0 }),
  );
  const session = new AgentSession({
    store,
    sessionId: "omit",
    adapter,
    model: MODEL,
    system: "You are sage.",
    tools: [read],
    viewProvider: recorder,
  });

  await session.run("look");

  const third = recorder.views[2];
  if (!third) throw new Error("missing third view");
  assertPaired(third.messages);
  expect(usesOf(third.messages, "r1")).toHaveLength(1);
  expect(usesOf(third.messages, "r2")).toHaveLength(1);
  expect(resultsOf(third.messages, "r1")).toEqual([
    {
      type: "tool_result",
      toolUseId: "r1",
      content:
        '[#seq:3#r1 · read · {"path":"/repo/a.ts"} · 0.0k tokens · omitted · expand("seq:3#r1") to retrieve]',
      isError: false,
    },
  ]);
  expect(resultsOf(third.messages, "r2")).toEqual([
    { type: "tool_result", toolUseId: "r2", content: "v2", isError: false },
  ]);

  const manifests = store
    .all("omit")
    .filter((entry) => entry.kind === "manifest");
  const payload = manifests[2]?.payload as {
    anomalies?: unknown[];
    chunks?: { id: string; tier: string; reason: string; tokens?: number }[];
  };
  expect(payload.anomalies).toEqual([]);
  expect(payload.chunks).toEqual([
    expect.objectContaining({ id: "seq:0", tier: "full", reason: "kept" }),
    expect.objectContaining({
      id: "seq:3#r1",
      tier: "none",
      reason: "superseded_by:seq:7#r2",
    }),
    expect.objectContaining({ id: "seq:7#r2", tier: "full", reason: "kept" }),
  ]);
  expect(
    payload.chunks?.every((chunk) => typeof chunk.tokens === "number"),
  ).toBe(true);

  const sent = adapter.requests[2];
  if (!sent) throw new Error("missing third request");
  const rebuilt = rebuildFromLog(store.path("omit"), 3);
  expect(rebuilt.messages).toEqual(sent.messages);
  expect(rebuilt.head).toEqual(sent.head);
});

it("protects the six most recent tool chunks when keepRecent is omitted", async () => {
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "go" }],
    },
  });
  for (let index = 0; index < 7; index += 1) {
    const id = `c${index}`;
    store.append("s", {
      kind: "message",
      payload: {
        role: "assistant",
        content: [
          {
            type: "tool_use",
            id,
            name: "read",
            input: { path: `/repo/${index}.ts` },
            rawInput: "{}",
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
            content: "body",
            isError: false,
          },
        ],
      },
    });
  }
  const ctx: ViewContext = {
    sessionId: "s",
    turn: 37,
    path: store.path("s"),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: 8000,
      pricing: null,
      adapterDefaults: {},
    },
    head: { system: "You are sage.", tools: [] },
    tools: [declared("read", "read", [])],
  };
  const view = await contextCompiler({ budget: 100_000, claims: [] }).view(ctx);
  const reasons = view.manifest.chunks
    .filter((chunk) => chunk.id.includes("#"))
    .map((chunk) => chunk.reason);
  expect(reasons).toEqual([
    "kept",
    "recent",
    "recent",
    "recent",
    "recent",
    "recent",
    "recent",
  ]);
  expect(view.manifest.anomalies).toEqual([]);
});

it("renders a summary tier as the stub plus a model note", () => {
  const messages: Message[] = [
    {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "b1",
          name: "bash",
          input: { cmd: "ls" },
          rawInput: '{"cmd":"ls"}',
        },
      ],
    },
    {
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: "b1",
          content: "ORIGINAL",
          isError: false,
        },
      ],
    },
  ];
  const chunks: Chunk[] = indexChunks(messages, [12, 13], [], () => 1);
  const next = applyDecisions(messages, chunks, [
    {
      id: "seq:12#b1",
      tier: "summary",
      reason: "scorer:model",
      summary: "listed the workspace",
    },
  ]);
  expect(resultsOf(next, "b1")).toEqual([
    {
      type: "tool_result",
      toolUseId: "b1",
      content: [
        '[#seq:12#b1 · bash · {"cmd":"ls"} · 0.0k tokens · omitted · expand("seq:12#b1") to retrieve]',
        "summary (model notes, not evidence): listed the workspace",
      ].join("\n"),
      isError: false,
    },
  ]);
  expect(resultsOf(messages, "b1")[0]).toMatchObject({ content: "ORIGINAL" });
  assertPaired(next);
});
