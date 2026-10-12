import { AnthropicAdapter, OpenAIAdapter } from "@sage/provider";
import { isRecord, type ProviderAdapter } from "@sage/provider/contract";
import { describe, expect, it } from "vitest";
import { AgentSession, MemoryLogStore } from "../../src/index.ts";
import {
  pathlessTool,
  FAUX_SYSTEM as SYSTEM,
  scriptedSseFetch,
} from "../../src/testing/index.ts";

const MALFORMED = '{"q":"half';

type Wire = {
  name: string;
  model: { provider: string; model: string };
  make: (fetchImpl: typeof fetch) => ProviderAdapter;
  /** Ends normally with one `lookup` call whose arguments are `MALFORMED`. */
  malformedTool: string;
  ok: string;
  /** The replayed arguments of every tool call in a request body. */
  replayedArguments: (body: Record<string, unknown>) => unknown[];
};

const OPENAI: Wire = {
  name: "OpenAI",
  model: { provider: "openai", model: "gpt-5" },
  make: (fetchImpl) =>
    new OpenAIAdapter({ apiKey: "k", fetchImpl, sleep: async () => {} }),
  malformedTool: sse([
    JSON.stringify({
      id: "c1",
      object: "chat.completion.chunk",
      model: "gpt-5",
      choices: [
        {
          index: 0,
          delta: {
            tool_calls: [
              {
                index: 0,
                id: "call_1",
                type: "function",
                function: { name: "lookup", arguments: MALFORMED },
              },
            ],
          },
          finish_reason: null,
        },
      ],
    }),
    '{"id":"c1","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}',
    '{"id":"c1","object":"chat.completion.chunk","model":"gpt-5","choices":[],"usage":{"prompt_tokens":2,"completion_tokens":1}}',
    "[DONE]",
  ]),
  ok: sse([
    '{"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{"content":"done"},"finish_reason":null}]}',
    '{"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}',
    '{"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[],"usage":{"prompt_tokens":2,"completion_tokens":1}}',
    "[DONE]",
  ]),
  replayedArguments: (body) =>
    records(body.messages).flatMap((message) =>
      records(message.tool_calls).map((call) => {
        const fn = isRecord(call.function) ? call.function : {};
        return typeof fn.arguments === "string"
          ? parseJson(fn.arguments)
          : undefined;
      }),
    ),
};

const ANTHROPIC: Wire = {
  name: "Anthropic",
  model: { provider: "anthropic", model: "claude-sonnet-4-5" },
  make: (fetchImpl) =>
    new AnthropicAdapter({ apiKey: "k", fetchImpl, sleep: async () => {} }),
  malformedTool: events([
    'event: message_start\ndata: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}}',
    'event: content_block_start\ndata: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"lookup","input":{}}}',
    `event: content_block_delta\ndata: ${JSON.stringify({
      type: "content_block_delta",
      index: 0,
      delta: { type: "input_json_delta", partial_json: MALFORMED },
    })}`,
    'event: content_block_stop\ndata: {"type":"content_block_stop","index":0}',
    'event: message_delta\ndata: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":1}}',
    'event: message_stop\ndata: {"type":"message_stop"}',
  ]),
  ok: events([
    'event: message_start\ndata: {"type":"message_start","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"stop_reason":null,"usage":{"input_tokens":2,"output_tokens":1}}}',
    'event: content_block_start\ndata: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
    'event: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}}',
    'event: content_block_stop\ndata: {"type":"content_block_stop","index":0}',
    'event: message_delta\ndata: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}',
    'event: message_stop\ndata: {"type":"message_stop"}',
  ]),
  replayedArguments: (body) =>
    records(body.messages).flatMap((message) =>
      records(message.content)
        .filter((block) => block.type === "tool_use")
        .map((block) => block.input),
    ),
};

const WIRES = [OPENAI, ANTHROPIC];

function sse(frames: string[]): string {
  return `${frames.map((frame) => `data: ${frame}`).join("\n\n")}\n\n`;
}

function events(blocks: string[]): string {
  return `${blocks.join("\n\n")}\n\n`;
}

function records(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.filter(isRecord) : [];
}

/** `text` parsed, or the text itself when it is not JSON. */
function parseJson(text: string): unknown {
  try {
    return JSON.parse(text) as unknown;
  } catch {
    // The raw text is the evidence the assertion reports on failure.
    return text;
  }
}

async function runWith(wire: Wire) {
  const store = new MemoryLogStore();
  const { fetchImpl, requestBodies } = scriptedSseFetch([
    wire.malformedTool,
    wire.ok,
  ]);
  const sessionId = `${wire.name}-issue-172`;
  const session = new AgentSession({
    store,
    sessionId,
    adapters: { [wire.model.provider]: wire.make(fetchImpl) },
    model: wire.model,
    system: SYSTEM,
    tools: [pathlessTool("lookup")],
    sleep: async () => {},
  });
  await session.run("go");
  return {
    path: store.path(sessionId),
    requests: requestBodies().map((body) => {
      const parsed = parseJson(body);
      return isRecord(parsed) ? parsed : {};
    }),
  };
}

type PathEntry = { kind: string; payload: unknown };

function toolResults(path: PathEntry[]): Record<string, unknown>[] {
  return path.flatMap((entry) =>
    entry.kind === "message" && isRecord(entry.payload)
      ? records(entry.payload.content).filter(
          (block) => block.type === "tool_result",
        )
      : [],
  );
}

describe("a tool call whose streamed arguments are not valid JSON", () => {
  for (const wire of WIRES) {
    it(`replays the call with an empty JSON object as its arguments (${wire.name})`, async () => {
      const { requests } = await runWith(wire);

      expect(requests).toHaveLength(2);
      expect(wire.replayedArguments(requests[1] ?? {})).toEqual([{}]);
    });

    it(`answers the call with an error naming the unparsable arguments and the run ends done (${wire.name})`, async () => {
      const { path } = await runWith(wire);

      expect(JSON.stringify(path)).toContain(JSON.stringify(MALFORMED));
      expect(toolResults(path)).toEqual([
        {
          type: "tool_result",
          toolUseId: expect.any(String),
          content: expect.stringContaining("not a JSON object"),
          isError: true,
        },
      ]);
      expect(String(toolResults(path)[0]?.content)).toContain(MALFORMED);
      expect(path.at(-1)?.payload).toMatchObject({ outcome: "done" });
    });
  }
});
