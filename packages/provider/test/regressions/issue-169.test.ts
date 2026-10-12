import { describe, expect, it } from "vitest";
import { AnthropicAdapter } from "../../src/anthropic/index.ts";
import type {
  ModelRef,
  ProviderAdapter,
  Request,
  StreamChunk,
} from "../../src/contract/index.ts";
import { OpenAIAdapter } from "../../src/openai/index.ts";
import { expectStreamInvariants } from "../contract-suite.ts";
import { drain } from "../support.ts";

type Wire = {
  name: string;
  make: (fetchImpl: typeof fetch) => ProviderAdapter;
  /** A 200 SSE body that streams text, then an error event with `error`. */
  body: (error: Record<string, unknown>) => string;
};

const WIRES: Wire[] = [
  {
    name: "OpenAI",
    make: (fetchImpl) =>
      new OpenAIAdapter({ apiKey: "k", fetchImpl, sleep: async () => {} }),
    body: (error) =>
      [
        'data: {"id":"c","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}',
        "",
        `data: ${JSON.stringify({ error })}`,
        "",
        "",
      ].join("\n"),
  },
  {
    name: "Anthropic",
    make: (fetchImpl) =>
      new AnthropicAdapter({ apiKey: "k", fetchImpl, sleep: async () => {} }),
    body: (error) =>
      [
        "event: message_start",
        'data: {"type":"message_start","message":{"id":"msg","type":"message","role":"assistant","model":"m","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}}',
        "",
        "event: error",
        `data: ${JSON.stringify({ type: "error", error })}`,
        "",
        "",
      ].join("\n"),
  },
];

type Case = {
  name: string;
  error: Record<string, unknown>;
  event: Record<string, unknown>;
  /** Wires this case applies to; every wire when absent. */
  only?: string;
};

const CASES: Case[] = [
  {
    name: "an event with only a message is transient",
    error: { message: "Upstream rate limit exceeded, please retry later" },
    event: { transient: true },
  },
  {
    name: "a numeric 429 status is transient",
    error: { message: "slow down", status: 429 },
    event: { transient: true, status: 429 },
  },
  {
    name: "a numeric 503 status is transient",
    error: { message: "busy", status: 503 },
    event: { transient: true, status: 503 },
  },
  {
    name: "a numeric 400 status is permanent",
    error: { message: "nope", status: 400 },
    event: { transient: false, status: 400 },
  },
  {
    name: "a numeric code is carried and does not decide",
    error: { message: "busy", code: 400 },
    event: { transient: true, code: 400 },
  },
  {
    name: "an unknown type is transient",
    error: { message: "x", type: "upstream_error", code: "stalled" },
    event: { transient: true, type: "upstream_error", code: "stalled" },
  },
  {
    name: "invalid_request_error is permanent",
    error: { message: "x", type: "invalid_request_error" },
    event: { transient: false, type: "invalid_request_error" },
  },
  {
    name: "a server_error type is transient",
    error: { message: "x", type: "server_error", code: null },
    event: { transient: true, type: "server_error" },
    only: "OpenAI",
  },
  {
    name: "a content_filter code is permanent",
    error: { message: "x", type: "server_error", code: "content_filter" },
    event: { transient: false, type: "server_error", code: "content_filter" },
    only: "OpenAI",
  },
  {
    name: "a context_length_exceeded code is permanent",
    error: { message: "x", code: "context_length_exceeded" },
    event: { transient: false, code: "context_length_exceeded" },
    only: "OpenAI",
  },
  {
    name: "an insufficient_quota type is permanent",
    error: { message: "x", type: "insufficient_quota" },
    event: { transient: false, type: "insufficient_quota" },
    only: "OpenAI",
  },
  {
    name: "overloaded_error is transient",
    error: { message: "Overloaded", type: "overloaded_error" },
    event: { transient: true, type: "overloaded_error" },
    only: "Anthropic",
  },
  {
    name: "authentication_error is permanent",
    error: { message: "x", type: "authentication_error" },
    event: { transient: false, type: "authentication_error" },
    only: "Anthropic",
  },
  {
    name: "request_too_large is permanent",
    error: { message: "x", type: "request_too_large" },
    event: { transient: false, type: "request_too_large" },
    only: "Anthropic",
  },
];

const MODEL: ModelRef = { provider: "anthropic", model: "claude-sonnet-4-5" };

const REQUEST: Request = {
  model: MODEL,
  head: { system: "sys", tools: [] },
  messages: [{ role: "user", content: [{ type: "text", text: "hi" }] }],
  cacheBreakpoints: [],
  sampling: { maxTokens: 16 },
};

function fetchOnce(body: string): typeof fetch {
  return async () =>
    new Response(body, {
      status: 200,
      headers: { "content-type": "text/event-stream" },
    });
}

describe("classifying an in-stream error event", () => {
  for (const wire of WIRES) {
    for (const item of CASES) {
      if (item.only !== undefined && item.only !== wire.name) continue;
      it(`${item.name} (${wire.name})`, async () => {
        const adapter = wire.make(fetchOnce(wire.body(item.error)));
        const chunks: StreamChunk[] = await drain(
          adapter.stream(REQUEST, adapter.prepare(MODEL)),
        );
        const finish = chunks.at(-1);
        if (finish?.type !== "finish" || !finish.error) {
          throw new Error("expected an error finish");
        }
        expect(finish.error.event).toEqual(item.event);
        expect(finish.error).not.toHaveProperty("transport");
        expect(finish.error).not.toHaveProperty("status");
        expectStreamInvariants(chunks);
      });
    }
  }
});
