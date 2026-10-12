/**
 * Provider selection, API key lookup, and adapter assembly for the CLI.
 *
 * Contract: an explicit `--provider` wins; otherwise the first of
 * {@link AUTO_PROVIDERS} whose env key is set is used. Presets from the
 * user's providers file resolve like built-ins but are never auto-selected.
 * Every preset with a key set gets an adapter, and every adapter carries
 * the user's catalog entries; OpenAI-compatible presets share one adapter
 * class and differ by base URL. `typesafe` is refused as a chat provider:
 * its catalog row prices the Jev scorer. Failures are returned as values
 * with the exit code the command uses.
 *
 * Intentionally not in the Log: API keys, env var names, and base URLs.
 * The session logs provider and model on `request_header`.
 */

import {
  AnthropicAdapter,
  defaultModel,
  type LocalProviders,
  OpenAIAdapter,
  PRESETS,
  type ProviderAdapter,
  type ProviderPreset,
  resolvePreset,
} from "@sage/provider";
import { assertNever } from "./guards.js";

/** Auto-selection order when `--provider` is absent. */
const AUTO_PROVIDERS = [
  "deepseek",
  "dashscope",
  "moonshot",
  "openai",
  "anthropic",
] as const;

/** Why no provider could be used, with the exit code the command returns. */
type AuthFailure = { ok: false; message: string; exitCode: 1 | 2 };

/** Why {@link selectPreset} found no usable preset; `message` is for stderr. */
type SelectFailure = {
  ok: false;
  kind: "no-auto-key" | "not-chat" | "unknown-provider" | "missing-key";
  message: string;
};

/** A resolved chat provider, or why none could be used. */
type Auth =
  | {
      ok: true;
      provider: string;
      model: string;
      apiKey: string;
      baseUrl: string;
    }
  | AuthFailure;

/** Provider flags as written on argv; all optional. */
type AuthFlags = {
  provider?: string;
  model?: string;
  baseUrl?: string;
  apiKeyEnv?: string;
};

/**
 * Picks the provider, model, key, and base URL for `sage -p`. Exit 1 when
 * no auto provider has a key, the provider is unknown, it is `typesafe`,
 * or `--model` is absent and the provider has no catalog entry; exit 2
 * when the chosen preset's key (or `--api-key-env`) is unset. The model
 * defaults to the first catalog entry of that provider, built-in or local.
 */
export function resolveAuth(
  flags: AuthFlags,
  env: NodeJS.ProcessEnv,
  local: LocalProviders,
): Auth {
  const selected = selectPreset(flags, env, local);
  if (!selected.ok) {
    return {
      ok: false,
      message: selected.message,
      exitCode: selected.kind === "missing-key" ? 2 : 1,
    };
  }
  const { provider, apiKey, preset } = selected;
  const model = flags.model ?? defaultModel(provider, local.models)?.model;
  if (model === undefined) {
    return {
      ok: false,
      exitCode: 1,
      message: `No models declared for provider ${provider}; pass --model`,
    };
  }
  return {
    ok: true,
    provider,
    model,
    apiKey,
    baseUrl: flags.baseUrl ?? preset.baseUrl,
  };
}

/**
 * The bench pre-flight key check, mirroring {@link resolveAuth}: the
 * provider each child will select (`--provider`, else the first auto
 * provider with a key in `env`) and whether the key it reads (the
 * `--api-key-env` variable, else the preset's) is set. Children get the
 * same env, so the returned provider is the one they use. On failure,
 * `message` is the stderr line; `exitCode` is 1 for an unknown or non-chat
 * provider and 2 for a missing key.
 */
export function resolveBenchProvider(
  flags: Pick<AuthFlags, "provider" | "apiKeyEnv">,
  env: NodeJS.ProcessEnv,
  local: LocalProviders,
): { ok: true; provider: string } | AuthFailure {
  const selected = selectPreset(flags, env, local);
  if (selected.ok) return { ok: true, provider: selected.provider };
  return {
    ok: false,
    message: selected.message,
    exitCode: benchExitCode(selected.kind),
  };
}

function benchExitCode(kind: SelectFailure["kind"]): 1 | 2 {
  switch (kind) {
    case "unknown-provider":
    case "not-chat":
      return 1;
    case "no-auto-key":
    case "missing-key":
      return 2;
    default:
      return assertNever(kind);
  }
}

/**
 * One adapter per preset, built-in or local, whose env key is set, keyed
 * by preset name.
 */
export function adaptersFromEnv(
  env: NodeJS.ProcessEnv,
  local: LocalProviders,
): Record<string, ProviderAdapter> {
  const adapters: Record<string, ProviderAdapter> = {};
  for (const preset of [...PRESETS, ...local.providers]) {
    const apiKey = env[preset.envKey];
    if (!apiKey) continue;
    adapters[preset.name] = createAdapter(
      preset.name,
      apiKey,
      preset.baseUrl,
      local,
    );
  }
  return adapters;
}

/**
 * Anthropic-messages presets get the Messages adapter; the rest are OpenAI
 * chat. The adapter prices models from the built-in table plus
 * `local.models`.
 */
export function createAdapter(
  provider: string,
  apiKey: string,
  baseUrl: string,
  local: LocalProviders,
): ProviderAdapter {
  const options = { apiKey, baseUrl, models: local.models };
  const preset = resolvePreset(provider, local.providers);
  if (preset?.api === "anthropic-messages") {
    return new AnthropicAdapter(options);
  }
  return new OpenAIAdapter(options);
}

/** The shared half of {@link resolveAuth} and {@link resolveBenchProvider}. */
function selectPreset(
  flags: Pick<AuthFlags, "provider" | "apiKeyEnv">,
  env: NodeJS.ProcessEnv,
  local: LocalProviders,
):
  | { ok: true; provider: string; apiKey: string; preset: ProviderPreset }
  | SelectFailure {
  const provider = flags.provider ?? autoProvider(env);
  if (!provider) {
    return { ok: false, kind: "no-auto-key", message: missingAutoKeyMessage() };
  }
  if (provider === "typesafe") {
    return {
      ok: false,
      kind: "not-chat",
      message:
        "typesafe is not a chat endpoint; use --scorer jev with JEV_API_KEY or TYPESAFE_API_KEY",
    };
  }
  const preset = resolvePreset(provider, local.providers);
  if (!preset) {
    return {
      ok: false,
      kind: "unknown-provider",
      message: `Unknown provider: ${provider}`,
    };
  }
  const envKey = flags.apiKeyEnv ?? preset.envKey;
  const apiKey = env[envKey];
  if (!apiKey) {
    return { ok: false, kind: "missing-key", message: `Missing ${envKey}` };
  }
  return { ok: true, provider, apiKey, preset };
}

function autoProvider(env: NodeJS.ProcessEnv): string | undefined {
  return AUTO_PROVIDERS.find((name) => {
    const preset = resolvePreset(name);
    return preset !== undefined && Boolean(env[preset.envKey]);
  });
}

function missingAutoKeyMessage(): string {
  const keys = AUTO_PROVIDERS.flatMap((name) => {
    const preset = resolvePreset(name);
    return preset ? [preset.envKey] : [];
  });
  if (keys.length === 0) return "Missing API key";
  if (keys.length === 1) return `Missing ${keys[0]}`;
  return `Missing ${keys.slice(0, -1).join(", ")}, or ${keys.at(-1)}`;
}
