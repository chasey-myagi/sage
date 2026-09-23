import { renderLlmInput } from "@sage/core/testing";
import type {
  ProviderAdapter,
  Request,
  StreamChunk,
  Usage,
} from "@sage/provider/contract";
import { expect, it } from "vitest";
import type { Chunk } from "../../src/compiler/chunks.ts";
import { modelScorer } from "../../src/scorer/model.ts";

const PREV_MODEL = { provider: "anthropic", model: "claude-sonnet-5" };
const SCORE_MODEL = { provider: "anthropic", model: "claude-haiku-4-5" };

const USAGE: Usage = {
  inputOther: 11,
  inputCacheRead: 80,
  inputCacheCreation: 2,
  output: 7,
};

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

function prevRequest(): Request {
  return {
    model: PREV_MODEL,
    head: {
      system: "You are sage.",
      tools: [
        {
          name: "bash",
          description: "run a command",
          inputSchema: {
            type: "object",
            properties: { cmd: { type: "string" } },
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

function scripted(
  script:
    | StreamChunk[]
    | ((signal: AbortSignal | undefined) => Promise<StreamChunk[]>),
): ProviderAdapter & { requests: Request[]; calls: number } {
  const requests: Request[] = [];
  const adapter: ProviderAdapter & { requests: Request[]; calls: number } = {
    api: "anthropic-messages",
    requests,
    calls: 0,
    prepare(model) {
      return {
        model,
        contextWindow: 8000,
        pricing: null,
        adapterDefaults: {},
      };
    },
    stream(request, _prepared, signal) {
      requests.push(structuredClone(request));
      adapter.calls += 1;
      return (async function* () {
        const turn =
          typeof script === "function" ? await script(signal) : script;
        for (const chunk of turn) yield chunk;
      })();
    },
  };
  return adapter;
}

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

it("extends the previous main request by one user message and keeps its tools", async () => {
  const prev = prevRequest();
  const listed = [
    {
      id: "seq:12#x",
      tool: "bash",
      summary: '{"cmd":"ls"}',
      tokens: 12,
    },
  ];
  const text = [
    INSTRUCTION,
    "Query: find the bug",
    JSON.stringify(listed),
  ].join("\n");
  const adapter = scripted(
    textScript(
      JSON.stringify([
        { id: "seq:12#x", tier: "summary", summary: "listed files" },
      ]),
    ),
  );
  const scorer = modelScorer({
    adapters: { anthropic: adapter },
    model: SCORE_MODEL,
  });
  const result = await scorer.score({
    candidates: [chunk("seq:12#x")],
    prevRequest: prev,
    query: "find the bug",
  });

  expect(scorer.name).toBe("model");
  expect(adapter.calls).toBe(1);
  const scoreReq = adapter.requests[0];
  if (!scoreReq) throw new Error("missing score request");
  expect(scoreReq.head).toEqual(prev.head);
  expect(scoreReq.messages.slice(0, prev.messages.length)).toEqual(
    prev.messages,
  );
  expect(scoreReq.messages).toHaveLength(prev.messages.length + 1);
  expect(scoreReq.messages.at(-1)).toEqual({
    role: "user",
    content: [{ type: "text", text }],
  });
  expect(scoreReq.head.tools).toEqual(prev.head.tools);
  expect(scoreReq.toolChoice).toBe("none");
  expect(scoreReq.model).toEqual(SCORE_MODEL);
  expect(renderLlmInput(scoreReq, prev)).toBe(
    [
      "system: <same>",
      "tools: <same>",
      "messages:",
      "<last>",
      "+ [user] You are judging which earlier tool results can be dropped from the context.",
      "+ This is a side query, not a continuation of the task.",
      "+ do not call tools. Do not output any other text.",
      "+ Each candidate is the result of an earlier tool call.",
      "+ Output a JSON array only.",
      '+ Each item is {"id":"<candidate id>","tier":"none"|"summary"|"full","summary"?:string}.',
      "+ Copy id from the candidate list.",
      "+ summary is optional and at most 200 characters.",
      "+ A summary must let a later turn locate the result without rereading it.",
      "+ File read: path, key symbols or exports, and the line range.",
      "+ grep or ls: the matching paths and line numbers.",
      '+ Command output: the command and conclusive numbers or status, for example "tests 23 passed 1 failed: name".',
      '+ Do not write a judgment only ("already reviewed", "not needed", "same as before"). Use tier none or full instead.',
      "+ 文件读取写路径、关键符号或导出、以及行号范围；grep 或 ls 写命中路径与行号；命令输出写命令和结论性数字或状态。不要只写判断（已看过、不再需要、和之前一样），改判 none 或 full。",
      "+ tier none drops the result, summary keeps a short note, full keeps the original text.",
      "+ Example:",
      '+ [{"id":"seq:1#a","tier":"none"},{"id":"seq:2#b","tier":"summary","summary":"src/utils/url.ts: isURLMatched, isURLInList (L12-40)"}]',
      "+ Query: find the bug",
      `+ ${JSON.stringify(listed)}`,
    ].join("\n"),
  );
  expect(result.fallback).toBeUndefined();
  expect(result.usage).toEqual(USAGE);
  expect(result.decisions).toEqual([
    { id: "seq:12#x", tier: "summary", summary: "listed files" },
  ]);
});

it("does not call the adapter when there is no previous request", async () => {
  const adapter = scripted(textScript("{}"));
  const scorer = modelScorer({
    adapters: { anthropic: adapter },
    model: SCORE_MODEL,
  });
  const result = await scorer.score({
    candidates: [chunk("seq:1#a")],
    prevRequest: undefined,
    query: "find the bug",
  });
  expect(adapter.calls).toBe(0);
  expect(result.decisions).toEqual([]);
  expect(result.fallback).toBe("no-previous-request");
  expect(result.usage).toBeUndefined();
});

it("drops the oldest candidates past the default cap of 60", async () => {
  const candidates = Array.from({ length: 61 }, (_, index) =>
    chunk(`seq:${index}#c${index}`),
  );
  const adapter = scripted(textScript("[]"));
  const scorer = modelScorer({
    adapters: { anthropic: adapter },
    model: SCORE_MODEL,
  });
  await scorer.score({
    candidates,
    prevRequest: prevRequest(),
    query: "q",
  });
  const text = JSON.stringify(adapter.requests[0]?.messages.at(-1));
  expect(text).not.toContain("seq:0#c0");
  expect(text).toContain("seq:1#c1");
  expect(text).toContain("seq:60#c60");
});

it("truncates a summary to 200 characters", async () => {
  const summary = "s".repeat(250);
  const adapter = scripted(
    textScript(JSON.stringify([{ id: "seq:12#x", tier: "summary", summary }])),
  );
  const result = await modelScorer({
    adapters: { anthropic: adapter },
    model: SCORE_MODEL,
  }).score({
    candidates: [chunk("seq:12#x")],
    prevRequest: prevRequest(),
    query: "q",
  });
  expect(result.decisions[0]?.summary).toBe("s".repeat(200));
});

it("falls back when the model returns invalid JSON", async () => {
  const adapter = scripted(textScript("not json"));
  const result = await modelScorer({
    adapters: { anthropic: adapter },
    model: SCORE_MODEL,
  }).score({
    candidates: [chunk("seq:12#x")],
    prevRequest: prevRequest(),
    query: "q",
  });
  expect(result.decisions).toEqual([]);
  expect(result.fallback).toBe("invalid-json");
  expect(result.usage).toEqual(USAGE);
});

it("falls back when the adapter finishes with an error", async () => {
  const adapter = scripted([
    { type: "usage", usage: USAGE },
    { type: "finish", kind: "error", error: { message: "nope" } },
  ]);
  const result = await modelScorer({
    adapters: { anthropic: adapter },
    model: SCORE_MODEL,
  }).score({
    candidates: [chunk("seq:12#x")],
    prevRequest: prevRequest(),
    query: "q",
  });
  expect(result.decisions).toEqual([]);
  expect(result.fallback).toBe("finish:error");
  expect(result.usage).toEqual(USAGE);
});

it("falls back on timeout and stops waiting", async () => {
  const adapter = scripted(async (signal) => {
    await new Promise<void>((resolve) => {
      const timer = setTimeout(resolve, 60_000);
      const abort = () => {
        clearTimeout(timer);
        resolve();
      };
      if (signal?.aborted) abort();
      else signal?.addEventListener("abort", abort, { once: true });
    });
    return [{ type: "finish", kind: "aborted" }];
  });
  const started = Date.now();
  const result = await modelScorer({
    adapters: { anthropic: adapter },
    model: SCORE_MODEL,
    timeoutMs: 40,
  }).score({
    candidates: [chunk("seq:12#x")],
    prevRequest: prevRequest(),
    query: "q",
  });
  expect(Date.now() - started).toBeLessThan(2_000);
  expect(result.decisions).toEqual([]);
  expect(result.fallback).toBe("timeout");
  expect(adapter.calls).toBe(1);
});
