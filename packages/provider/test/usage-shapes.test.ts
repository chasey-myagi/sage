import { describe, expect, it } from "vitest";
import type { Usage } from "../src/contract/index.ts";
import { OpenAIAdapter } from "../src/openai/index.ts";
import { drain, fixtureFetch, readFixture } from "./support.ts";

function usageSse(usage: Record<string, unknown>): string {
  const frame = (payload: unknown): string =>
    `data: ${JSON.stringify(payload)}`;
  return [
    frame({
      id: "chatcmpl-s",
      object: "chat.completion.chunk",
      model: "m",
      choices: [{ index: 0, delta: { content: "ok" }, finish_reason: null }],
    }),
    "",
    frame({
      id: "chatcmpl-s",
      object: "chat.completion.chunk",
      model: "m",
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
    }),
    "",
    frame({
      id: "chatcmpl-s",
      object: "chat.completion.chunk",
      model: "m",
      choices: [],
      usage,
    }),
    "",
    "data: [DONE]",
    "",
  ].join("\n");
}

async function mapped(usage: Record<string, unknown>): Promise<Usage> {
  const { fetchImpl } = fixtureFetch(usageSse(usage));
  const adapter = new OpenAIAdapter({ apiKey: "k", fetchImpl });
  const model = { provider: "openai", model: "gpt-5.4" };
  const chunks = await drain(
    adapter.stream(
      {
        model,
        head: { system: "s", tools: [] },
        messages: [],
        cacheBreakpoints: [],
        sampling: {},
      },
      adapter.prepare(model),
    ),
  );
  const found = chunks.find((chunk) => chunk.type === "usage");
  if (found?.type !== "usage") throw new Error("no usage chunk");
  return found.usage;
}

describe("openai-compatible usage shapes", () => {
  it("maps DeepSeek hit and miss tokens", async () => {
    const raw = {
      prompt_tokens: 100,
      completion_tokens: 8,
      prompt_cache_hit_tokens: 70,
      prompt_cache_miss_tokens: 30,
    };
    expect(await mapped(raw)).toEqual({
      inputOther: 30,
      inputCacheRead: 70,
      inputCacheCreation: 0,
      output: 8,
      raw,
    });

    const inferred = {
      prompt_tokens: 100,
      completion_tokens: 1,
      prompt_cache_hit_tokens: 40,
    };
    expect(await mapped(inferred)).toEqual({
      inputOther: 60,
      inputCacheRead: 40,
      inputCacheCreation: 0,
      output: 1,
      raw: inferred,
    });
  });

  it("maps OpenAI prompt_tokens_details.cached_tokens", async () => {
    const raw = {
      prompt_tokens: 50,
      completion_tokens: 4,
      prompt_tokens_details: { cached_tokens: 12 },
    };
    expect(await mapped(raw)).toEqual({
      inputOther: 38,
      inputCacheRead: 12,
      inputCacheCreation: 0,
      output: 4,
      raw,
    });
  });

  it("maps DeepSeek reasoning_content into a thinking block and still reads hit tokens", async () => {
    const raw = {
      prompt_tokens: 100,
      completion_tokens: 8,
      prompt_cache_hit_tokens: 70,
      prompt_cache_miss_tokens: 30,
    };
    const { fetchImpl } = fixtureFetch(readFixture("deepseek-reasoner.sse"));
    const adapter = new OpenAIAdapter({ apiKey: "k", fetchImpl });
    const model = { provider: "deepseek", model: "deepseek-v4-pro" };
    const chunks = await drain(
      adapter.stream(
        {
          model,
          head: { system: "s", tools: [] },
          messages: [],
          cacheBreakpoints: [],
          sampling: {},
        },
        adapter.prepare(model),
      ),
    );
    const thinking = chunks.filter((chunk) => chunk.type === "thinking_delta");
    expect(thinking).toEqual([
      { type: "thinking_delta", index: 0, text: "REASON" },
      { type: "thinking_delta", index: 0, text: "_STEP" },
    ]);
    const text = chunks.find((chunk) => chunk.type === "text_delta");
    expect(text).toEqual({ type: "text_delta", index: 1, text: "Hi" });
    const usage = chunks.find((chunk) => chunk.type === "usage");
    expect(usage).toEqual({
      type: "usage",
      usage: {
        inputOther: 30,
        inputCacheRead: 70,
        inputCacheCreation: 0,
        output: 8,
        raw,
      },
    });
  });

  it("maps DashScope cached_tokens and keeps the raw payload", async () => {
    const raw = {
      prompt_tokens: 20,
      completion_tokens: 3,
      total_tokens: 23,
      prompt_tokens_details: { cached_tokens: 5 },
    };
    expect(await mapped(raw)).toEqual({
      inputOther: 15,
      inputCacheRead: 5,
      inputCacheCreation: 0,
      output: 3,
      raw,
    });
  });
});
