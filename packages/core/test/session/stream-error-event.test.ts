import { AnthropicAdapter, OpenAIAdapter } from "@sage/provider";
import { isRecord, type ProviderAdapter } from "@sage/provider/contract";
import { describe, expect, it } from "vitest";
import { AgentSession, MemoryLogStore } from "../../src/index.ts";
import { FAUX_SYSTEM as SYSTEM } from "../../src/testing/index.ts";

// The server reports an error inside a 200 response, then closes.
const OPENAI_ERROR = [
  'data: {"id":"c1","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{"content":"half-"},"finish_reason":null}]}',
  "",
  'data: {"error":{"message":"Overloaded","type":"server_error","code":null}}',
  "",
  "",
].join("\n");

const ANTHROPIC_ERROR = [
  "event: message_start",
  'data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}}',
  "",
  "event: content_block_start",
  'data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
  "",
  "event: content_block_delta",
  'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"half-"}}',
  "",
  "event: error",
  'data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}',
  "",
  "",
].join("\n");

/** Every fetch answers 200 with `body`; counts the calls. */
function repeatingFetch(body: string): {
  fetchImpl: typeof fetch;
  calls: () => number;
} {
  let count = 0;
  return {
    fetchImpl: async () => {
      count += 1;
      return new Response(body, {
        status: 200,
        headers: { "content-type": "text/event-stream" },
      });
    },
    calls: () => count,
  };
}

async function runWith(
  sessionId: string,
  model: { provider: string; model: string },
  adapter: ProviderAdapter,
) {
  const store = new MemoryLogStore();
  const session = new AgentSession({
    store,
    sessionId,
    adapter,
    model,
    system: SYSTEM,
    tools: [],
    sleep: async () => undefined,
  });
  await session.run("go");
  return store.path(sessionId);
}

type PathEntry = { kind: string; payload: unknown; details?: unknown };

function assistantFinishes(path: PathEntry[]): unknown[] {
  const finishes: unknown[] = [];
  for (const entry of path) {
    if (entry.kind !== "message" || !isRecord(entry.payload)) continue;
    if (entry.payload.role !== "assistant" || !isRecord(entry.details))
      continue;
    finishes.push(entry.details.finish);
  }
  return finishes;
}

const CASES: {
  name: string;
  model: { provider: string; model: string };
  make: (fetchImpl: typeof fetch) => ProviderAdapter;
  body: string;
}[] = [
  {
    name: "OpenAI",
    model: { provider: "openai", model: "gpt-5" },
    make: (fetchImpl) =>
      new OpenAIAdapter({ apiKey: "k", fetchImpl, sleep: async () => {} }),
    body: OPENAI_ERROR,
  },
  {
    name: "Anthropic",
    model: { provider: "anthropic", model: "claude-sonnet-4-5" },
    make: (fetchImpl) =>
      new AnthropicAdapter({ apiKey: "k", fetchImpl, sleep: async () => {} }),
    body: ANTHROPIC_ERROR,
  },
];

describe("an error event inside the stream", () => {
  for (const item of CASES) {
    it(`ends the run with the server's message and no turn retry (${item.name})`, async () => {
      const { fetchImpl, calls } = repeatingFetch(item.body);
      const path = await runWith(
        `${item.name}-error-event`,
        item.model,
        item.make(fetchImpl),
      );

      expect(path.filter((entry) => entry.kind === "turn_retry")).toEqual([]);
      expect(calls()).toBe(1);
      expect(path.at(-1)?.payload).toMatchObject({
        outcome: "error",
        health: { turnRetries: 0 },
      });
      const finishes = assistantFinishes(path);
      expect(finishes).toHaveLength(1);
      expect(finishes[0]).toMatchObject({
        kind: "error",
        error: { message: "Overloaded" },
      });
      expect(finishes[0]).not.toHaveProperty("error.transport");
    });
  }
});
