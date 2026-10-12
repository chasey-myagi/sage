import { AnthropicAdapter, OpenAIAdapter } from "@sage/provider";
import type { ProviderAdapter } from "@sage/provider/contract";
import { describe, expect, it } from "vitest";
import { AgentSession, MemoryLogStore } from "../../src/index.ts";
import {
  FAUX_SYSTEM as SYSTEM,
  scriptedSseFetch,
} from "../../src/testing/index.ts";

const OPENAI_PARTIAL = [
  'data: {"id":"c1","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{"content":"half-"},"finish_reason":null}]}',
  "",
  "",
].join("\n");

const OPENAI_OK = [
  'data: {"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{"content":"done"},"finish_reason":null}]}',
  "",
  'data: {"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}',
  "",
  'data: {"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[],"usage":{"prompt_tokens":2,"completion_tokens":1}}',
  "",
  "data: [DONE]",
  "",
  "",
].join("\n");

const ANTHROPIC_START = [
  "event: message_start",
  'data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}}',
  "",
  "event: content_block_start",
  'data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
  "",
];

// Ends after message_delta: the stop reason arrived, message_stop did not.
const ANTHROPIC_PARTIAL = [
  ...ANTHROPIC_START,
  "event: content_block_delta",
  'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"half-"}}',
  "",
  "event: content_block_stop",
  'data: {"type":"content_block_stop","index":0}',
  "",
  "event: message_delta",
  'data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}',
  "",
  "",
].join("\n");

const ANTHROPIC_OK = [
  ...ANTHROPIC_START,
  "event: content_block_delta",
  'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}}',
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

async function runWith(
  sessionId: string,
  model: { provider: string; model: string },
  adapter: ProviderAdapter,
) {
  const store = new MemoryLogStore();
  const session = new AgentSession({
    store,
    sessionId,
    adapters: { [model.provider]: adapter },
    model,
    system: SYSTEM,
    tools: [],
    sleep: async () => undefined,
  });
  await session.run("go");
  return store.path(sessionId);
}

function turnRetries(path: { kind: string; payload: unknown }[]): unknown[] {
  return path
    .filter((entry) => entry.kind === "turn_retry")
    .map((entry) => entry.payload);
}

const CUT_RETRY = {
  turn: 1,
  attempt: 1,
  reason: "stream ended before completion",
};

describe("a body that closes before the stream completes", () => {
  it("retries the turn when an OpenAI body closes without [DONE]", async () => {
    const { fetchImpl, calls } = scriptedSseFetch([OPENAI_PARTIAL, OPENAI_OK]);
    const path = await runWith(
      "openai-cut",
      { provider: "openai", model: "gpt-5" },
      new OpenAIAdapter({ apiKey: "k", fetchImpl, sleep: async () => {} }),
    );

    expect(calls()).toBe(2);
    expect(turnRetries(path)).toEqual([CUT_RETRY]);
    expect(path.at(-1)?.payload).toMatchObject({
      outcome: "done",
      health: { turnRetries: 1 },
    });
    expect(JSON.stringify(path)).not.toContain("half-");
  });

  it("retries the turn when an Anthropic body closes before message_stop", async () => {
    const { fetchImpl, calls } = scriptedSseFetch([
      ANTHROPIC_PARTIAL,
      ANTHROPIC_OK,
    ]);
    const path = await runWith(
      "anthropic-cut",
      { provider: "anthropic", model: "claude-sonnet-4-5" },
      new AnthropicAdapter({ apiKey: "k", fetchImpl, sleep: async () => {} }),
    );

    expect(calls()).toBe(2);
    expect(turnRetries(path)).toEqual([CUT_RETRY]);
    expect(path.at(-1)?.payload).toMatchObject({
      outcome: "done",
      health: { turnRetries: 1 },
    });
    expect(JSON.stringify(path)).not.toContain("half-");
  });

  it("retries before the stream when a 2xx response has no body", async () => {
    const { fetchImpl, calls } = scriptedSseFetch([
      { kind: "no-body" },
      OPENAI_OK,
    ]);
    const path = await runWith(
      "no-body",
      { provider: "openai", model: "gpt-5" },
      new OpenAIAdapter({ apiKey: "k", fetchImpl, sleep: async () => {} }),
    );

    expect(calls()).toBe(2);
    expect(turnRetries(path)).toEqual([]);
    expect(path.at(-1)?.payload).toMatchObject({ outcome: "done" });
  });
});
