import { AnthropicAdapter, OpenAIAdapter } from "@sage/provider";
import type { ProviderAdapter } from "@sage/provider/contract";
import { describe, expect, it } from "vitest";
import { AgentSession, MemoryLogStore } from "../../src/index.ts";
import { SYSTEM } from "../support/faux.ts";

const ANTHROPIC_SSE = [
  "event: message_start",
  'data: {"type":"message_start","message":{"id":"msg_wire_42","type":"message","role":"assistant","model":"claude-sonnet-4-5-20250929","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}}',
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
  "",
].join("\n");

const OPENAI_SSE = [
  'data: {"id":"chatcmpl-wire-7","object":"chat.completion.chunk","model":"gpt-5-2026-08-07","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}',
  "",
  'data: {"id":"chatcmpl-wire-7","object":"chat.completion.chunk","model":"gpt-5-2026-08-07","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}',
  "",
  'data: {"id":"chatcmpl-wire-7","object":"chat.completion.chunk","model":"gpt-5-2026-08-07","choices":[],"usage":{"prompt_tokens":2,"completion_tokens":1}}',
  "",
  "data: [DONE]",
  "",
  "",
].join("\n");

function sseFetch(body: string): typeof fetch {
  return async () =>
    new Response(body, {
      status: 200,
      headers: { "content-type": "text/event-stream" },
    });
}

type Case = {
  name: string;
  model: { provider: string; model: string };
  adapter: ProviderAdapter;
  responseId: string;
  responseModel: string;
};

const CASES: Case[] = [
  {
    name: "anthropic",
    model: { provider: "anthropic", model: "claude-sonnet-4-5" },
    adapter: new AnthropicAdapter({
      apiKey: "k",
      fetchImpl: sseFetch(ANTHROPIC_SSE),
    }),
    responseId: "msg_wire_42",
    responseModel: "claude-sonnet-4-5-20250929",
  },
  {
    name: "openai",
    model: { provider: "openai", model: "gpt-5" },
    adapter: new OpenAIAdapter({
      apiKey: "k",
      fetchImpl: sseFetch(OPENAI_SSE),
    }),
    responseId: "chatcmpl-wire-7",
    responseModel: "gpt-5-2026-08-07",
  },
];

describe("response identity in the Log", () => {
  for (const item of CASES) {
    it(`stores the ${item.name} response id and model on the assistant entry`, async () => {
      const store = new MemoryLogStore();
      const session = new AgentSession({
        store,
        sessionId: item.name,
        adapter: item.adapter,
        model: item.model,
        system: SYSTEM,
        tools: [],
      });
      await session.run("go");
      const assistant = store.path(item.name).find((entry) => {
        if (entry.kind !== "message") return false;
        return (entry.payload as { role?: string }).role === "assistant";
      });

      expect(assistant?.details).toMatchObject({
        responseId: item.responseId,
        responseModel: item.responseModel,
      });
    });
  }
});
