import { describe, expect, it } from "vitest";
import { AnthropicAdapter } from "../src/anthropic/index.ts";
import { collect } from "../src/contract/collect.ts";
import type { StreamChunk } from "../src/contract/index.ts";
import { OpenAIAdapter } from "../src/openai/index.ts";
import { drain, fixtureFetch, quadruple, readFixture } from "./support.ts";

async function* from(chunks: StreamChunk[]): AsyncIterable<StreamChunk> {
  for (const chunk of chunks) yield chunk;
}

const usage = {
  inputOther: 10,
  inputCacheRead: 2,
  inputCacheCreation: 0,
  output: 12,
};
const finish = { type: "finish" as const, kind: "tool_use" as const };

describe("scenario 7 collect", () => {
  it("assembles blocks, parses tool input, and copies the envelope", async () => {
    const message = await collect(
      from([
        { type: "response", id: "msg_01", model: "claude-sonnet-4-5" },
        { type: "block_start", index: 0, blockType: "text" },
        { type: "text_delta", index: 0, text: "H" },
        { type: "text_delta", index: 0, text: "i" },
        { type: "block_end", index: 0 },
        { type: "block_start", index: 1, blockType: "thinking" },
        { type: "thinking_delta", index: 1, text: "ab" },
        { type: "block_end", index: 1 },
        {
          type: "block_start",
          index: 2,
          blockType: "tool_use",
          id: "toolu_1",
          name: "lookup",
        },
        { type: "tool_input_delta", index: 2, partialJson: "{" },
        { type: "tool_input_delta", index: 2, partialJson: '"q":1}' },
        { type: "block_end", index: 2 },
        { type: "usage", usage },
        finish,
      ]),
      { api: "anthropic-messages" },
    );
    expect(message).toEqual({
      role: "assistant",
      content: [
        { type: "text", text: "Hi" },
        { type: "thinking", text: "ab" },
        {
          type: "tool_use",
          id: "toolu_1",
          name: "lookup",
          input: { q: 1 },
          rawInput: '{"q":1}',
        },
      ],
      usage,
      finish,
      responseModel: "claude-sonnet-4-5",
      responseId: "msg_01",
      api: "anthropic-messages",
    });
  });

  it("keeps rawInput and sets input to null when JSON.parse fails", async () => {
    const message = await collect(
      from([
        {
          type: "block_start",
          index: 0,
          blockType: "tool_use",
          id: "toolu_1",
          name: "lookup",
        },
        { type: "tool_input_delta", index: 0, partialJson: "{" },
        { type: "block_end", index: 0 },
        { type: "usage", usage },
        finish,
      ]),
      { api: "openai-chat" },
    );
    expect(message.content).toEqual([
      {
        type: "tool_use",
        id: "toolu_1",
        name: "lookup",
        input: null,
        rawInput: "{",
      },
    ]);
  });

  it("leaves response identity empty when no response chunk arrives", async () => {
    const message = await collect(
      from([
        { type: "usage", usage },
        { type: "finish", kind: "error", error: { message: "HTTP 500" } },
      ]),
      { api: "openai-chat" },
    );
    expect(message.responseId).toBe("");
    expect(message.responseModel).toBe("");
  });

  it("collects an Anthropic stream into one assistant message", async () => {
    const { fetchImpl } = fixtureFetch(readFixture("anthropic-text-tool.sse"));
    const adapter = new AnthropicAdapter({ apiKey: "k", fetchImpl });
    const model = { provider: "anthropic", model: "claude-sonnet-4-5" };
    const stream = adapter.stream(
      {
        model,
        head: { system: "sys", tools: [] },
        messages: [{ role: "user", content: [{ type: "text", text: "hi" }] }],
        cacheBreakpoints: [],
        sampling: { maxTokens: 16 },
      },
      adapter.prepare(model),
    );
    const message = await collect(stream, { api: adapter.api });
    expect(message.role).toBe("assistant");
    expect(message.api).toBe("anthropic-messages");
    expect(message.responseId).toBe("msg_01");
    expect(message.responseModel).toBe("claude-sonnet-4-5");
    expect(message.content).toEqual([
      { type: "text", text: "Hi" },
      {
        type: "tool_use",
        id: "toolu_1",
        name: "lookup",
        input: { q: "x" },
        rawInput: '{"q":"x"}',
      },
    ]);
    expect(quadruple(message.usage)).toEqual(usage);
    expect(message.finish).toEqual({ type: "finish", kind: "tool_use" });
    expect({
      responseId: message.responseId,
      responseModel: message.responseModel,
    }).toEqual({
      responseId: "msg_01",
      responseModel: "claude-sonnet-4-5",
    });
  });

  it("collects an OpenAI stream into the same content shape", async () => {
    const { fetchImpl } = fixtureFetch(readFixture("openai-text-tool.sse"));
    const adapter = new OpenAIAdapter({ apiKey: "k", fetchImpl });
    const model = { provider: "openai", model: "gpt-5" };
    const stream = adapter.stream(
      {
        model,
        head: { system: "sys", tools: [] },
        messages: [{ role: "user", content: [{ type: "text", text: "hi" }] }],
        cacheBreakpoints: [],
        sampling: { maxTokens: 16 },
      },
      adapter.prepare(model),
    );
    const message = await collect(stream, { api: adapter.api });
    expect(message.content).toEqual([
      { type: "text", text: "Hi" },
      {
        type: "tool_use",
        id: "toolu_1",
        name: "lookup",
        input: { q: "x" },
        rawInput: '{"q":"x"}',
      },
    ]);
    expect(quadruple(message.usage)).toEqual(usage);
    expect(message.finish).toEqual({ type: "finish", kind: "tool_use" });
    expect(message.api).toBe("openai-chat");
    expect({
      responseId: message.responseId,
      responseModel: message.responseModel,
    }).toEqual({
      responseId: "chatcmpl-abc",
      responseModel: "gpt-5",
    });
    await drain(from([]));
  });
});
