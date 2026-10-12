import { describe, expect, it } from "vitest";
import { isPricing } from "../src/contract/index.ts";
import {
  AnthropicAdapter,
  defaultModel,
  listModels,
  OpenAIAdapter,
  parseLocalProviders,
  resolvePreset,
} from "../src/index.ts";

const SOURCE = "~/.sage/providers.json";

const gateway = {
  name: "example-gateway",
  baseUrl: "https://gateway.example.com/v1",
  envKey: "EXAMPLE_GATEWAY_KEY",
  api: "openai-chat",
} as const;

const creditPricing = {
  inputPerMTok: 3,
  outputPerMTok: 12,
  cacheReadPerMTok: 0.3,
  cacheWritePerMTok: 3.75,
  currency: "credit",
  source: "https://gateway.example.com/pricing",
};

function parse(value: unknown) {
  return parseLocalProviders(value, { defaultSource: SOURCE });
}

function parsed(value: unknown) {
  const result = parse(value);
  if (!result.ok) throw new Error(`${result.field}: ${result.message}`);
  return result.value;
}

describe("parseLocalProviders accepts a user providers file", () => {
  it("returns the declared providers and models, defaulting a missing pricing source", () => {
    const local = parsed({
      providers: [gateway],
      models: [
        {
          provider: "example-gateway",
          model: "example-model",
          contextWindow: 200_000,
          pricing: creditPricing,
        },
        {
          provider: "example-gateway",
          model: "example-small",
          contextWindow: 64_000,
          pricing: {
            inputPerMTok: 1,
            outputPerMTok: 2,
            cacheReadPerMTok: 0.1,
            cacheWritePerMTok: 0,
          },
        },
      ],
    });
    expect(local.providers).toEqual([gateway]);
    expect(local.models).toEqual([
      {
        provider: "example-gateway",
        model: "example-model",
        contextWindow: 200_000,
        pricing: creditPricing,
      },
      {
        provider: "example-gateway",
        model: "example-small",
        contextWindow: 64_000,
        pricing: {
          inputPerMTok: 1,
          outputPerMTok: 2,
          cacheReadPerMTok: 0.1,
          cacheWritePerMTok: 0,
          source: SOURCE,
        },
      },
    ]);
  });

  it("treats missing providers and models lists as empty", () => {
    expect(parsed({})).toEqual({ providers: [], models: [] });
  });

  it("accepts a model entry on a built-in provider", () => {
    const local = parsed({
      models: [
        {
          provider: "openai",
          model: "example-unlisted",
          contextWindow: 100_000,
          pricing: creditPricing,
        },
      ],
    });
    expect(listModels("openai", local.models).at(-1)?.model).toBe(
      "example-unlisted",
    );
  });
});

describe("parseLocalProviders names the field that is wrong", () => {
  const model = {
    provider: "example-gateway",
    model: "example-model",
    contextWindow: 200_000,
    pricing: creditPricing,
  };
  const cases: { name: string; value: unknown; field: string }[] = [
    { name: "a non-object root", value: [], field: "(root)" },
    { name: "an unknown top-level key", value: { model: [] }, field: "model" },
    {
      name: "providers not an array",
      value: { providers: {} },
      field: "providers",
    },
    {
      name: "a provider named like a built-in preset",
      value: { providers: [{ ...gateway, name: "openai" }] },
      field: "providers[0].name",
    },
    {
      name: "a provider named like a catalog-only provider",
      value: { providers: [{ ...gateway, name: "typesafe" }] },
      field: "providers[0].name",
    },
    {
      name: "a provider name with a slash",
      value: { providers: [{ ...gateway, name: "a/b" }] },
      field: "providers[0].name",
    },
    {
      name: "a provider declared twice",
      value: { providers: [gateway, gateway] },
      field: "providers[1].name",
    },
    {
      name: "a base URL that is not http(s)",
      value: {
        providers: [{ ...gateway, baseUrl: "ftp://gateway.example.com" }],
      },
      field: "providers[0].baseUrl",
    },
    {
      name: "an env key that is not a variable name",
      value: { providers: [{ ...gateway, envKey: "EXAMPLE-KEY" }] },
      field: "providers[0].envKey",
    },
    {
      name: "an unknown api",
      value: { providers: [{ ...gateway, api: "responses" }] },
      field: "providers[0].api",
    },
    {
      name: "an unknown provider key",
      value: { providers: [{ ...gateway, key: "x" }] },
      field: "providers[0].key",
    },
    {
      name: "a model on an undeclared provider",
      value: { models: [model] },
      field: "models[0].provider",
    },
    {
      name: "a model on the catalog-only scorer provider",
      value: { models: [{ ...model, provider: "typesafe" }] },
      field: "models[0].provider",
    },
    {
      name: "an empty model id",
      value: { providers: [gateway], models: [{ ...model, model: "" }] },
      field: "models[0].model",
    },
    {
      name: "a non-integer context window",
      value: {
        providers: [gateway],
        models: [{ ...model, contextWindow: 1.5 }],
      },
      field: "models[0].contextWindow",
    },
    {
      name: "a negative price",
      value: {
        providers: [gateway],
        models: [{ ...model, pricing: { ...creditPricing, inputPerMTok: -1 } }],
      },
      field: "models[0].pricing.inputPerMTok",
    },
    {
      name: "a missing price",
      value: {
        providers: [gateway],
        models: [
          {
            ...model,
            pricing: { inputPerMTok: 1, outputPerMTok: 1, cacheReadPerMTok: 1 },
          },
        ],
      },
      field: "models[0].pricing.cacheWritePerMTok",
    },
    {
      name: "a unit label with spaces",
      value: {
        providers: [gateway],
        models: [
          { ...model, pricing: { ...creditPricing, currency: "my credit" } },
        ],
      },
      field: "models[0].pricing.currency",
    },
    {
      name: "an unknown pricing key",
      value: {
        providers: [gateway],
        models: [{ ...model, pricing: { ...creditPricing, perCall: 1 } }],
      },
      field: "models[0].pricing.perCall",
    },
    {
      name: "the same provider and model twice",
      value: { providers: [gateway], models: [model, model] },
      field: "models[1].model",
    },
    {
      name: "a model the built-in catalog already prices",
      value: { models: [{ ...model, provider: "openai", model: "gpt-5.4" }] },
      field: "models[0].model",
    },
  ];
  for (const row of cases) {
    it(`rejects ${row.name}`, () => {
      const result = parse(row.value);
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.field).toBe(row.field);
      expect(result.message.length).toBeGreaterThan(0);
    });
  }
});

describe("adapters price user models by provider and model", () => {
  const local = parsed({
    providers: [gateway, { ...gateway, name: "example-other" }],
    models: [
      {
        provider: "example-gateway",
        model: "shared-model",
        contextWindow: 200_000,
        pricing: creditPricing,
      },
      {
        provider: "example-other",
        model: "shared-model",
        contextWindow: 100_000,
        pricing: { ...creditPricing, inputPerMTok: 9, currency: "USD" },
      },
    ],
  });

  it("returns each provider's own window and pricing for the same model id", () => {
    for (const adapter of [
      new OpenAIAdapter({ apiKey: "k", models: local.models }),
      new AnthropicAdapter({ apiKey: "k", models: local.models }),
    ]) {
      const first = adapter.prepare({
        provider: "example-gateway",
        model: "shared-model",
      });
      expect(first.contextWindow).toBe(200_000);
      expect(first.pricing).toEqual(creditPricing);
      expect(first.adapterDefaults).toEqual({ maxTokens: 8192 });
      const second = adapter.prepare({
        provider: "example-other",
        model: "shared-model",
      });
      expect(second.contextWindow).toBe(100_000);
      expect(second.pricing?.inputPerMTok).toBe(9);
      expect(second.pricing?.currency).toBe("USD");
    }
  });

  it("falls back for a user provider model that is not declared", () => {
    const adapter = new OpenAIAdapter({ apiKey: "k", models: local.models });
    expect(
      adapter.prepare({ provider: "example-gateway", model: "undeclared" }),
    ).toMatchObject({
      pricing: null,
      adapterDefaults: { catalogFallback: true },
    });
  });

  it("resolves user presets and their first model alongside the built-ins", () => {
    expect(resolvePreset("example-gateway", local.providers)).toEqual(gateway);
    expect(resolvePreset("example-gateway")).toBeUndefined();
    expect(resolvePreset("openai", local.providers)?.envKey).toBe(
      "OPENAI_API_KEY",
    );
    expect(defaultModel("example-gateway", local.models)).toEqual({
      provider: "example-gateway",
      model: "shared-model",
    });
    expect(defaultModel("example-empty", local.models)).toBeUndefined();
  });
});

describe("isPricing accepts a short unit label", () => {
  const base = {
    inputPerMTok: 1,
    outputPerMTok: 2,
    cacheReadPerMTok: 0.1,
    cacheWritePerMTok: 0,
    source: SOURCE,
  };
  it("takes ISO codes and a gateway's own unit, and rejects other values", () => {
    for (const currency of ["USD", "CNY", "EUR", "credit", "pts_v2"]) {
      expect(isPricing({ ...base, currency })).toBe(true);
    }
    for (const currency of ["", "my credit", 1, "x".repeat(17)]) {
      expect(isPricing({ ...base, currency })).toBe(false);
    }
  });
});
