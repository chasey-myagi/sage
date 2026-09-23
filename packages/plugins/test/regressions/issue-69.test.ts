import { MemoryLogStore, type ViewContext } from "@sage/core";
import { AnthropicAdapter, OpenAIAdapter } from "@sage/provider";
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

function chunk(id: string, tool = "bash"): Chunk {
  return {
    id,
    kind: "tool",
    messageIndex: 0,
    toolUseId: id.slice(id.indexOf("#") + 1),
    toolName: tool,
    input: { cmd: "ls" },
    tokens: 12,
    accesses: [],
  };
}

function prevRequest(model: { provider: string; model: string }): Request {
  return {
    model,
    head: {
      system: "You are sage.",
      tools: [
        {
          name: "edit",
          description: "edit a file",
          inputSchema: {
            type: "object",
            properties: { path: { type: "string" } },
          },
        },
      ],
    },
    messages: [
      { role: "user", content: [{ type: "text", text: "hello" }] },
      { role: "assistant", content: [{ type: "text", text: "ok" }] },
    ],
    cacheBreakpoints: ["head", { afterMessage: 1 }],
    sampling: { temperature: 0 },
  };
}

function textScript(text: string, usage: Usage = USAGE): StreamChunk[] {
  return [
    { type: "block_start", index: 0, blockType: "text" },
    { type: "text_delta", index: 0, text },
    { type: "block_end", index: 0 },
    { type: "usage", usage },
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
  const model = { provider: "anthropic", model: "claude-haiku-4-5" };
  const result = await modelScorer({
    adapters: { anthropic: wrapped },
    model,
  }).score({
    candidates,
    prevRequest: prevRequest(model),
    query: "find the bug",
  });
  return { result, request: seen[0] };
}

it("parses a fenced JSON array", async () => {
  const text = [
    "```json",
    '[{"id":"a","tier":"none"},{"id":"b","tier":"full"}]',
    "```",
  ].join("\n");
  const { result } = await score(text, [chunk("a"), chunk("b")]);
  expect(result.fallback).toBeUndefined();
  expect(result.raw).toBeUndefined();
  expect(result.decisions).toEqual([
    { id: "a", tier: "none" },
    { id: "b", tier: "full" },
  ]);
});

it("wraps a single object that has an id into one decision", async () => {
  const { result } = await score('{"id":"a","tier":"none"}', [chunk("a")]);
  expect(result.fallback).toBeUndefined();
  expect(result.raw).toBeUndefined();
  expect(result.decisions).toEqual([{ id: "a", tier: "none" }]);
});

it("falls back when the object has no id and keeps a raw prefix", async () => {
  const text = '{"tier":"summary","summary":"Read src appended (reviewed)"}';
  const { result } = await score(text, [chunk("a")]);
  expect(result.decisions).toEqual([]);
  expect(result.fallback).toBe("invalid-json");
  expect(result.raw?.text.startsWith(text.slice(0, 300))).toBe(true);
  expect(result.raw?.text.length).toBeLessThanOrEqual(300);
  expect(result.raw?.finish).toBe("stop");
});

it("treats an empty array as keep-all", async () => {
  const { result } = await score("[]", [chunk("a")]);
  expect(result.fallback).toBeUndefined();
  expect(result.raw).toBeUndefined();
  expect(result.decisions).toEqual([]);
  expect(result.dropped).toBeUndefined();
});

it("drops decisions whose id is not a candidate and counts them", async () => {
  const text = '[{"id":"a","tier":"none"},{"id":"missing","tier":"full"}]';
  const { result } = await score(text, [chunk("a")]);
  expect(result.fallback).toBeUndefined();
  expect(result.raw).toBeUndefined();
  expect(result.decisions).toEqual([{ id: "a", tier: "none" }]);
  expect(result.dropped).toBe(1);
});

it("keeps only the first 300 characters of fallback text", async () => {
  const text = `x${"y".repeat(400)}`;
  const { result } = await score(text, [chunk("a")]);
  expect(result.fallback).toBe("invalid-json");
  expect(result.raw).toEqual({ text: text.slice(0, 300), finish: "stop" });
});

it("pins the scorer instruction", async () => {
  const { request } = await score("[]", [chunk("a")]);
  if (!request) throw new Error("missing score request");
  const instruction = instructionOf(request);
  expect(instruction).toBe(INSTRUCTION);
  expect(instruction).toContain("do not call tools");
  expect(instruction).toContain("seq:1#a");
  expect(instruction).toContain("seq:2#b");
});

const OPENAI_SSE = [
  'data: {"id":"c","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"content":"[]"},"finish_reason":null}]}',
  "",
  'data: {"id":"c","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}',
  "",
  'data: {"id":"c","object":"chat.completion.chunk","model":"m","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":1}}',
  "",
  "data: [DONE]",
  "",
].join("\n");

const ANTHROPIC_SSE = [
  "event: message_start",
  'data: {"type":"message_start","message":{"id":"msg_s","type":"message","role":"assistant","model":"m","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}}',
  "",
  "event: content_block_start",
  'data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
  "",
  "event: content_block_delta",
  'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"[]"}}',
  "",
  "event: content_block_stop",
  'data: {"type":"content_block_stop","index":0}',
  "",
  "event: message_delta",
  'data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}',
  "",
  "event: message_stop",
  'data: {"type":"message_stop"}',
  "",
].join("\n");

function recordFetch(sse: string): {
  fetchImpl: typeof fetch;
  calls: { body: unknown }[];
} {
  const calls: { body: unknown }[] = [];
  const fetchImpl: typeof fetch = async (_input, init) => {
    calls.push({
      body: JSON.parse(String(init?.body ?? "null")) as unknown,
    });
    return new Response(sse, {
      status: 200,
      headers: { "content-type": "text/event-stream" },
    });
  };
  return { fetchImpl, calls };
}

function asRecord(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("expected object");
  }
  return value as Record<string, unknown>;
}

async function scoredBodies(kind: "openai" | "anthropic"): Promise<{
  previous: Record<string, unknown>;
  score: Record<string, unknown>;
}> {
  const sse = kind === "openai" ? OPENAI_SSE : ANTHROPIC_SSE;
  const { fetchImpl, calls } = recordFetch(sse);
  const adapter =
    kind === "openai"
      ? new OpenAIAdapter({ apiKey: "k", fetchImpl })
      : new AnthropicAdapter({ apiKey: "k", fetchImpl });
  const model = { provider: kind, model: "m" };
  const prev = prevRequest(model);
  const stream = adapter.stream(prev, adapter.prepare(model));
  for await (const _chunk of stream) {
    // drain the previous request so its wire body is recorded
  }
  await modelScorer({
    adapters: { [kind]: adapter },
    model,
  }).score({
    candidates: [chunk("seq:1#a")],
    prevRequest: prev,
    query: "find the bug",
  });
  return {
    previous: asRecord(calls[0]?.body),
    score: asRecord(calls[1]?.body),
  };
}

function prefixSame(
  previous: Record<string, unknown>,
  score: Record<string, unknown>,
): void {
  expect(score.tools).toEqual(previous.tools);
  const prevMessages = previous.messages;
  const scoreMessages = score.messages;
  if (!Array.isArray(prevMessages) || !Array.isArray(scoreMessages)) {
    throw new Error("messages");
  }
  expect(JSON.stringify(scoreMessages.slice(0, prevMessages.length))).toBe(
    JSON.stringify(prevMessages),
  );
  expect(previous).not.toHaveProperty("tool_choice");
}

it("sends openai tool_choice none without changing the message prefix or tools", async () => {
  const { previous, score } = await scoredBodies("openai");
  prefixSame(previous, score);
  expect(score.tool_choice).toBe("none");
});

it("sends anthropic tool_choice none without changing the message prefix or tools", async () => {
  const { previous, score } = await scoredBodies("anthropic");
  prefixSame(previous, score);
  expect(score.tool_choice).toEqual({ type: "none" });
});

const SHADOW_MODEL = { provider: "anthropic", model: "claude-test" };

function shadowContext(store: MemoryLogStore, turn: number): ViewContext {
  return {
    sessionId: "s",
    turn,
    path: store.path("s"),
    model: SHADOW_MODEL,
    prepared: {
      model: SHADOW_MODEL,
      contextWindow: 1_000_000,
      pricing: null,
      adapterDefaults: {},
    },
    head: { system: "You are sage.", tools: [] },
    tools: [],
  };
}

async function shadowOf(text: string) {
  const compiler = contextCompiler({
    budget: 100_000,
    claims: [],
    shadowScorer: modelScorer({
      adapters: { anthropic: scripted(textScript(text)) },
      model: SHADOW_MODEL,
    }),
  });
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "find the bug" }],
    },
  });
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "t1",
          name: "read",
          input: { path: "a.ts" },
          rawInput: '{"path":"a.ts"}',
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
          toolUseId: "t1",
          content: "AAA",
          isError: false,
        },
      ],
    },
  });
  const view = await compiler.view(shadowContext(store, 1));
  return view.manifest.shadow;
}

it("writes shadow.raw only when the model scorer falls back", async () => {
  const text = '{"tier":"summary","summary":"Read src appended (reviewed)"}';
  const bad = await shadowOf(text);
  expect(bad?.fallback).toBe("invalid-json");
  expect(bad?.raw).toEqual({ text, finish: "stop" });
  expect(bad?.decisions).toEqual([]);

  const good = await shadowOf('[{"id":"seq:1#t1","tier":"none"}]');
  expect(good?.fallback).toBeUndefined();
  expect(good?.raw).toBeUndefined();
  expect(good?.decisions).toEqual([{ id: "seq:1#t1", tier: "none" }]);
});
