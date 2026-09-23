import { MemoryLogStore, runHealth, type ViewContext } from "@sage/core";
import type {
  ProviderAdapter,
  Request,
  StreamChunk,
  Usage,
} from "@sage/provider/contract";
import { expect, it } from "vitest";
import type { Chunk } from "../../src/compiler/chunks.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { modelScorer } from "../../src/scorer/model.ts";
import type { Scorer } from "../../src/scorer/types.ts";

const MODEL = { provider: "anthropic", model: "claude-sonnet-5" };
const HEAD = { system: "You are sage.", tools: [] as const };
const BODY = `ENTRY87_${"Q".repeat(400)}`;
const USAGE: Usage = {
  inputOther: 11,
  inputCacheRead: 80,
  inputCacheCreation: 2,
  output: 7,
};

const INSTRUCTION = [
  "You are judging which earlier tool results can be dropped from the context.",
  "This is a side query, not a continuation of the task.",
  "do not call tools. Do not output any other text.",
  "Each candidate is the result of an earlier tool call.",
  "Output a JSON array only.",
  'Each item is {"id":"<candidate id>","tier":"none"|"summary"|"full","summary"?:string}.',
  "Copy id from the candidate list.",
  "summary is optional and at most 200 characters.",
  "A summary must let a later turn locate the result without rereading it.",
  "File read: path, key symbols or exports, and the line range.",
  "grep or ls: the matching paths and line numbers.",
  'Command output: the command and conclusive numbers or status, for example "tests 23 passed 1 failed: name".',
  'Do not write a judgment only ("already reviewed", "not needed", "same as before"). Use tier none or full instead.',
  "文件读取写路径、关键符号或导出、以及行号范围；grep 或 ls 写命中路径与行号；命令输出写命令和结论性数字或状态。不要只写判断（已看过、不再需要、和之前一样），改判 none 或 full。",
  "tier none drops the result, summary keeps a short note, full keeps the original text.",
  "Example:",
  '[{"id":"seq:1#a","tier":"none"},{"id":"seq:2#b","tier":"summary","summary":"src/utils/url.ts: isURLMatched, isURLInList (L12-40)"}]',
].join("\n");

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

it("rejects a judgment-only summary and counts it", async () => {
  const view = await compileBig("already reviewed; not needed");
  expect(chunkOf(view.view, "big")).toMatchObject({
    tier: "full",
    reason: "scorer:entry:full;summary-rejected",
  });
  expect(resultOf(view.view.messages, "big")).toBe(BODY);
  expect(view.view.manifest.entryScored).toMatchObject({
    calls: 1,
    none: 0,
    summary: 0,
    full: 1,
    summaryRejected: 1,
  });
  expect(runHealth(view.store.path("s")).entryScored.summaryRejected).toBe(1);
});

it("keeps a summary that names the path, symbols, and lines", async () => {
  const note = "src/utils/url.ts: isURLMatched, isURLInList (L12-40)";
  const view = await compileBig(note);
  expect(chunkOf(view.view, "big")).toMatchObject({
    tier: "summary",
    reason: "scorer:entry:summary",
  });
  expect(resultOf(view.view.messages, "big")).toContain(
    `summary (model notes, not evidence): ${note}`,
  );
  expect(resultOf(view.view.messages, "big")).not.toContain("ENTRY87");
  expect(view.view.manifest.entryScored).toMatchObject({
    summary: 1,
    full: 0,
    summaryRejected: 0,
  });
});

it("rejects an empty summary string", async () => {
  const view = await compileBig("");
  expect(chunkOf(view.view, "big")).toMatchObject({
    tier: "full",
    reason: "scorer:entry:full;summary-rejected",
  });
  expect(resultOf(view.view.messages, "big")).toBe(BODY);
  expect(runHealth(view.store.path("s")).entryScored.summaryRejected).toBe(1);
});

it("rejects a summary shorter than 20 characters", async () => {
  const view = await compileBig("src/a.ts:1");
  expect(chunkOf(view.view, "big")).toMatchObject({
    tier: "full",
    reason: "scorer:entry:full;summary-rejected",
  });
  expect(resultOf(view.view.messages, "big")).toBe(BODY);
  expect(runHealth(view.store.path("s")).entryScored.summaryRejected).toBe(1);
});

it("snapshots the summary rules in the scorer instruction", async () => {
  const { request } = await score("[]", [scoreChunk("a")]);
  if (!request) throw new Error("missing score request");
  const instruction = instructionOf(request);
  expect(instruction).toBe(INSTRUCTION);
  expect(instruction).toContain("路径");
  expect(instruction).toContain("行号");
  expect(instruction).toContain("不要只写判断");
});

async function compileBig(summary: string): Promise<{
  store: MemoryLogStore;
  view: Awaited<ReturnType<ReturnType<typeof contextCompiler>["view"]>>;
}> {
  const scorer: Scorer = {
    name: "fake",
    async score(input) {
      return {
        decisions: input.candidates.map((chunk) => ({
          id: chunk.id,
          tier: "summary" as const,
          summary,
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

function user(text: string): MessageUser {
  return { role: "user", content: [{ type: "text", text }] };
}

type MessageUser = {
  role: "user";
  content: { type: "text"; text: string }[];
};

function appendTool(
  store: MemoryLogStore,
  id: string,
  filePath: string,
  output: string,
): void {
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [
        {
          type: "tool_use" as const,
          id,
          name: "read",
          input: { path: filePath },
          rawInput: JSON.stringify({ path: filePath }),
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
          type: "tool_result" as const,
          toolUseId: id,
          content: output,
          isError: false,
        },
      ],
    },
  });
}

function chunkOf(
  view: {
    manifest: { chunks: { id: string; tier: string; reason: string }[] };
  },
  toolUseId: string,
): { id: string; tier: string; reason: string } | undefined {
  return view.manifest.chunks.find((chunk) =>
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function scoreChunk(id: string): Chunk {
  return {
    id,
    kind: "tool",
    messageIndex: 0,
    toolUseId: id,
    toolName: "bash",
    input: { cmd: "ls" },
    tokens: 12,
    accesses: [],
  };
}

function prevRequest(): Request {
  return {
    model: MODEL,
    head: { system: HEAD.system, tools: [] },
    messages: [{ role: "user", content: [{ type: "text", text: "hello" }] }],
    cacheBreakpoints: ["head"],
    sampling: {},
  };
}

function textScript(text: string): StreamChunk[] {
  return [
    { type: "block_start", index: 0, blockType: "text" },
    { type: "text_delta", index: 0, text },
    { type: "block_end", index: 0 },
    { type: "usage", usage: USAGE },
    { type: "finish", kind: "stop" },
  ];
}

function scripted(script: StreamChunk[]): ProviderAdapter {
  const adapter: ProviderAdapter = {
    api: "anthropic-messages",
    prepare(model) {
      return {
        model,
        contextWindow: 8000,
        pricing: null,
        adapterDefaults: {},
      };
    },
    stream() {
      return (async function* () {
        for (const item of script) yield item;
      })();
    },
  };
  return adapter;
}

function instructionOf(request: Request): string {
  const last = request.messages.at(-1);
  const block = last?.content[0];
  if (block?.type !== "text") throw new Error("missing instruction");
  const at = block.text.indexOf("\nQuery:");
  if (at < 0) throw new Error("missing query");
  return block.text.slice(0, at);
}

async function score(text: string, candidates: Chunk[]) {
  const seen: Request[] = [];
  const adapter = scripted(textScript(text));
  const wrapped: ProviderAdapter = {
    ...adapter,
    stream(request, prepared, signal) {
      seen.push(structuredClone(request));
      return adapter.stream(request, prepared, signal);
    },
  };
  const result = await modelScorer({
    adapters: { anthropic: wrapped },
    model: MODEL,
  }).score({
    candidates,
    prevRequest: prevRequest(),
    query: "find the bug",
  });
  return { result, request: seen[0] };
}
