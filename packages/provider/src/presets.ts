/**
 * Named endpoint presets. Data plus resolvePreset; nothing is fetched.
 *
 * Deliberately not a Log entry: base URLs and env key names are deploy
 * config. The API key itself never lives here.
 */

import type { ProviderAdapter } from "./contract/adapter.js";

export type ProviderPreset = {
  name: string;
  baseUrl: string;
  envKey: string;
  api: ProviderAdapter["api"];
};

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

export function resolvePreset(name: string): ProviderPreset | undefined {
  return PRESETS.find((preset) => preset.name === name);
}
