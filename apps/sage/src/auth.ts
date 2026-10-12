/**
 * Provider selection, API key lookup, and adapter assembly for the CLI.
 *
 * Contract: an explicit `--provider` wins; otherwise the first of
 * {@link AUTO_PROVIDERS} whose env key is set is used. Every preset with a
 * key set gets an adapter; OpenAI-compatible presets share one adapter class
 * and differ by base URL. `typesafe` is refused as a chat provider: its
 * catalog row prices the Jev scorer. Failures are returned as values with
 * the exit code the command uses.
 *
 * Intentionally not in the Log: API keys, env var names, and base URLs.
 * The session logs provider and model on `request_header`.
 */

import {
  AnthropicAdapter,
  defaultModel,
  OpenAIAdapter,
  PRESETS,
  type ProviderAdapter,
  resolvePreset,
} from "@sage/provider";

/** Auto-selection order when `--provider` is absent. */
const AUTO_PROVIDERS = [
  "deepseek",
  "dashscope",
  "moonshot",
  "openai",
  "anthropic",
] as const;

/** A resolved chat provider, or why none could be used. */
type Auth =
  | {
      ok: true;
      provider: string;
      model: string;
      apiKey: string;
      baseUrl: string;
    }
  | { ok: false; message: string; exitCode: 1 | 2 };

/** Provider flags as written on argv; all optional. */
type AuthFlags = {
  provider?: string;
  model?: string;
  baseUrl?: string;
  apiKeyEnv?: string;
};

/**
 * Picks the provider, model, key, and base URL for `sage -p`. Exit 1 when
 * no auto provider has a key, the provider is unknown, or it is `typesafe`;
 * exit 2 when the chosen preset's key (or `--api-key-env`) is unset. The
 * model defaults to the first catalog entry of that provider.
 */
export function resolveAuth(flags: AuthFlags, env: NodeJS.ProcessEnv): Auth {
  const selected = flags.provider ?? autoProvider(env);
  if (!selected) {
    return { ok: false, exitCode: 1, message: missingAutoKeyMessage() };
  }
  if (selected === "typesafe") {
    return {
      ok: false,
      exitCode: 1,
      message:
        "typesafe is not a chat endpoint; use --scorer jev with JEV_API_KEY or TYPESAFE_API_KEY",
    };
  }
  const preset = resolvePreset(selected);
  if (!preset) {
    return { ok: false, exitCode: 1, message: `Unknown provider: ${selected}` };
  }
  const envKey = flags.apiKeyEnv ?? preset.envKey;
  const apiKey = env[envKey];
  if (!apiKey) {
    return { ok: false, exitCode: 2, message: `Missing ${envKey}` };
  }
  return {
    ok: true,
    provider: selected,
    model: flags.model ?? defaultModel(selected).model,
    apiKey,
    baseUrl: flags.baseUrl ?? preset.baseUrl,
  };
}

/**
 * The bench pre-flight key check: the named preset's key, or any auto
 * provider's key. Returns the stderr line, or undefined when a key is set.
 */
export function missingBenchKey(
  provider: string | undefined,
  env: NodeJS.ProcessEnv,
): string | undefined {
  if (provider) {
    const preset = resolvePreset(provider);
    if (!preset) return `Unknown provider: ${provider}`;
    return env[preset.envKey] ? undefined : `Missing ${preset.envKey}`;
  }
  return autoProvider(env) ? undefined : missingAutoKeyMessage();
}

/** One adapter per preset whose env key is set, keyed by preset name. */
export function adaptersFromEnv(
  env: NodeJS.ProcessEnv,
): Record<string, ProviderAdapter> {
  const adapters: Record<string, ProviderAdapter> = {};
  for (const preset of PRESETS) {
    const apiKey = env[preset.envKey];
    if (!apiKey) continue;
    adapters[preset.name] = createAdapter(preset.name, apiKey, preset.baseUrl);
  }
  return adapters;
}

/** Anthropic presets get the Messages adapter; the rest are OpenAI chat. */
export function createAdapter(
  provider: string,
  apiKey: string,
  baseUrl: string,
): ProviderAdapter {
  const preset = resolvePreset(provider);
  if (preset?.api === "anthropic-messages") {
    return new AnthropicAdapter({ apiKey, baseUrl });
  }
  return new OpenAIAdapter({ apiKey, baseUrl });
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
