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
  "Output JSON only. No other text.",
  "tier must be one of none, summary, full.",
  "summary must be at most 200 characters.",
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
      JSON.stringify({
        decisions: [
          { id: "seq:12#x", tier: "summary", summary: "listed files" },
        ],
      }),
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
  expect(scoreReq.model).toEqual(SCORE_MODEL);
  expect(renderLlmInput(scoreReq, prev)).toBe(
    [
      "system: <same>",
      "tools: <same>",
      "messages:",
      "<last>",
      "+ [user] Output JSON only. No other text.",
      "+ tier must be one of none, summary, full.",
      "+ summary must be at most 200 characters.",
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
  const adapter = scripted(textScript('{"decisions":[]}'));
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
    textScript(
      JSON.stringify({
        decisions: [{ id: "seq:12#x", tier: "summary", summary }],
      }),
    ),
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
