/**
 * Named endpoint presets: base URL, API-key environment variable, and wire
 * protocol per provider name. Data only; nothing is fetched. Callers pass
 * the user's own presets (from `parseLocalProviders`) beside the built-ins.
 *
 * Intentionally not in the Log: base URLs and env key names are deploy
 * config. The API key itself never lives here.
 */

import type { ProviderAdapter } from "./contract/adapter.js";

/** One named endpoint. `envKey` names the variable that holds its API key. */
export type ProviderPreset = {
  name: string;
  baseUrl: string;
  envKey: string;
  api: ProviderAdapter["api"];
};

/** Every built-in preset. */
export const PRESETS: readonly ProviderPreset[] = [
  {
    name: "openai",
    baseUrl: "https://api.openai.com/v1",
    envKey: "OPENAI_API_KEY",
    api: "openai-chat",
  },
  {
    name: "deepseek",
    baseUrl: "https://api.deepseek.com",
    envKey: "DEEPSEEK_API_KEY",
    api: "openai-chat",
  },
  {
    name: "dashscope",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    envKey: "DASHSCOPE_API_KEY",
    api: "openai-chat",
  },
  {
    name: "moonshot",
    baseUrl: "https://api.moonshot.cn/v1",
    envKey: "MOONSHOT_API_KEY",
    api: "openai-chat",
  },
  {
    name: "anthropic",
    baseUrl: "https://api.anthropic.com",
    envKey: "ANTHROPIC_API_KEY",
    api: "anthropic-messages",
  },
];

/**
 * The preset called `name` among the built-ins and `local`, or undefined.
 * `parseLocalProviders` guarantees no local preset shadows a built-in.
 */
export function resolvePreset(
  name: string,
  local: readonly ProviderPreset[] = [],
): ProviderPreset | undefined {
  return [...PRESETS, ...local].find((preset) => preset.name === name);
}
