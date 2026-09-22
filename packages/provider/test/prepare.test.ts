import { describe, expect, it } from "vitest";
import { AnthropicAdapter } from "../src/anthropic/index.ts";
import type { ModelRef, Pricing } from "../src/contract/index.ts";
import { OpenAIAdapter } from "../src/openai/index.ts";
import { drain, fixtureFetch, readFixture } from "./support.ts";

const ANTHROPIC_SOURCE =
  "https://platform.claude.com/docs/en/about-claude/pricing";
const OPENAI_SOURCE = "https://developers.openai.com/api/docs/pricing";

const known: { ref: ModelRef; contextWindow: number; pricing: Pricing }[] = [
  {
    ref: { provider: "anthropic", model: "claude-sonnet-5" },
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 2,
      outputPerMTok: 10,
      cacheWritePerMTok: 2.5,
      cacheReadPerMTok: 0.2,
      source: ANTHROPIC_SOURCE,
    },
  },
  {
    ref: { provider: "anthropic", model: "claude-opus-5" },
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 5,
      outputPerMTok: 25,
      cacheWritePerMTok: 6.25,
      cacheReadPerMTok: 0.5,
      source: ANTHROPIC_SOURCE,
    },
  },
  {
    ref: { provider: "anthropic", model: "claude-haiku-4-5-20251001" },
    contextWindow: 200_000,
    pricing: {
      inputPerMTok: 1,
      outputPerMTok: 5,
      cacheWritePerMTok: 1.25,
      cacheReadPerMTok: 0.1,
      source: ANTHROPIC_SOURCE,
    },
  },
  {
    ref: { provider: "anthropic", model: "claude-fable-5-1" },
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 10,
      outputPerMTok: 50,
      cacheWritePerMTok: 12.5,
      cacheReadPerMTok: 0.25,
      source: ANTHROPIC_SOURCE,
    },
  },
  {
    ref: { provider: "openai", model: "gpt-5.4" },
    contextWindow: 272_000,
    pricing: {
      inputPerMTok: 2.5,
      outputPerMTok: 15,
      cacheWritePerMTok: 0,
      cacheReadPerMTok: 0.25,
      source: OPENAI_SOURCE,
    },
  },
  {
    ref: { provider: "openai", model: "gpt-5.5" },
    contextWindow: 272_000,
    pricing: {
      inputPerMTok: 5,
      outputPerMTok: 30,
      cacheWritePerMTok: 0,
      cacheReadPerMTok: 0.5,
      source: OPENAI_SOURCE,
    },
  },
  {
    ref: { provider: "openai", model: "gpt-5.4-mini" },
    contextWindow: 272_000,
    pricing: {
      inputPerMTok: 0.75,
      outputPerMTok: 4.5,
      cacheWritePerMTok: 0,
      cacheReadPerMTok: 0.075,
      source: OPENAI_SOURCE,
    },
  },
];

describe("catalog export", () => {
  it("defaults to the first catalog entry for that provider", async () => {
    const provider = await import("../src/index.ts");
    expect(
      provider.listModels("anthropic").map((entry) => entry.model),
    ).toEqual([
      "claude-sonnet-5",
      "claude-opus-5",
      "claude-haiku-4-5-20251001",
      "claude-fable-5-1",
    ]);
    expect(provider.listModels("openai").map((entry) => entry.model)).toEqual([
      "gpt-5.4",
      "gpt-5.5",
      "gpt-5.4-mini",
    ]);
    for (const entry of [
      ...provider.listModels("anthropic"),
      ...provider.listModels("openai"),
    ]) {
      expect(entry.pricing).not.toBeNull();
      expect(entry.pricing.source).toMatch(/^https:\/\//);
    }
    expect(provider.defaultModel("anthropic")).toEqual({
      provider: "anthropic",
      model: "claude-sonnet-5",
    });
    expect(provider.defaultModel("openai")).toEqual({
      provider: "openai",
      model: "gpt-5.4",
    });
    expect(provider.listModels("missing")).toEqual([]);
  });
});

describe("scenario 6 prepare", () => {
  it("returns the static window, sourced pricing, and maxTokens default", () => {
    const anthropic = new AnthropicAdapter({ apiKey: "k" });
    const openai = new OpenAIAdapter({ apiKey: "k" });
    const prepared = anthropic.prepare({
      provider: "anthropic",
      model: "claude-sonnet-5",
    });
    expect(prepared).toEqual({
      model: { provider: "anthropic", model: "claude-sonnet-5" },
      contextWindow: 1_000_000,
      pricing: known[0]?.pricing,
      adapterDefaults: { maxTokens: 8192 },
    });
    for (const entry of known) {
      for (const adapter of [anthropic, openai]) {
        expect(adapter.prepare(entry.ref)).toEqual({
          model: entry.ref,
          contextWindow: entry.contextWindow,
          pricing: entry.pricing,
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
