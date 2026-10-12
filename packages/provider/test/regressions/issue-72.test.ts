import { expect, it } from "vitest";
import { AnthropicAdapter } from "../../src/anthropic/index.ts";
import type { ProviderAdapter, StreamChunk } from "../../src/contract/index.ts";
import { OpenAIAdapter } from "../../src/openai/index.ts";
import { expectStreamInvariants } from "../contract-suite.ts";
import { drain } from "../support.ts";

const OPENAI_OK = [
  'data: {"id":"c","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}',
  "",
  'data: {"id":"c","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}',
  "",
  'data: {"id":"c","object":"chat.completion.chunk","model":"m","choices":[],"usage":{"prompt_tokens":2,"completion_tokens":1}}',
  "",
  "data: [DONE]",
  "",
].join("\n");

const OPENAI_PARTIAL = [
  'data: {"id":"c","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}',
  "",
].join("\n");

const ANTHROPIC_OK = [
  "event: message_start",
  'data: {"type":"message_start","message":{"id":"msg_s","type":"message","role":"assistant","model":"m","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}}',
  "",
  "event: content_block_start",
  'data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
  "",
  "event: content_block_delta",
  'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}',
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

const ANTHROPIC_PARTIAL = [
  "event: message_start",
  'data: {"type":"message_start","message":{"id":"msg_s","type":"message","role":"assistant","model":"m","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":0}}}',
  "",
  "event: content_block_start",
  'data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
  "",
  "event: content_block_delta",
  'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}',
  "",
  "event: content_block_stop",
  'data: {"type":"content_block_stop","index":0}',
  "",
].join("\n");

type Step =
  | { action: "throw"; error: Error }
  | { action: "status"; status: number; body: string }
  | { action: "sse"; body: string };

type AdapterCase = {
  api: "openai-chat" | "anthropic-messages";
  ok: string;
  partial: string;
  make: (
    fetchImpl: typeof fetch,
    sleep: (ms: number) => Promise<void>,
  ) => ProviderAdapter;
};

const ADAPTERS: AdapterCase[] = [
  {
    api: "openai-chat",
    ok: OPENAI_OK,
    partial: OPENAI_PARTIAL,
    make: (fetchImpl, sleep) =>
      new OpenAIAdapter({ apiKey: "k", fetchImpl, sleep }),
  },
  {
    api: "anthropic-messages",
    ok: ANTHROPIC_OK,
    partial: ANTHROPIC_PARTIAL,
    make: (fetchImpl, sleep) =>
      new AnthropicAdapter({ apiKey: "k", fetchImpl, sleep }),
  },
];

function scripted(steps: Step[]): {
  fetchImpl: typeof fetch;
  calls: { n: number };
  waits: number[];
  sleep: (ms: number) => Promise<void>;
} {
  const calls = { n: 0 };
  const waits: number[] = [];
  const fetchImpl: typeof fetch = async () => {
    const step = steps[calls.n];
    calls.n += 1;
    if (!step) {
      return new Response(JSON.stringify({ error: { message: "extra" } }), {
        status: 400,
        headers: { "content-type": "application/json" },
      });
    }
    if (step.action === "throw") throw step.error;
    if (step.action === "sse") {
      return new Response(step.body, {
        status: 200,
        headers: { "content-type": "text/event-stream" },
      });
    }
    return new Response(step.body, {
      status: step.status,
      headers: { "content-type": "application/json" },
    });
  };
  return {
    fetchImpl,
    calls,
    waits,
    sleep: async (ms: number) => {
      waits.push(ms);
    },
  };
}

function jsonError(message: string): string {
  return JSON.stringify({ error: { message } });
}

async function run(
  adapterCase: AdapterCase,
  steps: Step[],
): Promise<{ chunks: StreamChunk[]; calls: number; waits: number[] }> {
  const seen = scripted(steps);
  const adapter = adapterCase.make(seen.fetchImpl, seen.sleep);
  const model = { provider: "openai", model: "gpt-5" };
  const chunks = await drain(
    adapter.stream(
      {
        model,
        head: { system: "sys", tools: [] },
        messages: [{ role: "user", content: [{ type: "text", text: "hi" }] }],
        cacheBreakpoints: [],
        sampling: { maxTokens: 16 },
      },
      adapter.prepare(model),
    ),
  );
  return { chunks, calls: seen.calls.n, waits: seen.waits };
}

function textOf(chunks: StreamChunk[]): string {
  return chunks
    .filter((chunk) => chunk.type === "text_delta")
    .map((chunk) => (chunk.type === "text_delta" ? chunk.text : ""))
    .join("");
}

function retryDetails(
  chunks: StreamChunk[],
): { retries: number; reason: string } | undefined {
  const usage = chunks.find((chunk) => chunk.type === "usage");
  if (usage?.type !== "usage") return undefined;
  return usage.usage.details;
}

it("retries a pre-stream fetch failure once and returns the reply", async () => {
  for (const adapterCase of ADAPTERS) {
    const { chunks, calls, waits } = await run(adapterCase, [
      { action: "throw", error: new TypeError("fetch failed") },
      { action: "sse", body: adapterCase.ok },
    ]);
    expect(textOf(chunks), adapterCase.api).toBe("Hi");
    expect(chunks.at(-1), adapterCase.api).toMatchObject({
      type: "finish",
      kind: "stop",
    });
    expect(retryDetails(chunks), adapterCase.api).toEqual({
      retries: 1,
      reason: "fetch failed",
    });
    expect(calls, adapterCase.api).toBe(2);
    expect(waits, adapterCase.api).toEqual([1000]);
    expectStreamInvariants(chunks);
  }
});

it("finishes with error after two retries of HTTP 503", async () => {
  const body = jsonError("unavailable");
  for (const adapterCase of ADAPTERS) {
    const { chunks, calls, waits } = await run(adapterCase, [
      { action: "status", status: 503, body },
      { action: "status", status: 503, body },
      { action: "status", status: 503, body },
    ]);
    expect(chunks.at(-1), adapterCase.api).toEqual({
      type: "finish",
      kind: "error",
      error: {
        message: "unavailable",
        cause: "http",
        status: 503,
        raw: JSON.parse(body),
      },
    });
    expect(retryDetails(chunks), adapterCase.api).toEqual({
      retries: 2,
      reason: "unavailable",
    });
    expect(calls, adapterCase.api).toBe(3);
    expect(waits, adapterCase.api).toEqual([1000, 4000]);
    expectStreamInvariants(chunks);
  }
});

it("does not retry HTTP 400", async () => {
  const body = jsonError("bad request");
  for (const adapterCase of ADAPTERS) {
    const { chunks, calls, waits } = await run(adapterCase, [
      { action: "status", status: 400, body },
      { action: "sse", body: adapterCase.ok },
    ]);
    expect(chunks.at(-1), adapterCase.api).toEqual({
      type: "finish",
      kind: "error",
      error: {
        message: "bad request",
        cause: "http",
        status: 400,
        raw: JSON.parse(body),
      },
    });
    expect(retryDetails(chunks), adapterCase.api).toBeUndefined();
    expect(calls, adapterCase.api).toBe(1);
    expect(waits, adapterCase.api).toEqual([]);
    expectStreamInvariants(chunks);
  }
});

it("does not retry after the response stream has started", async () => {
  for (const adapterCase of ADAPTERS) {
    const { chunks, calls, waits } = await run(adapterCase, [
      { action: "sse", body: adapterCase.partial },
      { action: "sse", body: adapterCase.ok },
    ]);
    expect(textOf(chunks), adapterCase.api).toBe("Hi");
    expect(chunks.at(-1), adapterCase.api).toMatchObject({
      type: "finish",
      kind: "error",
      error: { message: "stream ended before completion" },
    });
    expect(retryDetails(chunks), adapterCase.api).toBeUndefined();
    expect(calls, adapterCase.api).toBe(1);
    expect(waits, adapterCase.api).toEqual([]);
    expectStreamInvariants(chunks);
  }
});
