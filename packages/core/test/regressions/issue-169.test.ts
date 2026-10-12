import { AnthropicAdapter, OpenAIAdapter } from "@sage/provider";
import { isRecord, type ProviderAdapter } from "@sage/provider/contract";
import { describe, expect, it } from "vitest";
import { AgentSession, MemoryLogStore } from "../../src/index.ts";
import {
  pathlessTool,
  FAUX_SYSTEM as SYSTEM,
} from "../../src/testing/index.ts";

type Wire = {
  name: string;
  model: { provider: string; model: string };
  make: (fetchImpl: typeof fetch) => ProviderAdapter;
  /** Streams partial text, then an error event whose `error` is `error`. */
  failing: (error: Record<string, unknown>) => string;
  /** Streams partial tool input, then an error event. */
  failingTool: (error: Record<string, unknown>) => string;
  ok: string;
};

const OPENAI: Wire = {
  name: "OpenAI",
  model: { provider: "openai", model: "gpt-5" },
  make: (fetchImpl) =>
    new OpenAIAdapter({ apiKey: "k", fetchImpl, sleep: async () => {} }),
  failing: (error) =>
    sse([
      '{"id":"c1","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{"content":"half-"},"finish_reason":null}]}',
      JSON.stringify({ error }),
    ]),
  failingTool: (error) =>
    sse([
      '{"id":"c1","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\\"q\\":\\"half-"}}]},"finish_reason":null}]}',
      JSON.stringify({ error }),
    ]),
  ok: sse([
    '{"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{"content":"done"},"finish_reason":null}]}',
    '{"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}',
    '{"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[],"usage":{"prompt_tokens":2,"completion_tokens":1}}',
    "[DONE]",
  ]),
};

const ANTHROPIC_START =
  'event: message_start\ndata: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}}';

const ANTHROPIC: Wire = {
  name: "Anthropic",
  model: { provider: "anthropic", model: "claude-sonnet-4-5" },
  make: (fetchImpl) =>
    new AnthropicAdapter({ apiKey: "k", fetchImpl, sleep: async () => {} }),
  failing: (error) =>
    events([
      ANTHROPIC_START,
      'event: content_block_start\ndata: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
      'event: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"half-"}}',
      `event: error\ndata: ${JSON.stringify({ type: "error", error })}`,
    ]),
  failingTool: (error) =>
    events([
      ANTHROPIC_START,
      'event: content_block_start\ndata: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"lookup","input":{}}}',
      'event: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\\"q\\":\\"half-"}}',
      `event: error\ndata: ${JSON.stringify({ type: "error", error })}`,
    ]),
  ok: events([
    'event: message_start\ndata: {"type":"message_start","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"stop_reason":null,"usage":{"input_tokens":2,"output_tokens":1}}}',
    'event: content_block_start\ndata: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
    'event: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}}',
    'event: content_block_stop\ndata: {"type":"content_block_stop","index":0}',
    'event: message_delta\ndata: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}',
    'event: message_stop\ndata: {"type":"message_stop"}',
  ]),
};

const WIRES = [OPENAI, ANTHROPIC];

const RATE_LIMITED = {
  message: "Upstream rate limit exceeded, please retry later",
};
const STALLED = { message: "Upstream response stream stalled" };

/** A permanent error event per wire, classified by its `type` or `code`. */
const PERMANENT: Record<string, Record<string, unknown>> = {
  OpenAI: {
    message: "too long",
    type: "invalid_request_error",
    code: "context_length_exceeded",
  },
  Anthropic: { message: "bad", type: "invalid_request_error" },
};

function sse(frames: string[]): string {
  return `${frames.map((frame) => `data: ${frame}`).join("\n\n")}\n\n`;
}

function events(blocks: string[]): string {
  return `${blocks.join("\n\n")}\n\n`;
}

/** Answers each fetch with the next body as a 200 SSE response. */
function scriptedFetch(bodies: string[]): {
  fetchImpl: typeof fetch;
  calls: () => number;
} {
  let count = 0;
  return {
    fetchImpl: async () => {
      const body = bodies[Math.min(count, bodies.length - 1)];
      count += 1;
      return new Response(body, {
        status: 200,
        headers: { "content-type": "text/event-stream" },
      });
    },
    calls: () => count,
  };
}

async function runWith(wire: Wire, bodies: string[]) {
  const store = new MemoryLogStore();
  const { fetchImpl, calls } = scriptedFetch(bodies);
  const waits: number[] = [];
  const sessionId = `${wire.name}-issue-169`;
  const session = new AgentSession({
    store,
    sessionId,
    adapters: { [wire.model.provider]: wire.make(fetchImpl) },
    model: wire.model,
    system: SYSTEM,
    tools: [pathlessTool("lookup")],
    sleep: async (ms: number) => {
      waits.push(ms);
    },
  });
  await session.run("go");
  return { path: store.path(sessionId), calls: calls(), waits };
}

type PathEntry = { kind: string; payload: unknown; details?: unknown };

function turnRetries(path: PathEntry[]): unknown[] {
  return path
    .filter((entry) => entry.kind === "turn_retry")
    .map((entry) => entry.payload);
}

function assistants(path: PathEntry[]): PathEntry[] {
  return path.filter(
    (entry) =>
      entry.kind === "message" &&
      isRecord(entry.payload) &&
      entry.payload.role === "assistant",
  );
}

describe("a transient error event inside a 200 stream", () => {
  for (const wire of WIRES) {
    it(`resends the turn and keeps the partial text out of the Log (${wire.name})`, async () => {
      const { path, calls, waits } = await runWith(wire, [
        wire.failing(RATE_LIMITED),
        wire.ok,
      ]);

      expect(calls).toBe(2);
      expect(waits).toEqual([2000]);
      expect(turnRetries(path)).toEqual([
        { turn: 1, attempt: 1, reason: RATE_LIMITED.message },
      ]);
      expect(assistants(path)).toHaveLength(1);
      expect(JSON.stringify(path)).not.toContain("half-");
      expect(path.at(-1)?.payload).toMatchObject({
        outcome: "done",
        health: { turnRetries: 1 },
      });
    });

    it(`resends the turn without running a half-streamed tool call (${wire.name})`, async () => {
      const { path, calls } = await runWith(wire, [
        wire.failingTool(STALLED),
        wire.ok,
      ]);

      expect(calls).toBe(2);
      expect(turnRetries(path)).toEqual([
        { turn: 1, attempt: 1, reason: STALLED.message },
      ]);
      expect(assistants(path)).toHaveLength(1);
      expect(JSON.stringify(path)).not.toContain("half-");
      expect(JSON.stringify(path)).not.toContain("tool_result");
      expect(path.at(-1)?.payload).toMatchObject({ outcome: "done" });
    });

    it(`ends the run as an error after the whole-turn retries run out (${wire.name})`, async () => {
      const { path, calls, waits } = await runWith(wire, [
        wire.failing(STALLED),
      ]);

      expect(calls).toBe(3);
      expect(waits).toEqual([2000, 8000]);
      expect(turnRetries(path)).toHaveLength(2);
      expect(assistants(path)).toHaveLength(1);
      expect(path.at(-1)?.payload).toMatchObject({
        outcome: "error",
        health: { turnRetries: 2 },
      });
      const last = assistants(path)[0];
      expect(isRecord(last?.details) ? last.details.finish : undefined).toEqual(
        {
          type: "finish",
          kind: "error",
          error: {
            message: STALLED.message,
            raw: expect.any(Object),
            event: { transient: true },
          },
        },
      );
    });
  }
});

describe("a permanent error event inside a 200 stream", () => {
  for (const wire of WIRES) {
    it(`ends the run on the first call without a turn retry (${wire.name})`, async () => {
      const permanent = PERMANENT[wire.name] ?? {};
      const { path, calls } = await runWith(wire, [
        wire.failing(permanent),
        wire.ok,
      ]);

      expect(calls).toBe(1);
      expect(turnRetries(path)).toEqual([]);
      expect(path.at(-1)?.payload).toMatchObject({
        outcome: "error",
        health: { turnRetries: 0 },
      });
    });
  }
});
