/**
 * Validates a user's providers file: extra endpoint presets and extra
 * catalog entries with their prices. Pure; the caller reads the file.
 *
 * Guarantees: a returned value has no preset that shadows a built-in name
 * (preset or catalog provider), no duplicate preset name, no model on a
 * provider that is neither a built-in preset nor declared in the file, and
 * no (provider, model) pair that appears twice or is already in the
 * built-in table. Unknown keys are rejected, not ignored. Failures name
 * the first offending field as a path such as `models[2].pricing.currency`.
 *
 * Intentionally not in the Log: the file and its presets. Prices reach the
 * Log only through `PreparedCall.pricing` on each request_header, so
 * replay uses the prices of the run, not the current file.
 */

import { type CatalogEntry, MODELS } from "./catalog.js";
import { isPricing, PRICE_UNIT, type Pricing } from "./contract/adapter.js";
import { isFiniteNumber, isRecord } from "./contract/record.js";
import { PRESETS, type ProviderPreset } from "./presets.js";

/** User presets and catalog entries, ready to pass beside the built-ins. */
export type LocalProviders = {
  providers: readonly ProviderPreset[];
  models: readonly CatalogEntry[];
};

/** No user file: built-ins only. */
export const NO_LOCAL_PROVIDERS: LocalProviders = { providers: [], models: [] };

/** The parsed file, or the first field that is wrong and why. */
export type LocalProvidersResult =
  | { ok: true; value: LocalProviders }
  | { ok: false; field: string; message: string };

type Failure = Extract<LocalProvidersResult, { ok: false }>;

/**
 * Provider names travel in `--provider` and `--ladder provider/model,...`,
 * so they may not contain `/`, `,`, or spaces.
 */
const PROVIDER_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;
/** A portable environment variable name. */
const ENV_KEY = /^[A-Za-z_][A-Za-z0-9_]*$/;
const APIS: readonly ProviderPreset["api"][] = [
  "openai-chat",
  "anthropic-messages",
];
const ROOT_KEYS = ["providers", "models"];
const PROVIDER_KEYS = ["name", "baseUrl", "envKey", "api"];
const MODEL_KEYS = ["provider", "model", "contextWindow", "pricing"];
const PRICE_KEYS = [
  "inputPerMTok",
  "outputPerMTok",
  "cacheReadPerMTok",
  "cacheWritePerMTok",
] as const;
const PRICING_KEYS = [...PRICE_KEYS, "currency", "source"];

/**
 * Validates the parsed JSON of a providers file. `defaultSource` becomes
 * `Pricing.source` for entries that omit it. Never throws.
 */
export function parseLocalProviders(
  value: unknown,
  opts: { defaultSource: string },
): LocalProvidersResult {
  if (!isRecord(value)) return fail("(root)", "must be a JSON object");
  const extra = unknownKey(value, ROOT_KEYS, "");
  if (extra) return extra;
  const providers = parseProviders(value.providers);
  if (!providers.ok) return providers;
  const models = parseModels(value.models, providers.value, opts);
  if (!models.ok) return models;
  return {
    ok: true,
    value: { providers: providers.value, models: models.value },
  };
}

function parseProviders(
  value: unknown,
): { ok: true; value: ProviderPreset[] } | Failure {
  const items = listAt(value, "providers");
  if (!items.ok) return items;
  const reserved = new Set([
    ...PRESETS.map((preset) => preset.name),
    ...MODELS.map((entry) => entry.provider),
  ]);
  const out: ProviderPreset[] = [];
  for (const [index, item] of items.value.entries()) {
    const at = `providers[${index}]`;
    const preset = parseProvider(item, at);
    if (!preset.ok) return preset;
    if (reserved.has(preset.value.name)) {
      return fail(`${at}.name`, `"${preset.value.name}" is a built-in name`);
    }
    if (out.some((seen) => seen.name === preset.value.name)) {
      return fail(`${at}.name`, `"${preset.value.name}" is declared twice`);
    }
    out.push(preset.value);
  }
  return { ok: true, value: out };
}

function parseProvider(
  value: unknown,
  at: string,
): { ok: true; value: ProviderPreset } | Failure {
  if (!isRecord(value)) return fail(at, "must be an object");
  const extra = unknownKey(value, PROVIDER_KEYS, at);
  if (extra) return extra;
  const { name, baseUrl, envKey, api } = value;
  if (typeof name !== "string" || !PROVIDER_NAME.test(name)) {
    return fail(`${at}.name`, "must be letters, digits, '.', '_' or '-'");
  }
  if (typeof baseUrl !== "string" || !isHttpUrl(baseUrl)) {
    return fail(`${at}.baseUrl`, "must be an http(s) URL");
  }
  if (typeof envKey !== "string" || !ENV_KEY.test(envKey)) {
    return fail(`${at}.envKey`, "must be an environment variable name");
  }
  const chosen = APIS.find((candidate) => candidate === api);
  if (!chosen) return fail(`${at}.api`, `must be one of ${APIS.join(", ")}`);
  return { ok: true, value: { name, baseUrl, envKey, api: chosen } };
}

function parseModels(
  value: unknown,
  providers: readonly ProviderPreset[],
  opts: { defaultSource: string },
): { ok: true; value: CatalogEntry[] } | Failure {
  const items = listAt(value, "models");
  if (!items.ok) return items;
  const known = new Set(
    [...PRESETS, ...providers].map((preset) => preset.name),
  );
  const out: CatalogEntry[] = [];
  for (const [index, item] of items.value.entries()) {
    const at = `models[${index}]`;
    const entry = parseModel(item, at, opts);
    if (!entry.ok) return entry;
    const { provider, model } = entry.value;
    if (!known.has(provider)) {
      return fail(`${at}.provider`, `"${provider}" is not a declared provider`);
    }
    const same = (seen: CatalogEntry) =>
      seen.provider === provider && seen.model === model;
    if (MODELS.some(same)) {
      return fail(`${at}.model`, `${provider}/${model} is built in`);
    }
    if (out.some(same)) {
      return fail(`${at}.model`, `${provider}/${model} is declared twice`);
    }
    out.push(entry.value);
  }
  return { ok: true, value: out };
}

function parseModel(
  value: unknown,
  at: string,
  opts: { defaultSource: string },
): { ok: true; value: CatalogEntry } | Failure {
  if (!isRecord(value)) return fail(at, "must be an object");
  const extra = unknownKey(value, MODEL_KEYS, at);
  if (extra) return extra;
  const { provider, model, contextWindow } = value;
  if (typeof provider !== "string") {
    return fail(`${at}.provider`, "must be a string");
  }
  if (typeof model !== "string" || model.length === 0) {
    return fail(`${at}.model`, "must be a non-empty string");
  }
  if (
    typeof contextWindow !== "number" ||
    !Number.isSafeInteger(contextWindow) ||
    contextWindow <= 0
  ) {
    return fail(`${at}.contextWindow`, "must be a positive integer");
  }
  const pricing = parsePricing(value.pricing, `${at}.pricing`, opts);
  if (!pricing.ok) return pricing;
  return {
    ok: true,
    value: { provider, model, contextWindow, pricing: pricing.value },
  };
}

function parsePricing(
  value: unknown,
  at: string,
  opts: { defaultSource: string },
): { ok: true; value: Pricing } | Failure {
  if (!isRecord(value)) return fail(at, "must be an object");
  const extra = unknownKey(value, PRICING_KEYS, at);
  if (extra) return extra;
  for (const key of PRICE_KEYS) {
    const price = value[key];
    if (!isFiniteNumber(price) || price < 0) {
      return fail(`${at}.${key}`, "must be a non-negative number");
    }
  }
  const { currency, source } = value;
  if (
    currency !== undefined &&
    (typeof currency !== "string" || !PRICE_UNIT.test(currency))
  ) {
    return fail(
      `${at}.currency`,
      "must be a short label such as USD or credit",
    );
  }
  if (source !== undefined && typeof source !== "string") {
    return fail(`${at}.source`, "must be a string");
  }
  const pricing = {
    inputPerMTok: value.inputPerMTok,
    outputPerMTok: value.outputPerMTok,
    cacheReadPerMTok: value.cacheReadPerMTok,
    cacheWritePerMTok: value.cacheWritePerMTok,
    source: source ?? opts.defaultSource,
    ...(currency !== undefined ? { currency } : {}),
  };
  // Every field was checked above; the guard only restores the narrowing.
  if (!isPricing(pricing)) return fail(at, "is not a valid pricing");
  return { ok: true, value: pricing };
}

/** `value` as a list; absent means empty. */
function listAt(
  value: unknown,
  at: string,
): { ok: true; value: readonly unknown[] } | Failure {
  if (value === undefined) return { ok: true, value: [] };
  if (!Array.isArray(value)) return fail(at, "must be an array");
  return { ok: true, value };
}

function unknownKey(
  value: Record<string, unknown>,
  allowed: readonly string[],
  at: string,
): Failure | undefined {
  const key = Object.keys(value).find((name) => !allowed.includes(name));
  if (key === undefined) return undefined;
  return fail(
    at ? `${at}.${key}` : key,
    `unknown key; expected ${allowed.join(", ")}`,
  );
}

function isHttpUrl(text: string): boolean {
  if (!URL.canParse(text)) return false;
  const { protocol } = new URL(text);
  return protocol === "https:" || protocol === "http:";
}

function fail(field: string, message: string): Failure {
  return { ok: false, field, message };
}
