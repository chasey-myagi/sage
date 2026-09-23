import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { AnthropicAdapter } from "../src/anthropic/index.ts";
import type { Message } from "../src/contract/index.ts";
import { normalizeForApi } from "../src/normalize.ts";
import { OpenAIAdapter } from "../src/openai/index.ts";
import { fixtureFetch, readFixture } from "./support.ts";

function callId(id: string): string {
  return `call_${createHash("sha256").update(id).digest("hex").slice(0, 24)}`;
}

function crossMessages(): Message[] {
  return [
    {
      role: "assistant",
      content: [
        { type: "thinking", text: "SIGNED", signature: "sig_keep" },
        { type: "thinking", text: "UNSIGNED" },
        { type: "text", text: "keep-me" },
        {
          type: "tool_use",
          id: "toolu_abc",
          name: "read",
          input: { path: ".env" },
          rawInput: '{"path":".env"}',
        },
        {
          type: "tool_use",
          id: "call_already",
          name: "read",
          input: {},
          rawInput: "{}",
        },
      ],
    },
    {
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: "toolu_abc",
          content: "secret",
          isError: false,
        },
        {
          type: "tool_result",
          toolUseId: "call_already",
          content: "ok",
          isError: false,
        },
      ],
    },
  ];
}

describe("normalizeForApi reshapes messages for the target api", () => {
  it("rewrites Anthropic tool ids and drops thinking for OpenAI", () => {
    const messages = crossMessages();
    const snapshot = structuredClone(messages);
    const normalized = normalizeForApi(messages, "openai-chat");
    expect(messages).toEqual(snapshot);
    const expected = callId("toolu_abc");
    expect(expected).toMatch(/^call_[0-9a-f]{24}$/);
    expect(normalized[0]?.content).toEqual([
      { type: "text", text: "keep-me" },
      {
        type: "tool_use",
        id: expected,
        name: "read",
        input: { path: ".env" },
        rawInput: '{"path":".env"}',
      },
      {
        type: "tool_use",
        id: "call_already",
        name: "read",
        input: {},
        rawInput: "{}",
      },
    ]);
    expect(normalized[1]?.content).toEqual([
      {
        type: "tool_result",
        toolUseId: expected,
        content: "secret",
        isError: false,
      },
      {
        type: "tool_result",
        toolUseId: "call_already",
        content: "ok",
        isError: false,
      },
    ]);
    expect(JSON.stringify(normalized)).not.toContain("SIGNED");
    expect(JSON.stringify(normalized)).not.toContain("UNSIGNED");
  });

  it("drops unsigned thinking for Anthropic and leaves the rest", () => {
    const messages = crossMessages();
    const normalized = normalizeForApi(messages, "anthropic-messages");
    expect(normalized[0]?.content).toEqual([
      { type: "thinking", text: "SIGNED", signature: "sig_keep" },
      { type: "text", text: "keep-me" },
      {
        type: "tool_use",
        id: "toolu_abc",
        name: "read",
        input: { path: ".env" },
        rawInput: '{"path":".env"}',
      },
      {
        type: "tool_use",
        id: "call_already",
        name: "read",
        input: {},
        rawInput: "{}",
      },
    ]);
    expect(normalized[1]?.content[0]).toMatchObject({ toolUseId: "toolu_abc" });
  });

  it("encodes the rewritten ids on the OpenAI wire and drops thinking", async () => {
    const { fetchImpl, calls } = fixtureFetch(
      readFixture("openai-text-tool.sse"),
    );
    const adapter = new OpenAIAdapter({ apiKey: "test-key", fetchImpl });
    const messages = crossMessages();
    await drain(
      adapter.stream(
        {
          model: { provider: "openai", model: "gpt-5.4-mini" },
          head: { system: "sys", tools: [] },
          messages,
          cacheBreakpoints: [],
          sampling: { maxTokens: 8 },
        },
        adapter.prepare({ provider: "openai", model: "gpt-5.4-mini" }),
      ),
    );
    const body = JSON.stringify(calls[0]?.body);
    const expected = callId("toolu_abc");
    expect(body).toContain(expected);
    expect(body).not.toContain("toolu_abc");
    expect(body).not.toContain("SIGNED");
    expect(body).not.toContain("UNSIGNED");
    expect(body).toContain("keep-me");
  });

  it("drops unsigned thinking on the Anthropic wire and keeps signed blocks", async () => {
    const { fetchImpl, calls } = fixtureFetch(
      readFixture("anthropic-text-tool.sse"),
    );
    const adapter = new AnthropicAdapter({ apiKey: "test-key", fetchImpl });
    await drain(
      adapter.stream(
        {
          model: { provider: "anthropic", model: "claude-sonnet-5" },
          head: { system: "sys", tools: [] },
          messages: crossMessages(),
          cacheBreakpoints: [],
          sampling: { maxTokens: 8 },
        },
        adapter.prepare({ provider: "anthropic", model: "claude-sonnet-5" }),
      ),
    );
    const body = JSON.stringify(calls[0]?.body);
    expect(body).toContain("SIGNED");
    expect(body).toContain("sig_keep");
    expect(body).not.toContain("UNSIGNED");
    expect(body).toContain("toolu_abc");
    expect(body).toContain("keep-me");
  });
});

async function drain(chunks: AsyncIterable<unknown>): Promise<void> {
  for await (const _chunk of chunks) {
    // The wire body is captured by fixtureFetch before the stream ends.
  }
}
