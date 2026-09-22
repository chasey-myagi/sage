import { describe, expect, it } from "vitest";
import { AnthropicAdapter } from "../src/anthropic/index.ts";
import type { ModelRef } from "../src/contract/index.ts";
import { OpenAIAdapter } from "../src/openai/index.ts";
import { drain, fixtureFetch, readFixture } from "./support.ts";

const known: { ref: ModelRef; contextWindow: number }[] = [
  {
    ref: { provider: "anthropic", model: "claude-sonnet-4-5" },
    contextWindow: 200_000,
  },
  {
    ref: { provider: "anthropic", model: "claude-opus-4-1" },
    contextWindow: 200_000,
  },
  {
    ref: { provider: "anthropic", model: "claude-haiku-4-5" },
    contextWindow: 200_000,
  },
  { ref: { provider: "openai", model: "gpt-5" }, contextWindow: 400_000 },
  { ref: { provider: "openai", model: "gpt-5-mini" }, contextWindow: 400_000 },
];

describe("scenario 6 prepare", () => {
  it("returns the static window, null pricing, and maxTokens default", () => {
    const anthropic = new AnthropicAdapter({ apiKey: "k" });
    const openai = new OpenAIAdapter({ apiKey: "k" });
    const prepared = anthropic.prepare({
      provider: "anthropic",
      model: "claude-sonnet-4-5",
    });
    expect(prepared).toEqual({
      model: { provider: "anthropic", model: "claude-sonnet-4-5" },
      contextWindow: 200_000,
      pricing: null,
      adapterDefaults: { maxTokens: 8192 },
    });
    for (const entry of known) {
      for (const adapter of [anthropic, openai]) {
        expect(adapter.prepare(entry.ref)).toEqual({
          model: entry.ref,
          contextWindow: entry.contextWindow,
          pricing: null,
          adapterDefaults: { maxTokens: 8192 },
        });
      }
    }
  });

  it("falls back when the model is missing and says so in adapterDefaults", async () => {
    const { fetchImpl, calls } = fixtureFetch(
      readFixture("anthropic-text-tool.sse"),
    );
    const adapter = new AnthropicAdapter({ apiKey: "k", fetchImpl });
    const model = { provider: "anthropic", model: "missing-model" };
    expect(adapter.prepare(model)).toEqual({
      model,
      contextWindow: 128_000,
      pricing: null,
      adapterDefaults: { maxTokens: 4096, catalogFallback: true },
    });
    await drain(
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
    const body = calls[0]?.body as { max_tokens: number };
    expect(body.max_tokens).toBe(4096);
  });
});
