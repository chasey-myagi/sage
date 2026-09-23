import { describe, expect, it } from "vitest";
import { AnthropicAdapter } from "../src/anthropic/index.ts";
import { collect } from "../src/contract/collect.ts";
import type { Request } from "../src/contract/index.ts";
import { OpenAIAdapter } from "../src/openai/index.ts";
import { expectStreamInvariants, runContractSuite } from "./contract-suite.ts";
import { drain, fixtureFetch, readFixture, stripRaw } from "./support.ts";

const schemaA = { type: "object", properties: { q: { type: "string" } } };
const schemaB = { type: "object" };

function cacheRequest(provider: string, model: string): Request {
  return {
    model: { provider, model },
    head: {
      system: "You are sage.",
      tools: [
        { name: "a", description: "A", inputSchema: schemaA },
        { name: "b", description: "B", inputSchema: schemaB },
      ],
    },
    messages: [
      { role: "user", content: [{ type: "text", text: "m0" }] },
      {
        role: "assistant",
        content: [
          { type: "thinking", text: "THINK_SECRET", signature: "sig_1" },
          { type: "text", text: "m1" },
          {
            type: "tool_use",
            id: "t1",
            name: "a",
            input: { q: 1 },
            rawInput: '{"q":1}',
          },
        ],
      },
      {
        role: "user",
        content: [
          {
            type: "tool_result",
            toolUseId: "t1",
            content: "ok",
            isError: true,
          },
        ],
      },
      {
        role: "user",
        content: [
          { type: "text", text: "m3a" },
          { type: "text", text: "m3b" },
        ],
      },
    ],
    cacheBreakpoints: ["head", { afterMessage: 3 }],
    sampling: { temperature: 0 },
  };
}

function cacheControlPaths(value: unknown, path: string, acc: string[]): void {
  if (Array.isArray(value)) {
    for (let index = 0; index < value.length; index += 1) {
      cacheControlPaths(value[index], `${path}[${index}]`, acc);
    }
    return;
  }
  if (value === null || typeof value !== "object") return;
  const record = value as Record<string, unknown>;
  if ("cache_control" in record) acc.push(path);
  for (const [key, child] of Object.entries(record)) {
    if (key === "cache_control") continue;
    const next = path.length === 0 ? key : `${path}.${key}`;
    cacheControlPaths(child, next, acc);
  }
}

function keysContaining(value: unknown, needle: string, acc: string[]): void {
  if (Array.isArray(value)) {
    for (const item of value) keysContaining(item, needle, acc);
    return;
  }
  if (value === null || typeof value !== "object") return;
  for (const [key, child] of Object.entries(value)) {
    if (key.toLowerCase().includes(needle)) acc.push(key);
    keysContaining(child, needle, acc);
  }
}

function asRecord(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("expected object");
  }
  return value as Record<string, unknown>;
}

function anthropicStop(reason: string): string {
  return [
    "event: message_start",
    'data: {"type":"message_start","message":{"id":"msg_s","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}}',
    "",
    "event: message_delta",
    `data: {"type":"message_delta","delta":{"stop_reason":${JSON.stringify(reason)}},"usage":{"output_tokens":2}}`,
    "",
    "event: message_stop",
    'data: {"type":"message_stop"}',
    "",
  ].join("\n");
}

function openaiStop(reason: string): string {
  return [
    `data: {"id":"chatcmpl-s","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{},"finish_reason":${JSON.stringify(reason)}}]}`,
    "",
    'data: {"id":"chatcmpl-s","object":"chat.completion.chunk","model":"gpt-5","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":1}}}',
    "",
    "data: [DONE]",
    "",
  ].join("\n");
}

runContractSuite(
  "anthropic-messages",
  (fetchImpl) => new AnthropicAdapter({ apiKey: "test-key", fetchImpl }),
  {
    happy: readFixture("anthropic-text-tool.sse"),
    truncated: readFixture("anthropic-truncated.sse"),
    response: { type: "response", id: "msg_01", model: "claude-sonnet-4-5" },
  },
);

runContractSuite(
  "openai-chat",
  (fetchImpl) => new OpenAIAdapter({ apiKey: "test-key", fetchImpl }),
  {
    happy: readFixture("openai-text-tool.sse"),
    truncated: readFixture("openai-truncated.sse"),
    response: { type: "response", id: "chatcmpl-abc", model: "gpt-5" },
  },
);

describe("scenario 1 cross-adapter", () => {
  it("matches chunk for chunk after raw is removed", async () => {
    const left = fixtureFetch(readFixture("anthropic-text-tool.sse"));
    const right = fixtureFetch(readFixture("openai-text-tool.sse"));
    const anthropic = new AnthropicAdapter({
      apiKey: "test-key",
      fetchImpl: left.fetchImpl,
    });
    const openai = new OpenAIAdapter({
      apiKey: "test-key",
      fetchImpl: right.fetchImpl,
    });
    const model = { provider: "anthropic", model: "claude-sonnet-4-5" };
    const req: Request = {
      model,
      head: { system: "sys", tools: [] },
      messages: [{ role: "user", content: [{ type: "text", text: "hi" }] }],
      cacheBreakpoints: [],
      sampling: { maxTokens: 16 },
    };
    const aChunks = await drain(
      anthropic.stream(req, anthropic.prepare(model)),
    );
    const oChunks = await drain(openai.stream(req, openai.prepare(model)));
    // Response identity is per provider by nature; every other chunk matches.
    expect(aChunks[0]).toEqual({
      type: "response",
      id: "msg_01",
      model: "claude-sonnet-4-5",
    });
    expect(oChunks[0]).toEqual({
      type: "response",
      id: "chatcmpl-abc",
      model: "gpt-5",
    });
    expect(stripRaw(aChunks.slice(1))).toEqual(stripRaw(oChunks.slice(1)));
  });
});

describe("scenario 3 cache breakpoints", () => {
  it("paints ephemeral cache_control only on the three requested spots", async () => {
    const { fetchImpl, calls } = fixtureFetch(
      readFixture("anthropic-text-tool.sse"),
    );
    const adapter = new AnthropicAdapter({
      apiKey: "test-key",
      fetchImpl,
    });
    const req = cacheRequest("anthropic", "claude-sonnet-5");
    await drain(adapter.stream(req, adapter.prepare(req.model)));
    const call = calls[0];
    expect(call?.method).toBe("POST");
    expect(call?.url).toBe("https://api.anthropic.com/v1/messages");
    expect(call?.headers.get("x-api-key")).toBe("test-key");
    expect(call?.headers.get("anthropic-version")).toBe("2023-06-01");
    const body = asRecord(call?.body);
    expect(body.stream).toBe(true);
    expect(body.max_tokens).toBe(8192);
    expect(body.temperature).toBe(0);
    const system = body.system as { text: string; cache_control?: unknown }[];
    expect(system.map((block) => block.text)).toEqual(["You are sage."]);
    expect(system[0]?.cache_control).toEqual({ type: "ephemeral" });
    const tools = body.tools as {
      name: string;
      input_schema: unknown;
      cache_control?: unknown;
    }[];
    expect(tools.map((tool) => tool.name)).toEqual(["a", "b"]);
    expect(tools[0]?.input_schema).toEqual(schemaA);
    expect(tools[0]?.cache_control).toBeUndefined();
    expect(tools[1]?.cache_control).toEqual({ type: "ephemeral" });
    const messages = body.messages as {
      role: string;
      content: Record<string, unknown>[];
    }[];
    expect(messages).toHaveLength(4);
    expect(messages[1]?.content[0]).toEqual({
      type: "thinking",
      thinking: "THINK_SECRET",
      signature: "sig_1",
    });
    expect(messages[1]?.content[2]).toEqual({
      type: "tool_use",
      id: "t1",
      name: "a",
      input: { q: 1 },
    });
    expect(messages[2]?.content[0]).toEqual({
      type: "tool_result",
      tool_use_id: "t1",
      content: "ok",
      is_error: true,
    });
    expect(messages[3]?.content[0]?.cache_control).toBeUndefined();
    expect(messages[3]?.content[1]?.cache_control).toEqual({
      type: "ephemeral",
    });
    const paths: string[] = [];
    cacheControlPaths(body, "", paths);
    expect(paths.sort()).toEqual([
      "messages[3].content[1]",
      "system[0]",
      "tools[1]",
    ]);
  });

  it("leaves every cache field out of the OpenAI body", async () => {
    const { fetchImpl, calls } = fixtureFetch(
      readFixture("openai-text-tool.sse"),
    );
    const adapter = new OpenAIAdapter({ apiKey: "test-key", fetchImpl });
    const req = cacheRequest("openai", "gpt-5.4");
    await drain(adapter.stream(req, adapter.prepare(req.model)));
    const call = calls[0];
    expect(call?.method).toBe("POST");
    expect(call?.url).toBe("https://api.openai.com/v1/chat/completions");
    expect(call?.headers.get("authorization")).toBe("Bearer test-key");
    const body = asRecord(call?.body);
    expect(body.stream).toBe(true);
    expect(body.stream_options).toEqual({ include_usage: true });
    expect(body.max_tokens).toBe(8192);
    expect(body.temperature).toBe(0);
    expect(body.model).toBe("gpt-5.4");
    const tools = body.tools as {
      type: string;
      function: { name: string; description: string; parameters: unknown };
    }[];
    expect(tools[0]).toEqual({
      type: "function",
      function: { name: "a", description: "A", parameters: schemaA },
    });
    const messages = body.messages as Record<string, unknown>[];
    expect(messages[0]).toEqual({ role: "system", content: "You are sage." });
    expect(messages[1]).toEqual({ role: "user", content: "m0" });
    expect(messages[2]).toEqual({
      role: "assistant",
      content: "m1",
      tool_calls: [
        {
          id: "t1",
          type: "function",
          function: { name: "a", arguments: '{"q":1}' },
        },
      ],
    });
    expect(messages[3]).toEqual({
      role: "tool",
      tool_call_id: "t1",
      content: "ok",
    });
    expect(messages[4]).toEqual({ role: "user", content: "m3am3b" });
    expect(JSON.stringify(body)).not.toContain("THINK_SECRET");
    const cacheKeys: string[] = [];
    keysContaining(body, "cache", cacheKeys);
    expect(cacheKeys).toEqual([]);
  });

  it("uses a caller baseUrl without reading an ambient key", async () => {
    const anthropicFetch = fixtureFetch(readFixture("anthropic-text-tool.sse"));
    const openaiFetch = fixtureFetch(readFixture("openai-text-tool.sse"));
    const anthropic = new AnthropicAdapter({
      apiKey: "caller-key",
      baseUrl: "https://anthropic.example/proxy/",
      fetchImpl: anthropicFetch.fetchImpl,
    });
    const openai = new OpenAIAdapter({
      apiKey: "caller-key",
      baseUrl: "https://openai.example/v1",
      fetchImpl: openaiFetch.fetchImpl,
    });
    const model = { provider: "anthropic", model: "claude-sonnet-4-5" };
    const req = cacheRequest(model.provider, model.model);
    await drain(anthropic.stream(req, anthropic.prepare(model)));
    await drain(
      openai.stream(
        { ...req, model: { provider: "openai", model: "gpt-5" } },
        openai.prepare({ provider: "openai", model: "gpt-5" }),
      ),
    );
    expect(anthropicFetch.calls[0]?.url).toBe(
      "https://anthropic.example/proxy/v1/messages",
    );
    expect(anthropicFetch.calls[0]?.headers.get("x-api-key")).toBe(
      "caller-key",
    );
    expect(openaiFetch.calls[0]?.url).toBe(
      "https://openai.example/v1/chat/completions",
    );
    expect(openaiFetch.calls[0]?.headers.get("authorization")).toBe(
      "Bearer caller-key",
    );
  });
});

describe("request encoding extras", () => {
  it("passes a redacted thinking block back to Anthropic and drops it for OpenAI", async () => {
    const anthropicFetch = fixtureFetch(readFixture("anthropic-text-tool.sse"));
    const openaiFetch = fixtureFetch(readFixture("openai-text-tool.sse"));
    const req: Request = {
      model: { provider: "anthropic", model: "claude-sonnet-4-5" },
      head: { system: "sys", tools: [] },
      messages: [
        {
          role: "assistant",
          content: [
            {
              type: "thinking",
              text: "",
              signature: "opaque-sig",
              redacted: true,
            },
          ],
        },
      ],
      cacheBreakpoints: [],
      sampling: { maxTokens: 8 },
    };
    const anthropic = new AnthropicAdapter({
      apiKey: "k",
      fetchImpl: anthropicFetch.fetchImpl,
    });
    const openai = new OpenAIAdapter({
      apiKey: "k",
      fetchImpl: openaiFetch.fetchImpl,
    });
    await drain(anthropic.stream(req, anthropic.prepare(req.model)));
    await drain(
      openai.stream(
        { ...req, model: { provider: "openai", model: "gpt-5" } },
        openai.prepare({ provider: "openai", model: "gpt-5" }),
      ),
    );
    const anthropicMessages = asRecord(anthropicFetch.calls[0]?.body)
      .messages as { content: unknown[] }[];
    expect(anthropicMessages[0]?.content[0]).toEqual({
      type: "redacted_thinking",
      data: "opaque-sig",
    });
    expect(JSON.stringify(openaiFetch.calls[0]?.body)).not.toContain(
      "opaque-sig",
    );
  });

  it("lets sampling.maxTokens override the catalog default", async () => {
    const { fetchImpl, calls } = fixtureFetch(
      readFixture("anthropic-text-tool.sse"),
    );
    const adapter = new AnthropicAdapter({ apiKey: "k", fetchImpl });
    const req = cacheRequest("anthropic", "claude-sonnet-5");
    req.sampling = { maxTokens: 100 };
    await drain(adapter.stream(req, adapter.prepare(req.model)));
    expect(asRecord(calls[0]?.body).max_tokens).toBe(100);
    expect(adapter.prepare(req.model).adapterDefaults.maxTokens).toBe(8192);
  });
});

describe("finish reasons and transport errors", () => {
  it("maps Anthropic stop reasons and keeps unknown ones on rawFinishReason", async () => {
    const cases: [string, Record<string, unknown>][] = [
      ["end_turn", { type: "finish", kind: "stop" }],
      ["max_tokens", { type: "finish", kind: "max_tokens" }],
      [
        "pause_turn",
        { type: "finish", kind: "stop", rawFinishReason: "pause_turn" },
      ],
    ];
    for (const [reason, finish] of cases) {
      const { fetchImpl } = fixtureFetch(anthropicStop(reason));
      const adapter = new AnthropicAdapter({ apiKey: "k", fetchImpl });
      const model = { provider: "anthropic", model: "claude-sonnet-4-5" };
      const chunks = await drain(
        adapter.stream(
          {
            model,
            head: { system: "sys", tools: [] },
            messages: [],
            cacheBreakpoints: [],
            sampling: {},
          },
          adapter.prepare(model),
        ),
      );
      expect(chunks.at(-1)).toEqual(finish);
      expectStreamInvariants(chunks);
    }
  });

  it("maps OpenAI finish reasons and keeps unknown ones on rawFinishReason", async () => {
    const cases: [string, Record<string, unknown>][] = [
      ["stop", { type: "finish", kind: "stop" }],
      ["length", { type: "finish", kind: "max_tokens" }],
      [
        "content_filter",
        { type: "finish", kind: "stop", rawFinishReason: "content_filter" },
      ],
    ];
    for (const [reason, finish] of cases) {
      const { fetchImpl } = fixtureFetch(openaiStop(reason));
      const adapter = new OpenAIAdapter({ apiKey: "k", fetchImpl });
      const model = { provider: "openai", model: "gpt-5" };
      const chunks = await drain(
        adapter.stream(
          {
            model,
            head: { system: "sys", tools: [] },
            messages: [],
            cacheBreakpoints: [],
            sampling: {},
          },
          adapter.prepare(model),
        ),
      );
      expect(chunks.at(-1)).toEqual(finish);
      expectStreamInvariants(chunks);
    }
  });

  it("turns a non-2xx response into finish error", async () => {
    const body = JSON.stringify({
      error: { message: "nope", type: "api_error" },
    });
    for (const make of [
      (fetchImpl: typeof fetch) =>
        new AnthropicAdapter({
          apiKey: "k",
          fetchImpl,
          sleep: async () => undefined,
        }),
      (fetchImpl: typeof fetch) =>
        new OpenAIAdapter({
          apiKey: "k",
          fetchImpl,
          sleep: async () => undefined,
        }),
    ]) {
      const { fetchImpl } = fixtureFetch(body, 500);
      const adapter = make(fetchImpl);
      const model = { provider: "anthropic", model: "claude-sonnet-4-5" };
      const chunks = await drain(
        adapter.stream(
          {
            model,
            head: { system: "sys", tools: [] },
            messages: [],
            cacheBreakpoints: [],
            sampling: { maxTokens: 8 },
          },
          adapter.prepare(model),
        ),
      );
      expect(chunks.at(-1)).toEqual({
        type: "finish",
        kind: "error",
        error: { message: "nope", status: 500, raw: JSON.parse(body) },
      });
      expectStreamInvariants(chunks);
    }
  });
});

describe("deepseek reasoning_content", () => {
  it("stores reasoning as a thinking block and omits it from the next request", async () => {
    const { fetchImpl, calls } = fixtureFetch(
      readFixture("deepseek-reasoner.sse"),
    );
    const adapter = new OpenAIAdapter({
      apiKey: "k",
      baseUrl: "https://api.deepseek.com",
      fetchImpl,
    });
    const model = { provider: "deepseek", model: "deepseek-reasoner" };
    const first: Request = {
      model,
      head: { system: "sys", tools: [] },
      messages: [{ role: "user", content: [{ type: "text", text: "hi" }] }],
      cacheBreakpoints: [],
      sampling: { maxTokens: 16 },
    };
    const stream = adapter.stream(first, adapter.prepare(model));
    const message = await collect(stream, { api: adapter.api });
    expect(message.content).toEqual([
      { type: "thinking", text: "REASON_STEP" },
      { type: "text", text: "Hi" },
      {
        type: "tool_use",
        id: "call_1",
        name: "bash",
        input: { command: "true" },
        rawInput: '{"command":"true"}',
      },
    ]);
    expect(message.content[0]).not.toHaveProperty("signature");

    const follow: Request = {
      ...first,
      messages: [
        ...first.messages,
        { role: "assistant", content: message.content },
        {
          role: "user",
          content: [
            {
              type: "tool_result",
              toolUseId: "call_1",
              content: "ok",
              isError: false,
            },
          ],
        },
      ],
    };
    await drain(adapter.stream(follow, adapter.prepare(model)));
    const encoded = JSON.stringify(calls[1]?.body);
    expect(encoded).not.toContain("REASON_STEP");
    expect(encoded).not.toContain("reasoning_content");
    expect(encoded).toContain("Hi");
  });
});

describe("openai response identity", () => {
  it("takes the id from the first frame that carries one, not from a leading frame without it", async () => {
    const body = [
      'data: {"object":"chat.completion.chunk","choices":[]}',
      "",
      'data: {"id":"chatcmpl-real","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}',
      "",
      openaiStop("stop"),
    ].join("\n");
    const { fetchImpl } = fixtureFetch(body);
    const adapter = new OpenAIAdapter({ apiKey: "test-key", fetchImpl });
    const model = { provider: "openai", model: "gpt-5" };
    const req: Request = {
      model,
      head: { system: "sys", tools: [] },
      messages: [{ role: "user", content: [{ type: "text", text: "hi" }] }],
      cacheBreakpoints: [],
      sampling: { maxTokens: 16 },
    };
    const chunks = await drain(adapter.stream(req, adapter.prepare(model)));
    expect(chunks.filter((chunk) => chunk.type === "response")).toEqual([
      { type: "response", id: "chatcmpl-real", model: "gpt-5" },
    ]);
    expect(chunks[0]?.type).toBe("response");
  });
});
